use std::collections::HashSet;
use packet;
use psi;
use std;
use fixedbitset;
use StreamType;
use std::marker;
use psi::pmt::PmtSection;
use psi::pmt::StreamInfo;
use psi::pat;

// TODO: Pid = u16;

pub trait PacketFilter {
    type Ctx: DemuxContext;

    fn consume(&mut self, ctx: &mut Self::Ctx, pk: &packet::Packet);
}

pub struct NullPacketFilter<Ctx: DemuxContext> {
    phantom: marker::PhantomData<Ctx>,
}
impl<Ctx: DemuxContext> NullPacketFilter<Ctx> {
    pub fn construct(_pmt: &PmtSection, _stream_info: &StreamInfo) -> NullPacketFilter<Ctx> {
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
    fn consume(&mut self, _ctx: &mut Self::Ctx, _pk: &packet::Packet) {
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
            changeset: $crate::demultiplex::FilterChangeset<<$ctor as $crate::demultiplex::StreamConstructor>::F>,
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
            fn consume(&mut self, ctx: &mut $ctx, pk: &$crate::packet::Packet) {
                match self {
                    $( &mut $name::$case_name(ref mut f) => f.consume(ctx, pk), )*

                }
            }
        }
    }
}
pub struct Filters<F: PacketFilter> {
    filters_by_pid: Vec<Option<F>>
}
impl<F: PacketFilter> Default for Filters<F> {
    fn default() -> Filters<F> {
        Filters {
            filters_by_pid: vec!(),
        }
    }
}
impl<F: PacketFilter> Filters<F> {
    pub fn contains(&self, pid: u16) -> bool {
        (pid as usize) < self.filters_by_pid.len()
            && self.filters_by_pid[pid as usize].is_some()
    }

    pub fn get(&mut self, pid: u16) -> Option<&mut F> {
        if pid as usize >= self.filters_by_pid.len() {
            None
        } else {
            self.filters_by_pid[pid as usize].as_mut()
        }
    }

    pub fn insert(&mut self, pid: u16, filter: F) {
        let diff = pid as isize - self.filters_by_pid.len() as isize;
        if diff >= 0 {
            for _ in 0..diff+1 {
                self.filters_by_pid.push(None);
            }
        }
        self.filters_by_pid[pid as usize] = Some(filter);
    }

    pub fn remove(&mut self, pid: u16) {
        if (pid as usize) < self.filters_by_pid.len() {
            self.filters_by_pid[pid as usize] = None;
        }
    }

    pub fn pids(&self) -> Vec<u16> {
        self.filters_by_pid.iter().enumerate().filter_map(|(i, e)| { if e.is_some() { Some(i as u16) } else { None } } ).collect()
    }
}


// A filter can't change the map of filters-by-pid that it is itself owned by while the filter is
// running, so this changeset protocol allows a filter to specify any filter updates required so
// the demultiplexer can apply them when the filter is complete

pub enum FilterChange<F: PacketFilter> {
    Insert(u16, F),
    Remove(u16),
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
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        match *self {
            FilterChange::Insert(pid, _) => write!(f, "FilterChange::Insert {{ {}, ... }}", pid),
            FilterChange::Remove(pid) => write!(f, "FilterChange::Remove {{ {}, ... }}", pid),
        }
    }
}

#[derive(Debug)]
pub struct FilterChangeset<F: PacketFilter> {
    updates: Vec<FilterChange<F>>
}
impl<F: PacketFilter> Default for FilterChangeset<F> {
    fn default() -> FilterChangeset<F> {
        FilterChangeset { updates: Vec::new() }
    }
}
impl<F: PacketFilter> FilterChangeset<F> {
    fn insert(&mut self, pid: u16, filter: F) {
        self.updates.push(FilterChange::Insert(pid, filter))
    }
    fn remove(&mut self, pid: u16) {
        self.updates.push(FilterChange::Remove(pid))
    }

    fn apply(&mut self, filters: &mut Filters<F>) {
        for update in self.updates.drain(..) {
            update.apply(filters);
        }
    }
    fn is_empty(&self) -> bool {
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

pub enum FilterRequest<'a, 'buf: 'a> {
    ByPid(u16),
    ByStream(StreamType, &'a PmtSection<'buf>, &'a StreamInfo<'buf>),
    Pmt{pid: u16, program_number: u16},
    // requests a filter implementation to handle packets containing Network Information Table data
    Nit { pid: u16 },
}

// TODO: would be nice to have an impl of this trait for `Fn(FilterRequest)->F`, but that ends up
// not being usable without having additional type-parameters over several parts of the API.
pub trait StreamConstructor {
    type F: PacketFilter;

    fn construct(&mut self, req: FilterRequest) -> Self::F;
}

pub struct PmtProcessor<Ctx: DemuxContext> {
    pid: u16,
    program_number: u16,
    current_version: Option<u8>,
    filters_registered: fixedbitset::FixedBitSet,
    phantom: marker::PhantomData<Ctx>,
}

impl<Ctx: DemuxContext> PmtProcessor<Ctx> {
    pub fn new(pid:u16, program_number: u16) -> PmtProcessor<Ctx> {
        PmtProcessor {
            pid,
            program_number,
            current_version: None,
            filters_registered: fixedbitset::FixedBitSet::with_capacity(0x2000),
            phantom: marker::PhantomData,
        }
    }

    fn new_table(&mut self, ctx: &mut Ctx, header: &psi::SectionCommonHeader, table_syntax_header: &psi::TableSyntaxHeader, sect: &PmtSection) {
        if 0x02 != header.table_id {
            println!("[PMT pid:{} program:{}] Expected PMT to have table id 0x2, but got {:#x}", self.pid, self.program_number, header.table_id);
            return;
        }
        // pass the table_id value this far!
        let mut pids_seen = HashSet::new();
        for stream_info in sect.streams() {
            let pes_packet_consumer = ctx.filter_constructor().construct(FilterRequest::ByStream(stream_info.stream_type(), &sect, &stream_info));
            ctx.filter_changeset().insert(stream_info.elementary_pid(), pes_packet_consumer);
            pids_seen.insert(stream_info.elementary_pid());
            self.filters_registered.insert(stream_info.elementary_pid() as usize);
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        for pid in 0..0x1fff {
            if self.filters_registered.contains(pid) && !pids_seen.contains(&(pid as u16)) {
                ctx.filter_changeset().remove(pid as u16);
                self.filters_registered.set(pid, false);
            }
        }
        self.current_version = Some(table_syntax_header.version());
    }
}

impl<Ctx: DemuxContext> psi::WholeSectionSyntaxPayloadParser for PmtProcessor<Ctx> {
    type Context = Ctx;

    fn section<'a>(&mut self, ctx: &mut Self::Context, header: &psi::SectionCommonHeader, table_syntax_header: &psi::TableSyntaxHeader, data: &'a [u8]) {
        let start = psi::SectionCommonHeader::SIZE+psi::TableSyntaxHeader::SIZE;
        let end = data.len() - 4;  // remove CRC bytes
        match PmtSection::from_bytes(&data[start..end]) {
            Ok(sect) => self.new_table(ctx, header, table_syntax_header, &sect),
            Err(e) => println!("[PMT pid:{} program:{}] problem reading data: {:?}", self.pid, self.program_number, e),
        }
    }
}


#[derive(Debug)]
pub enum DemuxError {
    NotEnoughData{ field: &'static str, expected: usize, actual: usize }
}

pub struct PmtPacketFilter<Ctx: DemuxContext + 'static> {
    pmt_section_packet_consumer: psi::SectionPacketConsumer<
        psi::SectionSyntaxSectionProcessor<
            psi::DedupSectionSyntaxPayloadParser<
                psi::BufferSectionSyntaxParser<
                    psi::CrcCheckWholeSectionSyntaxPayloadParser<
                        PmtProcessor<Ctx>
                    >
                >
            >
        >
    >,
}
impl<Ctx: DemuxContext> PmtPacketFilter<Ctx> {
    pub fn new(pid: u16, program_number: u16) -> PmtPacketFilter<Ctx> {
        let pmt_proc = PmtProcessor::new(pid, program_number);
        PmtPacketFilter {
            pmt_section_packet_consumer: psi::SectionPacketConsumer::new(
                psi::SectionSyntaxSectionProcessor::new(
                    psi::DedupSectionSyntaxPayloadParser::new(
                        psi::BufferSectionSyntaxParser::new(
                            psi::CrcCheckWholeSectionSyntaxPayloadParser::new(
                                pmt_proc
                            )
                        )
                    )
                )
            )
        }
    }
}
impl<Ctx: DemuxContext> PacketFilter for PmtPacketFilter<Ctx> {
    type Ctx = Ctx;

    fn consume(&mut self, ctx: &mut Self::Ctx, pk: &packet::Packet) {
        self.pmt_section_packet_consumer.consume(ctx, pk);
    }
}

pub struct PatProcessor<Ctx: DemuxContext> {
    current_version: Option<u8>,
    filters_registered: fixedbitset::FixedBitSet,
    phantom: marker::PhantomData<Ctx>,
}

impl<Ctx: DemuxContext> Default for PatProcessor<Ctx> {
    fn default() -> PatProcessor<Ctx> {
        PatProcessor {
            current_version: None,
            filters_registered: fixedbitset::FixedBitSet::with_capacity(0x2000),
            phantom: marker::PhantomData,
        }
    }
}
impl<Ctx: DemuxContext> PatProcessor<Ctx> {
    fn new_table(&mut self, ctx: &mut Ctx, header: &psi::SectionCommonHeader, table_syntax_header: &psi::TableSyntaxHeader, sect: &pat::PatSection) {
        if 0x00 != header.table_id {
            println!("Expected PAT to have table id 0x0, but got {:#x}", header.table_id);
            return;
        }
        let mut pids_seen = HashSet::new();
        // add or update filters for descriptors we've not seen before,
        for desc in sect.programs() {
            let filter = match desc {
                pat::ProgramDescriptor::Program { program_number, pid } => ctx.filter_constructor().construct(FilterRequest::Pmt { pid, program_number }),
                pat::ProgramDescriptor::Network { pid } => ctx.filter_constructor().construct(FilterRequest::Nit { pid }),
            };
            ctx.filter_changeset().insert(desc.pid(), filter);
            pids_seen.insert(desc.pid());
            self.filters_registered.insert(desc.pid() as usize);
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        for pid in 0..0x1fff {
            if self.filters_registered.contains(pid) && !pids_seen.contains(&(pid as u16)) {
                ctx.filter_changeset().remove(pid as u16);
                self.filters_registered.set(pid, false);
            }
        }

        self.current_version = Some(table_syntax_header.version());
    }
}


impl<Ctx: DemuxContext> psi::WholeSectionSyntaxPayloadParser for PatProcessor<Ctx> {
    type Context = Ctx;

    fn section<'a>(&mut self, ctx: &mut Self::Context, header: &psi::SectionCommonHeader, table_syntax_header: &psi::TableSyntaxHeader, data: &'a [u8]) {
        let start = psi::SectionCommonHeader::SIZE+psi::TableSyntaxHeader::SIZE;
        let end = data.len() - 4;  // remove CRC bytes
        self.new_table(ctx, header, table_syntax_header, &pat::PatSection::new(&data[start..end]));
    }
}

// ---- demux ----

/// an implementation of `PacketFilter` that will log a message the first time that `consume()` is
/// called, reporting the PID of the given packet.  Register this pid filter as the 'default' in
/// order to have diagnostic logging for packets within the Transport Stream that were not
/// announced in the PAT or PMT tables.
///
/// If you do not want those diagnostic messages, use `NullPacketFilter` as the default instead.
pub struct UnhandledPid<Ctx: DemuxContext> {
    pid_seen: bool,
    phantom: marker::PhantomData<Ctx>,
}
impl<Ctx: DemuxContext> Default for UnhandledPid<Ctx> {
    fn default() -> UnhandledPid<Ctx> {
        UnhandledPid {
            pid_seen: false,
            phantom: marker::PhantomData
        }
    }
}
impl<Ctx: DemuxContext> PacketFilter for UnhandledPid<Ctx> {
    type Ctx = Ctx;
    fn consume(&mut self, _ctx: &mut Self::Ctx, pk: &packet::Packet) {
        if !self.pid_seen {
            println!("unhandled pid {}", pk.pid());
            self.pid_seen = true;
        }
    }
}

pub trait DemuxContext: Sized {
    type F: PacketFilter<Ctx=Self>;
    type Ctor: StreamConstructor<F=Self::F>;

    fn filter_changeset(&mut self) -> &mut FilterChangeset<Self::F>;
    fn filter_constructor(&mut self) -> &mut Self::Ctor;
}

pub struct PatPacketFilter<Ctx: DemuxContext> {
    pat_section_packet_consumer: psi::SectionPacketConsumer<
        psi::SectionSyntaxSectionProcessor<
            psi::DedupSectionSyntaxPayloadParser<
                psi::BufferSectionSyntaxParser<
                    psi::CrcCheckWholeSectionSyntaxPayloadParser<
                        PatProcessor<Ctx>
                    >
                >
            >
        >
    >,
}
impl<Ctx: DemuxContext> Default for PatPacketFilter<Ctx> {
    fn default() -> PatPacketFilter<Ctx> {
        let pat_proc = PatProcessor::default();
        PatPacketFilter {
            pat_section_packet_consumer: psi::SectionPacketConsumer::new(
                psi::SectionSyntaxSectionProcessor::new(
                    psi::DedupSectionSyntaxPayloadParser::new(
                        psi::BufferSectionSyntaxParser::new(
                            psi::CrcCheckWholeSectionSyntaxPayloadParser::new(pat_proc)
                        )
                    )
                )
            ),
        }
    }
}
impl<Ctx: DemuxContext> PacketFilter for PatPacketFilter<Ctx> {
    type Ctx = Ctx;

    fn consume(&mut self, ctx: &mut Self::Ctx, pk: &packet::Packet) {
        self.pat_section_packet_consumer.consume(ctx, pk);
    }
}

pub struct Demultiplex<Ctx: DemuxContext> {
    processor_by_pid: Filters<Ctx::F>,
}
impl<Ctx: DemuxContext> Demultiplex<Ctx> {
    pub fn new(ctx: &mut Ctx) -> Demultiplex<Ctx> {
        let mut result = Demultiplex {
            processor_by_pid: Filters::default(),
        };

        result.processor_by_pid.insert(0, ctx.filter_constructor().construct(FilterRequest::ByPid(0)));

        result
    }

    pub fn push(&mut self, ctx: &mut Ctx, buf: &[u8]) {
        // TODO: simplify
        // (maybe once chunks_exact() is stable; right now an iterator-based version of the
        // following always comes out performing slower, when I try.)
        let mut i=0;
        loop {
            let end = i+packet::Packet::SIZE;
            if end > buf.len() {
                break;
            }
            let mut pk_buf = &buf[i..end];
            if packet::Packet::is_sync_byte(pk_buf[0]) {
                {
                    let mut pk = packet::Packet::new(pk_buf);
                    let this_pid = pk.pid();
                    if !self.processor_by_pid.contains(this_pid) {
                        let filter = ctx.filter_constructor().construct(FilterRequest::ByPid(this_pid));
                        self.processor_by_pid.insert(this_pid, filter);
                    };
                    let this_proc = self.processor_by_pid.get(this_pid).unwrap();
                    while ctx.filter_changeset().is_empty() {
                        this_proc.consume(ctx, &pk);
                        i += packet::Packet::SIZE;
                        let end = i+packet::Packet::SIZE;
                        if end > buf.len() {
                            break;
                        }
                        pk_buf = &buf[i..end];
                        if !packet::Packet::is_sync_byte(pk_buf[0]) {
                            // TODO: attempt to resynchronise
                            return
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
                return
            }
            i += packet::Packet::SIZE;
        }
    }
}

#[cfg(test)]
mod test {
    use data_encoding::base16;
    use bitstream_io::{BE, BitWriter};
    use std::io;

    use demultiplex;
    use psi;
    use psi::WholeSectionSyntaxPayloadParser;

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

        fn construct(&mut self, req: demultiplex::FilterRequest) -> Self::F {
            match req {
                demultiplex::FilterRequest::ByPid(0) => NullFilterSwitch::Pat(demultiplex::PatPacketFilter::default()),
                demultiplex::FilterRequest::ByPid(_) => NullFilterSwitch::Nul(demultiplex::NullPacketFilter::default()),
                demultiplex::FilterRequest::ByStream(_stype, _pmt_section, _stream_info) => NullFilterSwitch::Nul(demultiplex::NullPacketFilter::default()),
                demultiplex::FilterRequest::Pmt{pid, program_number} => NullFilterSwitch::Pmt(demultiplex::PmtPacketFilter::new(pid, program_number)),
                demultiplex::FilterRequest::Nit{..} => NullFilterSwitch::Nul(demultiplex::NullPacketFilter::default()),
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
        let section = vec!(
            // common header
            0, 0, 0,

            // table syntax header
            0x0D, 0x00, 0b00000001, 0xC1, 0x00,

            0, 1,   // program_number
            0, 101,  // pid
            0, 0, 0, 0  // CRC (incorrect!)
        );
        let header = psi::SectionCommonHeader::new(&section[..psi::SectionCommonHeader::SIZE]);
        let table_syntax_header = psi::TableSyntaxHeader::new(&section[psi::SectionCommonHeader::SIZE..]);
        let mut ctx = NullDemuxContext::new(NullStreamConstructor);
        processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        let mut changes = ctx.changeset.updates.into_iter();
        assert_matches!(changes.next(), Some(demultiplex::FilterChange::Insert(101, _)));
    }

    #[test]
    fn pat_remove_existing_program() {
        let mut ctx = NullDemuxContext::new(NullStreamConstructor);
        let mut processor = demultiplex::PatProcessor::default();
        {
            let section = vec!(
                // common header
                0, 0, 0,

                // table syntax header
                0x0D, 0x00, 0b00000001, 0xC1, 0x00,

                // PAT with a single program; next version of the  table removes this,
                0, 1,   // program_number
                0, 101, // pid

                0, 0, 0, 0,  // CRC (incorrect)
            );
            let header = psi::SectionCommonHeader::new(&section[..psi::SectionCommonHeader::SIZE]);
            let table_syntax_header = psi::TableSyntaxHeader::new(&section[psi::SectionCommonHeader::SIZE..]);
            processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        }
        ctx.changeset.updates.clear();
        {
            let section = vec!(
                // common header
                0, 0, 0,

                // table syntax header
                0x0D, 0x00, 0b00000011, 0xC1, 0x00,  // new version!

                // empty PMT - simulate removal of PID 101

                0, 0, 0, 0,  // CRC (incorrect)
            );
            let header = psi::SectionCommonHeader::new(&section[..psi::SectionCommonHeader::SIZE]);
            let table_syntax_header = psi::TableSyntaxHeader::new(&section[psi::SectionCommonHeader::SIZE..]);
            processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        }
        let mut changes = ctx.changeset.updates.into_iter();
        assert_matches!(changes.next(), Some(demultiplex::FilterChange::Remove(101,)));
    }

    fn make_test_data<F>(builder: F) -> Vec<u8>
    where
        F: Fn(BitWriter<BE>)->Result<(), io::Error>
    {
        let mut data: Vec<u8> = Vec::new();
        builder(BitWriter::<BE>::new(&mut data)).unwrap();
        data
    }

    #[test]
    fn pmt_new_stream() {
        // TODO arrange for the filter table to already contain an entry for PID 101
        let pid = 101;
        let program_number = 1001;
        let mut processor = demultiplex::PmtProcessor::new(pid, program_number);
        let section = make_test_data(|mut w| {
            // common section header,
            w.write(8, 0x02)?;   // table_id
            w.write_bit(true)?;  // section_syntax_indicator
            w.write_bit(false)?; // private_indicator
            w.write(2, 3)?;      // reserved
            w.write(12, 20)?;    // section_length

            // section syntax header,
            w.write(16, 0)?;    // id
            w.write(2, 3)?;     // reserved
            w.write(5, 0)?;     // version
            w.write(1, 1)?;     // current_next_indicator
            w.write(8, 0)?;     // section_number
            w.write(8, 0)?;     // last_section_number

            // PMT section payload
            w.write(3, 7)?;     // reserved
            w.write(13, 123)?;  // pcr_pid
            w.write(4, 15)?;    // reserved
            w.write(12, 0)?;    // program_info_length
            // program_info_length=0, so no descriptors follow; straight into stream info
            w.write(8, 0)?;     // stream_type
            w.write(3, 7)?;     // reserved
            w.write(13, 201)?;  // elementary_pid
            w.write(4, 15)?;    // reserved
            w.write(12, 6)?;    // es_info_length
            // and now, two made-up descriptors which need to fill up es_info_length-bytes
            w.write(8, 0)?;     // descriptor_tag
            w.write(8, 1)?;     // descriptor_length
            w.write(8, 0)?;     // made-up descriptor data not following any spec
            // second descriptor
            w.write(8, 0)?;     // descriptor_tag
            w.write(8, 1)?;     // descriptor_length
            w.write(8, 0)?;     // made-up descriptor data not following any spec
            w.write(32, 0)      // CRC (incorrect)
        });
        let header = psi::SectionCommonHeader::new(&section[..psi::SectionCommonHeader::SIZE]);
        let table_syntax_header = psi::TableSyntaxHeader::new(&section[psi::SectionCommonHeader::SIZE..]);
        let mut ctx = NullDemuxContext::new(NullStreamConstructor);
        processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        let mut changes = ctx.changeset.updates.into_iter();
        assert_matches!(changes.next(), Some(demultiplex::FilterChange::Insert(201,_)));
    }
}
