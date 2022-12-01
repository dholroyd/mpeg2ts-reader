//! Main types implementing the demultiplexer state-machine.
//!
//! Construct an instance of [`Demultiplex`](struct.Demultiplex.html) and feed it one a succession
//! of byte-slices containing the Transport Stream data.
//!
//! Users of this crate are expected to provide their own implementations of,
//!
//!  - [`PacketFilter`](trait.PacketFilter.html) - for defining per-stream-type handling, possibly
//!    by using the [`packet_filter_switch!()`](../macro.packet_filter_switch.html) macro to create
//!    an enum implementing this trait.
//!  - [`DemuxContext`](trait.DemuxContext.html) - to create specific `PacketFilter` instances
//!    for each type of sub-stream found within the Transport Stream data. possibly by using the
//!    [`demux_context!()`](../macro.demux_context.html) macro.

use crate::packet;
use crate::packet::TransportScramblingControl;
use crate::psi;
use crate::psi::pat;
use crate::psi::pmt::PmtSection;
use crate::psi::pmt::StreamInfo;
use crate::StreamType;
use log::warn;
use std::marker;

/// Trait to which `Demultiplex` delegates handling of subsets of Transport Stream packets.
///
/// A packet filter can collaborate with the owning `Demultiplex` instance via the `DemuxContext`
/// to which they will both be lent mutable access.  For example, by making entries in the
/// `FilterChangeset` owned by the `DemuxContext`, a `PacketFilter` implementation may alter
/// the handling of subsequent packets in the Transport Stream.
pub trait PacketFilter {
    /// The type of context-object used by the `Demultiplex` instance with which implementing
    /// `PacketFilters` will be collaborating.
    type Ctx: DemuxContext;

    /// Implements filter-specific packet processing logic.
    fn consume(&mut self, ctx: &mut Self::Ctx, pk: &packet::Packet<'_>);
}

/// No-op implementation of `PacketFilter`.
///
/// Sometimes a Transport Stream will contain packets that you know you want to ignore.  This type
/// can be used when you have to register a filter for such packets with the demultiplexer.
pub struct NullPacketFilter<Ctx: DemuxContext> {
    phantom: marker::PhantomData<Ctx>,
}
impl<Ctx: DemuxContext> Default for NullPacketFilter<Ctx> {
    fn default() -> NullPacketFilter<Ctx> {
        NullPacketFilter {
            phantom: marker::PhantomData,
        }
    }
}
impl<Ctx: DemuxContext> PacketFilter for NullPacketFilter<Ctx> {
    type Ctx = Ctx;
    fn consume(&mut self, _ctx: &mut Self::Ctx, _pk: &packet::Packet<'_>) {
        // ignore
    }
}

/// Creates the boilerplate needed for a filter-implementation-specific `DemuxContext`.
///
/// This macro takes two arguments; the name for the new type, and the name of an existing
/// implementation of `PacketFilter`.  It then..
///
/// 1. creates a struct with the given name, wrapping an instance of `FilterChangeset`
/// 2. provides an implementation of `default()` for that struct
/// 3. provides an implementation of `DemuxContext`
///
/// **NB** The implementation of `DemuxContext` will assume that your own implementation of the
/// type will provide a `do_construct()` method, to which its implementation of
/// [`DemuxContext::construct()`](DemuxContext::construct) will delegate.
///
/// # Example
///
/// ```
/// # #[macro_use]
/// # extern crate mpeg2ts_reader;
/// use mpeg2ts_reader::demultiplex;
/// # fn main() {
/// // Create an enum that implements PacketFilter as required by your application.
/// packet_filter_switch!{
///     MyFilterSwitch<MyDemuxContext> {
///         Pat: demultiplex::PatPacketFilter<MyDemuxContext>,
///         Pmt: demultiplex::PmtPacketFilter<MyDemuxContext>,
///         Nul: demultiplex::NullPacketFilter<MyDemuxContext>,
///     }
/// };
///
/// // Create an implementation of the DemuxContext trait for the PacketFilter implementation
/// // created above.
/// demux_context!(MyDemuxContext, MyFilterSwitch);
/// impl MyDemuxContext {
///     fn do_construct(&mut self, req: demultiplex::FilterRequest<'_, '_>) -> MyFilterSwitch {
///         // ...inspect 'req', construct appropriate 'MyFilterSwitch' variant
/// #        unimplemented!()
///     }
/// }
///
/// let mut ctx = MyDemuxContext::new();
/// // .. use the ctx value while demultiplexing some data ..
/// # }
/// ```
#[macro_export]
macro_rules! demux_context {
    ($name:ident, $filter:ty) => {
        pub struct $name {
            changeset: $crate::demultiplex::FilterChangeset<$filter>,
        }
        impl $name {
            pub fn new() -> Self {
                $name {
                    changeset: $crate::demultiplex::FilterChangeset::default(),
                }
            }
        }
        impl $crate::demultiplex::DemuxContext for $name {
            type F = $filter;

            fn filter_changeset(&mut self) -> &mut $crate::demultiplex::FilterChangeset<Self::F> {
                &mut self.changeset
            }
            fn construct(&mut self, req: $crate::demultiplex::FilterRequest<'_, '_>) -> Self::F {
                self.do_construct(req)
            }
        }
    };
}

/// Creates an enum which implements [`PacketFilter`](demultiplex/trait.PacketFilter.html) by
/// delegating to other `PacketFilter` implementations, depending on the enum-variant.  It's
/// intended that the types created by this macro be used as the type-parameter for an instance of
/// the `Demultiplex` type, allowing `Demultiplex` to support many kinds of `PacketFilter`, without
/// the cost of having to box them.
///
/// See [demux_context!()](macro.demux_context.html) for an example.
#[macro_export]
macro_rules! packet_filter_switch {
    (
        $name:ident<$ctx:ty> {
            $( $case_name:ident : $t:ty ),*,
        }
    ) => {
        pub enum $name {
            $( $case_name($t), )*
        }
        impl $crate::demultiplex::PacketFilter for $name {
            type Ctx = $ctx;
            #[inline(always)]
            fn consume(&mut self, ctx: &mut $ctx, pk: &$crate::packet::Packet<'_>) {
                match self {
                    $( &mut $name::$case_name(ref mut f) => f.consume(ctx, pk), )*

                }
            }
        }
    }
}

/// Growable list of filters (implementations of [`PacketFilter`](trait.PacketFilter.html)),
/// indexed by [`Pid`](../packet/struct.Pid.html).  Lookups produce an `Option`, and the result
/// is `None` rather than `panic!()` when not found.
struct Filters<F: PacketFilter> {
    filters_by_pid: Vec<Option<F>>,
}
impl<F: PacketFilter> Default for Filters<F> {
    fn default() -> Filters<F> {
        Filters {
            filters_by_pid: vec![],
        }
    }
}
impl<F: PacketFilter> Filters<F> {
    pub fn contains(&self, pid: packet::Pid) -> bool {
        usize::from(pid) < self.filters_by_pid.len()
            && self.filters_by_pid[usize::from(pid)].is_some()
    }

    pub fn get(&mut self, pid: packet::Pid) -> Option<&mut F> {
        if usize::from(pid) >= self.filters_by_pid.len() {
            None
        } else {
            self.filters_by_pid[usize::from(pid)].as_mut()
        }
    }

    pub fn insert(&mut self, pid: packet::Pid, filter: F) {
        let diff = usize::from(pid) as isize - self.filters_by_pid.len() as isize;
        if diff >= 0 {
            for _ in 0..=diff {
                self.filters_by_pid.push(None);
            }
        }
        self.filters_by_pid[usize::from(pid)] = Some(filter);
    }

    pub fn remove(&mut self, pid: packet::Pid) {
        if usize::from(pid) < self.filters_by_pid.len() {
            self.filters_by_pid[usize::from(pid)] = None;
        }
    }
}

// A filter can't change the map of filters-by-pid that it is itself owned by while the filter is
// running, so this changeset protocol allows a filter to specify any filter updates required so
// the demultiplexer can apply them when the filter is complete

/// Represents the intention to either insert a new `PacketFilter` into the `Demultiplex` instance
/// or remove an old `PacketFilter` from the `Demultiplex` instance.
pub enum FilterChange<F: PacketFilter> {
    /// Insert the given filter for the given `Pid`.
    Insert(packet::Pid, F),
    /// Remove any filter for the given `Pid`.
    Remove(packet::Pid),
}
impl<F: PacketFilter> FilterChange<F> {
    fn apply(self, filters: &mut Filters<F>) {
        match self {
            FilterChange::Insert(pid, filter) => filters.insert(pid, filter),
            FilterChange::Remove(pid) => filters.remove(pid),
        };
    }
}
impl<F: PacketFilter> std::fmt::Debug for FilterChange<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match *self {
            FilterChange::Insert(pid, _) => write!(f, "FilterChange::Insert {{ {:?}, ... }}", pid),
            FilterChange::Remove(pid) => write!(f, "FilterChange::Remove {{ {:?}, ... }}", pid),
        }
    }
}

/// Owns a queue of [`FilterChange`](enum.FilterChange.html) objects representing pending updates
/// to the Pid handling of the `Demultiplexer`.
///
/// These changes need to be queued since practically a `PacketFilter` implementation cannot be
/// allowed to remove itself from the owning `Demultiplex` instance while it is in the act of
/// filtering a packet.
///
/// The public interface allows items to be added to the queue, and the internal implementation of
/// `Demultiplex` will later remove them.
#[derive(Debug)]
pub struct FilterChangeset<F: PacketFilter> {
    updates: Vec<FilterChange<F>>,
}
impl<F: PacketFilter> Default for FilterChangeset<F> {
    fn default() -> FilterChangeset<F> {
        FilterChangeset {
            updates: Vec::new(),
        }
    }
}
impl<F: PacketFilter> FilterChangeset<F> {
    /// Queue the insertion of the given `PacketFilter` for the given Pid, after the `Demultiplex`
    /// instance has finished handling the current packet.
    pub fn insert(&mut self, pid: packet::Pid, filter: F) {
        self.updates.push(FilterChange::Insert(pid, filter))
    }
    /// Queue the removal of the existing `PacketFilter` for the given Pid, after the `Demultiplex`
    /// instance has finished handling the current packet.
    pub fn remove(&mut self, pid: packet::Pid) {
        self.updates.push(FilterChange::Remove(pid))
    }

    fn apply(&mut self, filters: &mut Filters<F>) {
        for update in self.updates.drain(..) {
            update.apply(filters);
        }
    }
    /// Are there any changes queued in this changeset?
    pub fn is_empty(&self) -> bool {
        self.updates.is_empty()
    }
}

impl<F: PacketFilter> std::iter::IntoIterator for FilterChangeset<F> {
    type Item = FilterChange<F>;
    type IntoIter = std::vec::IntoIter<FilterChange<F>>;

    fn into_iter(self) -> std::vec::IntoIter<FilterChange<F>> {
        self.updates.into_iter()
    }
}

/// Request that may be submitted to a
/// [`DemuxContext::construct()`](trait.DemuxContext.html) implementation.
#[derive(Debug)]
pub enum FilterRequest<'a, 'buf> {
    /// requests a filter implementation for handling a PID contained in the transport stream that
    /// was not announced via other means (PAT/PMT).
    ByPid(packet::Pid),
    /// requests a filter for the stream with the given details which has just been discovered
    /// within a Program Map Table section.
    ByStream {
        /// The `Pid` of the program containing the stream to be handled
        program_pid: packet::Pid,
        /// The type of the stream to be handled
        stream_type: StreamType,
        /// The full PmtSection defining the stream needing to he handled
        pmt: &'a PmtSection<'buf>,
        /// the PMT stream information for the specific stream being handled (which will be one
        /// of the values inside the `pmt` which is also provided.
        stream_info: &'a StreamInfo<'buf>,
    },
    /// Requests a filter implementation for handling Program Map Table sections
    Pmt {
        /// the `Pid` which contains the PMT
        pid: packet::Pid,
        /// the _program number_ of the program for which this will be the PMT
        program_number: u16,
    },
    /// requests a filter implementation to handle packets containing Network Information Table data
    Nit {
        /// The `Pid` of the packets which contain the NIT.
        pid: packet::Pid,
    },
}

struct PmtProcessor<Ctx: DemuxContext> {
    pid: packet::Pid,
    program_number: u16,
    filters_registered: fixedbitset::FixedBitSet,
    phantom: marker::PhantomData<Ctx>,
}

impl<Ctx: DemuxContext> PmtProcessor<Ctx> {
    pub fn new(pid: packet::Pid, program_number: u16) -> PmtProcessor<Ctx> {
        PmtProcessor {
            pid,
            program_number,
            filters_registered: fixedbitset::FixedBitSet::with_capacity(packet::Pid::PID_COUNT),
            phantom: marker::PhantomData,
        }
    }

    fn new_table(
        &mut self,
        ctx: &mut Ctx,
        header: &psi::SectionCommonHeader,
        _table_syntax_header: &psi::TableSyntaxHeader<'_>,
        sect: &PmtSection<'_>,
    ) {
        if 0x02 != header.table_id {
            warn!(
                "[PMT {:?} program:{}] Expected PMT to have table id 0x2, but got {:#x}",
                self.pid, self.program_number, header.table_id
            );
            return;
        }
        // pass the table_id value this far!
        let mut pids_seen = fixedbitset::FixedBitSet::with_capacity(packet::Pid::PID_COUNT);
        for stream_info in sect.streams() {
            let pes_packet_consumer = ctx.construct(FilterRequest::ByStream {
                program_pid: self.pid,
                stream_type: stream_info.stream_type(),
                pmt: sect,
                stream_info: &stream_info,
            });
            ctx.filter_changeset()
                .insert(stream_info.elementary_pid(), pes_packet_consumer);
            pids_seen.insert(usize::from(stream_info.elementary_pid()));
            self.filters_registered
                .insert(usize::from(stream_info.elementary_pid()));
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        self.remove_outdated(ctx, pids_seen);
    }

    fn remove_outdated(&mut self, ctx: &mut Ctx, pids_seen: fixedbitset::FixedBitSet) {
        for pid in self.filters_registered.difference(&pids_seen) {
            ctx.filter_changeset().remove(packet::Pid::new(pid as u16));
        }
        self.filters_registered = pids_seen;
    }
}

impl<Ctx: DemuxContext> psi::WholeSectionSyntaxPayloadParser for PmtProcessor<Ctx> {
    type Context = Ctx;

    fn section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &psi::SectionCommonHeader,
        table_syntax_header: &psi::TableSyntaxHeader<'_>,
        data: &'a [u8],
    ) {
        let start = psi::SectionCommonHeader::SIZE + psi::TableSyntaxHeader::SIZE;
        let end = data.len() - 4; // remove CRC bytes
        match PmtSection::from_bytes(&data[start..end]) {
            Ok(sect) => self.new_table(ctx, header, table_syntax_header, &sect),
            Err(e) => warn!(
                "[PMT {:?} program:{}] problem reading data: {:?}",
                self.pid, self.program_number, e
            ),
        }
    }
}

/// TODO: this type does not belong here
#[derive(Debug)]
pub enum DemuxError {
    /// The transport stream has a syntax error that means there was not enough data present to
    /// parse the requested structure.
    NotEnoughData {
        /// The name of the field we were unable to parse
        field: &'static str,
        /// The expected size of the field data
        expected: usize,
        /// the actual size of date available within the transport stream
        actual: usize,
    },
}

type PacketFilterConsumer<Proc> = psi::SectionPacketConsumer<
    psi::SectionSyntaxSectionProcessor<
        psi::DedupSectionSyntaxPayloadParser<
            psi::BufferSectionSyntaxParser<psi::CrcCheckWholeSectionSyntaxPayloadParser<Proc>>,
        >,
    >,
>;

/// `PacketFilter` implementation which will insert some other `PacketFilter` into the `Demultiplex`
/// instance for each sub-stream listed in one of the stream's PMT-sections.
///
/// The particular `PacketFilter` to be inserted is determined by querying
/// [`DemuxContxt::construct()`](trait.DemuxContext.html), passing a
/// [`FilterRequest::ByStream`](enum.FilterRequest.html#variant.ByStream) request.
pub struct PmtPacketFilter<Ctx: DemuxContext + 'static> {
    pmt_section_packet_consumer: PacketFilterConsumer<PmtProcessor<Ctx>>,
}
impl<Ctx: DemuxContext> PmtPacketFilter<Ctx> {
    /// creates a new `PmtPacketFilter` for PMT sections in packets with the given `Pid`, and for
    /// the given _program number_.
    pub fn new(pid: packet::Pid, program_number: u16) -> PmtPacketFilter<Ctx> {
        let pmt_proc = PmtProcessor::new(pid, program_number);
        PmtPacketFilter {
            pmt_section_packet_consumer: psi::SectionPacketConsumer::new(
                psi::SectionSyntaxSectionProcessor::new(psi::DedupSectionSyntaxPayloadParser::new(
                    psi::BufferSectionSyntaxParser::new(
                        psi::CrcCheckWholeSectionSyntaxPayloadParser::new(pmt_proc),
                    ),
                )),
            ),
        }
    }
}
impl<Ctx: DemuxContext> PacketFilter for PmtPacketFilter<Ctx> {
    type Ctx = Ctx;

    fn consume(&mut self, ctx: &mut Self::Ctx, pk: &packet::Packet<'_>) {
        self.pmt_section_packet_consumer.consume(ctx, pk);
    }
}

struct PatProcessor<Ctx: DemuxContext> {
    filters_registered: fixedbitset::FixedBitSet, // TODO: https://crates.io/crates/typenum_bitset ?
    phantom: marker::PhantomData<Ctx>,
}

impl<Ctx: DemuxContext> Default for PatProcessor<Ctx> {
    fn default() -> PatProcessor<Ctx> {
        PatProcessor {
            filters_registered: fixedbitset::FixedBitSet::with_capacity(packet::Pid::PID_COUNT),
            phantom: marker::PhantomData,
        }
    }
}
impl<Ctx: DemuxContext> PatProcessor<Ctx> {
    fn new_table(
        &mut self,
        ctx: &mut Ctx,
        header: &psi::SectionCommonHeader,
        _table_syntax_header: &psi::TableSyntaxHeader<'_>,
        sect: &pat::PatSection<'_>,
    ) {
        if 0x00 != header.table_id {
            warn!(
                "Expected PAT to have table id 0x0, but got {:#x}",
                header.table_id
            );
            return;
        }
        let mut pids_seen = fixedbitset::FixedBitSet::with_capacity(packet::Pid::PID_COUNT);
        // add or update filters for descriptors we've not seen before,
        for desc in sect.programs() {
            let filter = match desc {
                pat::ProgramDescriptor::Program {
                    program_number,
                    pid,
                } => ctx.construct(FilterRequest::Pmt {
                    pid,
                    program_number,
                }),
                pat::ProgramDescriptor::Network { pid } => {
                    ctx.construct(FilterRequest::Nit { pid })
                }
            };
            ctx.filter_changeset().insert(desc.pid(), filter);
            pids_seen.insert(usize::from(desc.pid()));
            self.filters_registered.insert(usize::from(desc.pid()));
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        self.remove_outdated(ctx, pids_seen);
    }

    fn remove_outdated(&mut self, ctx: &mut Ctx, pids_seen: fixedbitset::FixedBitSet) {
        for pid in self.filters_registered.difference(&pids_seen) {
            ctx.filter_changeset().remove(packet::Pid::new(pid as u16));
        }
        self.filters_registered = pids_seen;
    }
}

impl<Ctx: DemuxContext> psi::WholeSectionSyntaxPayloadParser for PatProcessor<Ctx> {
    type Context = Ctx;

    fn section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &psi::SectionCommonHeader,
        table_syntax_header: &psi::TableSyntaxHeader<'_>,
        data: &'a [u8],
    ) {
        let start = psi::SectionCommonHeader::SIZE + psi::TableSyntaxHeader::SIZE;
        let end = data.len() - 4; // remove CRC bytes
        self.new_table(
            ctx,
            header,
            table_syntax_header,
            &pat::PatSection::new(&data[start..end]),
        );
    }
}

// ---- demux ----

/// Context shared between the `Dumultiplex` object and the `PacketFilter` instances
/// which customise its behavior.
///
/// This trait defines behaviour that the process of demultiplexing requires from the context
/// object, but an application has the opportunity to add further state to the type implementing
/// this trait, if desired.
pub trait DemuxContext: Sized {
    /// the type of `PacketFilter` which the `Demultiplex` object needs to use
    type F: PacketFilter<Ctx = Self>;

    /// mutable reference to the `FilterChangeset` this context holds
    fn filter_changeset(&mut self) -> &mut FilterChangeset<Self::F>;

    /// The application using this crate should provide an implementation of this method that
    /// returns an instance of `PacketFilter` implementing the application's desired handling for
    /// the given content.
    fn construct(&mut self, req: FilterRequest<'_, '_>) -> Self::F;
}

/// `PacketFilter` implementation which will insert some other `PacketFilter` into the `Demultiplex`
/// instance for each program listed in the stream's PAT-section.
///
/// The particular `PacketFilter` to be inserted is determined by querying
/// [`DemuxContext::construct()`](trait.DemuxContext.html), passing a
/// [`FilterRequest::Pmt`](enum.FilterRequest.html#variant.Pmt) request.
pub struct PatPacketFilter<Ctx: DemuxContext> {
    pat_section_packet_consumer: PacketFilterConsumer<PatProcessor<Ctx>>,
}
impl<Ctx: DemuxContext> Default for PatPacketFilter<Ctx> {
    fn default() -> PatPacketFilter<Ctx> {
        let pat_proc = PatProcessor::default();
        PatPacketFilter {
            pat_section_packet_consumer: psi::SectionPacketConsumer::new(
                psi::SectionSyntaxSectionProcessor::new(psi::DedupSectionSyntaxPayloadParser::new(
                    psi::BufferSectionSyntaxParser::new(
                        psi::CrcCheckWholeSectionSyntaxPayloadParser::new(pat_proc),
                    ),
                )),
            ),
        }
    }
}
impl<Ctx: DemuxContext> PacketFilter for PatPacketFilter<Ctx> {
    type Ctx = Ctx;

    fn consume(&mut self, ctx: &mut Self::Ctx, pk: &packet::Packet<'_>) {
        self.pat_section_packet_consumer.consume(ctx, pk);
    }
}

/// Transport Stream demultiplexer.
///
/// Uses the `DemuxContext` passed to `new()` to create Filters for
/// processing the payloads of each packet discovered in the TransportStream.
///
/// # Incremental parsing
///
/// Successive sections of transport stream data can be passed in order to `push()`, and the
/// demultiplexing process will resume at the start of one buffer where it left off at the end of
/// the last.  This supports for example the processing of sections of TS data as they are received
/// from the network, without needing to copy them out of the source network buffer.
pub struct Demultiplex<Ctx: DemuxContext> {
    processor_by_pid: Filters<Ctx::F>,
}
impl<Ctx: DemuxContext> Demultiplex<Ctx> {
    /// Create a `Dumultiplex` instance, and populate it with an initial `PacketFilter` for
    /// handling PAT packets (which is created by the given `DemuxContext` object).
    /// The returned value does not retain any reference to the given `DemuxContext` reference.
    pub fn new(ctx: &mut Ctx) -> Demultiplex<Ctx> {
        let mut result = Demultiplex {
            processor_by_pid: Filters::default(),
        };

        result.processor_by_pid.insert(
            psi::pat::PAT_PID,
            ctx.construct(FilterRequest::ByPid(psi::pat::PAT_PID)),
        );

        result
    }

    /// Parse the Transport Stream packets in the given buffer, using functions from the given
    /// `DemuxContent` object
    pub fn push(&mut self, ctx: &mut Ctx, buf: &[u8]) {
        // TODO: simplify
        let mut itr = buf
            .chunks_exact(packet::Packet::SIZE)
            .map(packet::Packet::try_new);
        let mut pk = if let Some(Some(p)) = itr.next() {
            p
        } else {
            return;
        };
        'outer: loop {
            let this_pid = pk.pid();
            if !self.processor_by_pid.contains(this_pid) {
                self.add_pid_filter(ctx, this_pid);
            };
            let this_proc = self.processor_by_pid.get(this_pid).unwrap();
            'inner: loop {
                if pk.transport_error_indicator() {
                    // drop packets that have transport_error_indicator set, on the assumption that
                    // the contents are nonsense
                    warn!("{:?} transport_error_indicator", pk.pid());
                } else if pk.transport_scrambling_control()
                    != TransportScramblingControl::NotScrambled
                {
                    // TODO: allow descrambler to be plugged in
                    warn!(
                        "{:?} dropping scrambled packet {:?}",
                        pk.pid(),
                        pk.transport_scrambling_control()
                    );
                } else {
                    this_proc.consume(ctx, &pk);
                    if !ctx.filter_changeset().is_empty() {
                        break 'inner;
                    }
                }
                pk = if let Some(Some(p)) = itr.next() {
                    p
                } else {
                    break 'outer;
                };
                if pk.pid() != this_pid {
                    continue 'outer;
                }
            }
            if !ctx.filter_changeset().is_empty() {
                ctx.filter_changeset().apply(&mut self.processor_by_pid);
            }
            debug_assert!(ctx.filter_changeset().is_empty());
            pk = if let Some(Some(p)) = itr.next() {
                p
            } else {
                break 'outer;
            };
        }
    }

    fn add_pid_filter(&mut self, ctx: &mut Ctx, this_pid: packet::Pid) {
        let filter = ctx.construct(FilterRequest::ByPid(this_pid));
        self.processor_by_pid.insert(this_pid, filter);
    }
}

#[cfg(test)]
pub(crate) mod test {
    use bitstream_io::{BitWrite, BitWriter, BE};
    use hex_literal::*;
    use std::io;

    use crate::demultiplex;
    use crate::packet;
    use crate::psi;
    use crate::psi::WholeSectionSyntaxPayloadParser;
    use bitstream_io::BigEndian;

    packet_filter_switch! {
        NullFilterSwitch<NullDemuxContext> {
            Pat: demultiplex::PatPacketFilter<NullDemuxContext>,
            Pmt: demultiplex::PmtPacketFilter<NullDemuxContext>,
            Nul: demultiplex::NullPacketFilter<NullDemuxContext>,
        }
    }
    demux_context!(NullDemuxContext, NullFilterSwitch);
    impl NullDemuxContext {
        fn do_construct(&mut self, req: demultiplex::FilterRequest<'_, '_>) -> NullFilterSwitch {
            match req {
                demultiplex::FilterRequest::ByPid(psi::pat::PAT_PID) => {
                    NullFilterSwitch::Pat(demultiplex::PatPacketFilter::default())
                }
                demultiplex::FilterRequest::ByPid(_) => {
                    NullFilterSwitch::Nul(demultiplex::NullPacketFilter::default())
                }
                demultiplex::FilterRequest::ByStream { .. } => {
                    NullFilterSwitch::Nul(demultiplex::NullPacketFilter::default())
                }
                demultiplex::FilterRequest::Pmt {
                    pid,
                    program_number,
                } => NullFilterSwitch::Pmt(demultiplex::PmtPacketFilter::new(pid, program_number)),
                demultiplex::FilterRequest::Nit { .. } => {
                    NullFilterSwitch::Nul(demultiplex::NullPacketFilter::default())
                }
            }
        }
    }

    #[test]
    fn demux_empty() {
        let mut ctx = NullDemuxContext::new();
        let mut deplex = demultiplex::Demultiplex::new(&mut ctx);
        deplex.push(&mut ctx, &[0x0; 0][..]);
    }

    #[test]
    fn pat() {
        // TODO: better
        let buf = hex!("474000150000B00D0001C100000001E1E02D507804FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");
        let mut ctx = NullDemuxContext::new();
        let mut deplex = demultiplex::Demultiplex::new(&mut ctx);
        deplex.push(&mut ctx, &buf[..]);
    }

    #[test]
    fn pat_no_existing_program() {
        let mut processor = demultiplex::PatProcessor::default();
        let section = vec![
            0, 0, 0, // common header
            // table syntax header
            0x0D, 0x00, 0b00000001, 0xC1, 0x00, // PAT section
            0, 1, // program_number
            0, 101, // pid
            0, 0, 0, 0, // CRC (incorrect!)
        ];
        let header = psi::SectionCommonHeader::new(&section[..psi::SectionCommonHeader::SIZE]);
        let table_syntax_header =
            psi::TableSyntaxHeader::new(&section[psi::SectionCommonHeader::SIZE..]);
        let mut ctx = NullDemuxContext::new();
        processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        let mut changes = ctx.changeset.updates.into_iter();
        if let Some(demultiplex::FilterChange::Insert(pid, _)) = changes.next() {
            assert_eq!(packet::Pid::new(101), pid);
        } else {
            panic!();
        }
    }

    #[test]
    fn pat_remove_existing_program() {
        let mut ctx = NullDemuxContext::new();
        let mut processor = demultiplex::PatProcessor::default();
        {
            let section = vec![
                0, 0, 0, // common header
                // table syntax header
                0x0D, 0x00, 0b00000001, 0xC1, 0x00,
                // PAT with a single program; next version of the  table removes this,
                0, 1, // program_number
                0, 101, // pid
                0, 0, 0, 0, // CRC (incorrect)
            ];
            let header = psi::SectionCommonHeader::new(&section[..psi::SectionCommonHeader::SIZE]);
            let table_syntax_header =
                psi::TableSyntaxHeader::new(&section[psi::SectionCommonHeader::SIZE..]);
            processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        }
        ctx.changeset.updates.clear();
        {
            let section = vec![
                0, 0, 0, // common header
                // table syntax header
                0x0D, 0x00, 0b00000011, 0xC1, 0x00, // new version!
                // empty PMT - simulate removal of PID 101
                0, 0, 0, 0, // CRC (incorrect)
            ];
            let header = psi::SectionCommonHeader::new(&section[..psi::SectionCommonHeader::SIZE]);
            let table_syntax_header =
                psi::TableSyntaxHeader::new(&section[psi::SectionCommonHeader::SIZE..]);
            processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        }
        let mut changes = ctx.changeset.updates.into_iter();
        if let Some(demultiplex::FilterChange::Remove(pid)) = changes.next() {
            assert_eq!(packet::Pid::new(101), pid);
        } else {
            panic!();
        }
    }

    pub(crate) fn make_test_data<F>(builder: F) -> Vec<u8>
    where
        F: Fn(&mut BitWriter<Vec<u8>, BE>) -> Result<(), io::Error>,
    {
        let data: Vec<u8> = Vec::new();
        let mut w = BitWriter::endian(data, BigEndian);
        builder(&mut w).unwrap();
        w.into_writer()
    }

    #[test]
    fn pmt_new_stream() {
        // TODO arrange for the filter table to already contain an entry for PID 101
        let pid = packet::Pid::new(101);
        let program_number = 1001;
        let mut processor = demultiplex::PmtProcessor::new(pid, program_number);
        let section = make_test_data(|w| {
            // common section header,
            w.write(8, 0x02)?; // table_id
            w.write_bit(true)?; // section_syntax_indicator
            w.write_bit(false)?; // private_indicator
            w.write(2, 3)?; // reserved
            w.write(12, 20)?; // section_length

            // section syntax header,
            w.write(16, 0)?; // id
            w.write(2, 3)?; // reserved
            w.write(5, 0)?; // version
            w.write(1, 1)?; // current_next_indicator
            w.write(8, 0)?; // section_number
            w.write(8, 0)?; // last_section_number

            // PMT section payload
            w.write(3, 7)?; // reserved
            w.write(13, 123)?; // pcr_pid
            w.write(4, 15)?; // reserved
            w.write(12, 0)?; // program_info_length

            // program_info_length=0, so no descriptors follow; straight into stream info
            w.write(8, 0)?; // stream_type
            w.write(3, 7)?; // reserved
            w.write(13, 201)?; // elementary_pid
            w.write(4, 15)?; // reserved
            w.write(12, 6)?; // es_info_length

            // and now, two made-up descriptors which need to fill up es_info_length-bytes
            w.write(8, 0)?; // descriptor_tag
            w.write(8, 1)?; // descriptor_length
            w.write(8, 0)?; // made-up descriptor data not following any spec

            // second descriptor
            w.write(8, 0)?; // descriptor_tag
            w.write(8, 1)?; // descriptor_length
            w.write(8, 0)?; // made-up descriptor data not following any spec
            w.write(32, 0) // CRC (incorrect)
        });
        let header = psi::SectionCommonHeader::new(&section[..psi::SectionCommonHeader::SIZE]);
        let table_syntax_header =
            psi::TableSyntaxHeader::new(&section[psi::SectionCommonHeader::SIZE..]);
        let mut ctx = NullDemuxContext::new();
        processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        let mut changes = ctx.changeset.updates.into_iter();
        if let Some(demultiplex::FilterChange::Insert(pid, _)) = changes.next() {
            assert_eq!(packet::Pid::new(201), pid);
        } else {
            panic!();
        }
    }
}
