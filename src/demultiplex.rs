//! Main types implementing the demultiplexer state-machine.
//!
//! Construct an instance of [`Demultiplex`](struct.Demultiplex.html) and feed it one a succession
//! of byte-slices containing the Transport Stream data.
//!
//! Users of this crate are expected to provide their own implementations of,
//!
//!  - [`StreamConstructor`](trail.StreamConstructor.html) - to create specific `PacketFilter` instances
//!    for each type of sub-stream found within the Transport Stream data.
//!  - [`PacketFilter`](trait.PacketFilter.html) - for defining per-stream-type handling, possibly
//!    by using the [`packet_filter_switch!()`](../macro.packet_filter_switch.html) macro to create
//!    an enum implementing this trait.
//!  - [`DemuxContext`](trait.DemuxContext.html) - to specify the specific `StreamConstructor` and
//!    `DemuxContext` types to actually be used. possibly by using the
//!    [`demux_context!()`](../macro.demux_context.html) macro.

use crate::packet;
use crate::psi;
use crate::psi::pat;
use crate::psi::pmt::PmtSection;
use crate::psi::pmt::StreamInfo;
use crate::StreamType;
use fixedbitset;
use std;
use std::marker;

/// Trait to which `Demultiplex` delegates handling of subsets of Transport Stream packets.
///
/// A packet filter can collaborate with the owning `Demultiplex` instance via the `DemuxContext`
/// to which they will both be lent mutable access.  For example, by making entries in the
/// `FilterChangeset` owned by the `DemuxContext`, a `PacketFilter` implementation may alter
/// the handling of subsequent packets in the Transport Stream.
pub trait PacketFilter {
    type Ctx: DemuxContext;

    fn consume(&mut self, ctx: &mut Self::Ctx, pk: &packet::Packet<'_>);
}

/// No-op implementation of `PacketFilter`.
///
/// Sometimes a Transport Stream will contain packets that you know you want to ignore.  This type
/// can be used when you have to register a filter for such packets with the demultiplexer.
pub struct NullPacketFilter<Ctx: DemuxContext> {
    phantom: marker::PhantomData<Ctx>,
}
impl<Ctx: DemuxContext> NullPacketFilter<Ctx> {
    pub fn construct(
        _pmt: &PmtSection<'_>,
        _stream_info: &StreamInfo<'_>,
    ) -> NullPacketFilter<Ctx> {
        Self::default()
    }
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
/// # Example
///
/// ```
/// # #[macro_use]
/// # extern crate mpeg2ts_reader;
/// # use mpeg2ts_reader::demultiplex::StreamConstructor;
/// # use mpeg2ts_reader::demultiplex::FilterRequest;
/// # fn main() {
/// // Create an enum that implements PacketFilter as required by your application.
/// packet_filter_switch!{
///     MyFilterSwitch<MyDemuxContext> {
///         Pat: mpeg2ts_reader::demultiplex::PatPacketFilter<MyDemuxContext>,
///         Pmt: mpeg2ts_reader::demultiplex::PmtPacketFilter<MyDemuxContext>,
///         Nul: mpeg2ts_reader::demultiplex::NullPacketFilter<MyDemuxContext>,
///     }
/// };
///
/// // Create an implementation of the DemuxContext trait for the PacketFilter implementation
/// // created above.
/// demux_context!(MyDemuxContext, MyStreamConstructor);
///
/// pub struct MyStreamConstructor;
/// impl StreamConstructor for MyStreamConstructor {
///     /// ...
/// #    type F = MyFilterSwitch;
/// #    fn construct(&mut self, req: FilterRequest) -> Self::F {
/// #        unimplemented!();
/// #    }
/// }
///
/// let mut ctx = MyDemuxContext::new(MyStreamConstructor);
/// // .. use the ctx value while demultiplexing some data ..
/// # }
/// ```
#[macro_export]
macro_rules! demux_context {
    ($name:ident, $ctor:ty) => {
        pub struct $name {
            changeset: $crate::demultiplex::FilterChangeset<
                <$ctor as $crate::demultiplex::StreamConstructor>::F,
            >,
            constructor: $ctor,
        }
        impl $name {
            pub fn new(constructor: $ctor) -> Self {
                $name {
                    changeset: $crate::demultiplex::FilterChangeset::default(),
                    constructor,
                }
            }
        }
        impl $crate::demultiplex::DemuxContext for $name {
            type F = <$ctor as $crate::demultiplex::StreamConstructor>::F;
            type Ctor = $ctor;

            fn filter_changeset(&mut self) -> &mut $crate::demultiplex::FilterChangeset<Self::F> {
                &mut self.changeset
            }
            fn filter_constructor(&mut self) -> &mut $ctor {
                &mut self.constructor
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
    Insert(packet::Pid, F),
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
/// [`StreamConstructor`](trait.StreamConstructor.html) implementation.
pub enum FilterRequest<'a, 'buf: 'a> {
    /// requests a filter implementation for handling a PID contained in the transport stream that
    /// was not announced via other means (PAT/PMT).
    ByPid(packet::Pid),
    /// requests a filter for the stream with the given details which has just been discovered
    /// within a Program Map Table section.
    ByStream {
        program_pid: packet::Pid,
        stream_type: StreamType,
        pmt: &'a PmtSection<'buf>,
        stream_info: &'a StreamInfo<'buf>,
    },
    /// Requests a filter implementation for handling Program Map Table sections
    Pmt {
        pid: packet::Pid,
        program_number: u16,
    },
    /// requests a filter implementation to handle packets containing Network Information Table data
    Nit { pid: packet::Pid },
}

/// Trait for type able to construct a `PacketFilter` instance for a given `FilterRequest`.
pub trait StreamConstructor {
    type F: PacketFilter;

    fn construct(&mut self, req: FilterRequest<'_, '_>) -> Self::F;
}

struct PmtProcessor<Ctx: DemuxContext> {
    pid: packet::Pid,
    program_number: u16,
    current_version: Option<u8>,
    filters_registered: fixedbitset::FixedBitSet,
    phantom: marker::PhantomData<Ctx>,
}

impl<Ctx: DemuxContext> PmtProcessor<Ctx> {
    pub fn new(pid: packet::Pid, program_number: u16) -> PmtProcessor<Ctx> {
        PmtProcessor {
            pid,
            program_number,
            current_version: None,
            filters_registered: fixedbitset::FixedBitSet::with_capacity(packet::Pid::PID_COUNT),
            phantom: marker::PhantomData,
        }
    }

    fn new_table(
        &mut self,
        ctx: &mut Ctx,
        header: &psi::SectionCommonHeader,
        table_syntax_header: &psi::TableSyntaxHeader<'_>,
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
            let pes_packet_consumer = ctx.filter_constructor().construct(FilterRequest::ByStream {
                program_pid: self.pid,
                stream_type: stream_info.stream_type(),
                pmt: &sect,
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

        self.current_version = Some(table_syntax_header.version());
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
    NotEnoughData {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
}

/// `PacketFilter` implementation which will insert some other `PacketFilter` into the `Demultiplex`
/// instance for each sub-stream listed in one of the stream's PMT-sections.
///
/// The particular `PacketFilter` to be inserted is determined by querying the
/// [`StreamConstructor`](trait.StreamConstructor.html) from the `DemuxContext`, passing a
/// [`FilterRequest::ByStream`](enum.FilterRequest.html#variant.ByStream) request.
pub struct PmtPacketFilter<Ctx: DemuxContext + 'static> {
    pmt_section_packet_consumer: psi::SectionPacketConsumer<
        psi::SectionSyntaxSectionProcessor<
            psi::DedupSectionSyntaxPayloadParser<
                psi::BufferSectionSyntaxParser<
                    psi::CrcCheckWholeSectionSyntaxPayloadParser<PmtProcessor<Ctx>>,
                >,
            >,
        >,
    >,
}
impl<Ctx: DemuxContext> PmtPacketFilter<Ctx> {
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
    current_version: Option<u8>,
    filters_registered: fixedbitset::FixedBitSet, // TODO: https://crates.io/crates/typenum_bitset ?
    phantom: marker::PhantomData<Ctx>,
}

impl<Ctx: DemuxContext> Default for PatProcessor<Ctx> {
    fn default() -> PatProcessor<Ctx> {
        PatProcessor {
            current_version: None,
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
        table_syntax_header: &psi::TableSyntaxHeader<'_>,
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
                } => ctx.filter_constructor().construct(FilterRequest::Pmt {
                    pid,
                    program_number,
                }),
                pat::ProgramDescriptor::Network { pid } => ctx
                    .filter_constructor()
                    .construct(FilterRequest::Nit { pid }),
            };
            ctx.filter_changeset().insert(desc.pid(), filter);
            pids_seen.insert(usize::from(desc.pid()));
            self.filters_registered.insert(usize::from(desc.pid()));
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        self.remove_outdated(ctx, pids_seen);

        self.current_version = Some(table_syntax_header.version());
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

pub trait DemuxContext: Sized {
    type F: PacketFilter<Ctx = Self>;
    type Ctor: StreamConstructor<F = Self::F>;

    fn filter_changeset(&mut self) -> &mut FilterChangeset<Self::F>;
    fn filter_constructor(&mut self) -> &mut Self::Ctor;
}

/// `PacketFilter` implementation which will insert some other `PacketFilter` into the `Demultiplex`
/// instance for each program listed in the stream's PAT-section.
///
/// The particular `PacketFilter` to be inserted is determined by querying the
/// [`StreamConstructor`](trait.StreamConstructor.html) from the `DemuxContext`, passing a
/// [`FilterRequest::Pmt`](enum.FilterRequest.html#variant.Pmt) request.
pub struct PatPacketFilter<Ctx: DemuxContext> {
    pat_section_packet_consumer: psi::SectionPacketConsumer<
        psi::SectionSyntaxSectionProcessor<
            psi::DedupSectionSyntaxPayloadParser<
                psi::BufferSectionSyntaxParser<
                    psi::CrcCheckWholeSectionSyntaxPayloadParser<PatProcessor<Ctx>>,
                >,
            >,
        >,
    >,
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
/// Uses the `StreamConstructor` of the `DemuxContext` passed to `new()` to create Filters for
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
    pub fn new(ctx: &mut Ctx) -> Demultiplex<Ctx> {
        let mut result = Demultiplex {
            processor_by_pid: Filters::default(),
        };

        result.processor_by_pid.insert(
            packet::Pid::PAT,
            ctx.filter_constructor()
                .construct(FilterRequest::ByPid(packet::Pid::PAT)),
        );

        result
    }

    pub fn push(&mut self, ctx: &mut Ctx, buf: &[u8]) {
        // TODO: simplify
        // (maybe once chunks_exact() is stable; right now an iterator-based version of the
        // following always comes out performing slower, when I try.)
        let mut i = 0;
        loop {
            let end = i + packet::Packet::SIZE;
            if end > buf.len() {
                break;
            }
            let mut pk_buf = &buf[i..end];
            if packet::Packet::is_sync_byte(pk_buf[0]) {
                {
                    let mut pk = packet::Packet::new(pk_buf);
                    let this_pid = pk.pid();
                    if !self.processor_by_pid.contains(this_pid) {
                        let filter = ctx
                            .filter_constructor()
                            .construct(FilterRequest::ByPid(this_pid));
                        self.processor_by_pid.insert(this_pid, filter);
                    };
                    let this_proc = self.processor_by_pid.get(this_pid).unwrap();
                    while ctx.filter_changeset().is_empty() {
                        this_proc.consume(ctx, &pk);
                        i += packet::Packet::SIZE;
                        let end = i + packet::Packet::SIZE;
                        if end > buf.len() {
                            break;
                        }
                        pk_buf = &buf[i..end];
                        if !packet::Packet::is_sync_byte(pk_buf[0]) {
                            // TODO: attempt to resynchronise
                            return;
                        }
                        pk = packet::Packet::new(pk_buf);
                        if pk.pid() != this_pid {
                            i -= packet::Packet::SIZE;
                            break;
                        }
                    }
                }
                if !ctx.filter_changeset().is_empty() {
                    ctx.filter_changeset().apply(&mut self.processor_by_pid);
                }
                debug_assert!(ctx.filter_changeset().is_empty());
            } else {
                // TODO: attempt to resynchronise
                return;
            }
            i += packet::Packet::SIZE;
        }
    }
}

#[cfg(test)]
mod test {
    use bitstream_io::{BitWriter, BE};
    use data_encoding::base16;
    use std::io;

    use crate::demultiplex;
    use crate::packet;
    use crate::psi;
    use crate::psi::WholeSectionSyntaxPayloadParser;

    packet_filter_switch!{
        NullFilterSwitch<NullDemuxContext> {
            Pat: demultiplex::PatPacketFilter<NullDemuxContext>,
            Pmt: demultiplex::PmtPacketFilter<NullDemuxContext>,
            Nul: demultiplex::NullPacketFilter<NullDemuxContext>,
        }
    }
    demux_context!(NullDemuxContext, NullStreamConstructor);

    pub struct NullStreamConstructor;
    impl demultiplex::StreamConstructor for NullStreamConstructor {
        type F = NullFilterSwitch;

        fn construct(&mut self, req: demultiplex::FilterRequest<'_, '_>) -> Self::F {
            match req {
                demultiplex::FilterRequest::ByPid(packet::Pid::PAT) => {
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
        let mut ctx = NullDemuxContext::new(NullStreamConstructor);
        let mut deplex = demultiplex::Demultiplex::new(&mut ctx);
        deplex.push(&mut ctx, &[0x0; 0][..]);
    }

    #[test]
    fn pat() {
        // TODO: better
        let buf = base16::decode(b"474000150000B00D0001C100000001E1E02D507804FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").unwrap();
        let mut ctx = NullDemuxContext::new(NullStreamConstructor);
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
        let mut ctx = NullDemuxContext::new(NullStreamConstructor);
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
        let mut ctx = NullDemuxContext::new(NullStreamConstructor);
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

    fn make_test_data<F>(builder: F) -> Vec<u8>
    where
        F: Fn(BitWriter<'_, BE>) -> Result<(), io::Error>,
    {
        let mut data: Vec<u8> = Vec::new();
        builder(BitWriter::<BE>::new(&mut data)).unwrap();
        data
    }

    #[test]
    fn pmt_new_stream() {
        // TODO arrange for the filter table to already contain an entry for PID 101
        let pid = packet::Pid::new(101);
        let program_number = 1001;
        let mut processor = demultiplex::PmtProcessor::new(pid, program_number);
        let section = make_test_data(|mut w| {
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
        let mut ctx = NullDemuxContext::new(NullStreamConstructor);
        processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        let mut changes = ctx.changeset.updates.into_iter();
        if let Some(demultiplex::FilterChange::Insert(pid, _)) = changes.next() {
            assert_eq!(packet::Pid::new(201), pid);
        } else {
            panic!();
        }
    }
}
