use std::collections::HashMap;
use std::collections::HashSet;
use std::cell::RefCell;
use std::rc::Rc;
use std::fmt;
use packet;
use psi;
use descriptor;
use std;
use fixedbitset;
use StreamType;

// TODO: Pid = u16;

pub trait PacketFilter {
    fn consume(&mut self, ctx: &mut DemuxContext, pk: packet::Packet);
}

pub struct NullPacketFilter { }
impl NullPacketFilter {
    pub fn construct(_pmt: &PmtSection, _stream_info: &StreamInfo) -> Box<RefCell<PacketFilter>> {
        Box::new(RefCell::new(NullPacketFilter { }))
    }
}
impl PacketFilter for NullPacketFilter {
    fn consume(&mut self, _ctx: &mut DemuxContext, _pk: packet::Packet) {
    }
}

// TODO: rather than Box all filters, have an enum for internal implementations, and allow
//       extension via one of the enum variants (that presumably then carries a boxed trait)
pub struct Filters {
    filters_by_pid: Vec<Option<Box<RefCell<PacketFilter>>>>
}
impl Filters {
    pub fn new() -> Filters {
        Filters {
            filters_by_pid: vec!(),
        }
    }

    pub fn get(&self, pid: u16) -> &Option<Box<RefCell<PacketFilter>>> {
        if pid as usize >= self.filters_by_pid.len() {
            &None
        } else {
            &self.filters_by_pid[pid as usize]
        }
    }

    pub fn insert(&mut self, pid: u16, filter: Box<RefCell<PacketFilter>>) {
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

pub enum FilterChange {
    Insert(u16, Box<RefCell<PacketFilter>>),
    Remove(u16),
}
impl FilterChange {
    fn apply(self, filters: &mut Filters) {
        match self {
            FilterChange::Insert(pid, filter) => filters.insert(pid, filter),
            FilterChange::Remove(pid) => filters.remove(pid),
        };
    }
}
impl std::fmt::Debug for FilterChange {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        match *self {
            FilterChange::Insert(pid, _) => write!(f, "FilterChange::Insert {{ {}, ... }}", pid),
            FilterChange::Remove(pid) => write!(f, "FilterChange::Remove {{ {}, ... }}", pid),
        }
    }
}

#[derive(Debug)]
pub struct FilterChangeset {
    updates: Vec<FilterChange>
}
impl FilterChangeset {
    fn new() -> FilterChangeset {
        FilterChangeset { updates: Vec::new() }
    }
    fn insert(&mut self, pid: u16, filter: Box<RefCell<PacketFilter>>) {
        self.updates.push(FilterChange::Insert(pid, filter))
    }
    fn remove(&mut self, pid: u16) {
        self.updates.push(FilterChange::Remove(pid))
    }

    fn apply(&mut self, filters: &mut Filters) {
        for update in self.updates.drain(..) {
            update.apply(filters);
        }
    }
    fn is_empty(&self) -> bool {
        self.updates.is_empty()
    }
}

impl std::iter::IntoIterator for FilterChangeset {
    type Item = FilterChange;
    type IntoIter = std::vec::IntoIter<FilterChange>;

    fn into_iter(self) -> std::vec::IntoIter<FilterChange> {
        self.updates.into_iter()
    }
}

// ---- PMT ----

/// Holds constructors for objects that will handle different types of Elementary Stream, keyed by
/// the `stream_type` value which will appear in the Program Mapping Table of a Transport Stream.
#[derive(Clone)]
pub struct StreamConstructor {
    default_ctor: fn(&PmtSection,&StreamInfo)->Box<RefCell<PacketFilter>>,
    ctors_by_type: Rc<HashMap<StreamType, fn(&PmtSection,&StreamInfo)->Box<RefCell<PacketFilter>>>>

}
impl StreamConstructor {
    pub fn new(default_ctor: fn(&PmtSection,&StreamInfo)->Box<RefCell<PacketFilter>>, ctors_by_type: HashMap<StreamType, fn(&PmtSection,&StreamInfo)->Box<RefCell<PacketFilter>>>) -> StreamConstructor {
        StreamConstructor {
            default_ctor,
            ctors_by_type: Rc::new(ctors_by_type),
        }
    }
    fn construct(&self, sect: &PmtSection, stream_info: &StreamInfo) -> Box<RefCell<PacketFilter>> {
        self
            .ctors_by_type
            .get(&stream_info.stream_type())
            .map(|ctor| ctor(sect, &stream_info) )
            .unwrap_or_else(|| (self.default_ctor)(sect, &stream_info) )
    }
}

pub struct PmtProcessor {
    program_number: u16,
    current_version: Option<u8>,
    stream_constructor: StreamConstructor,
    filters_registered: fixedbitset::FixedBitSet,
}

impl PmtProcessor {
    pub fn new(stream_constructor: StreamConstructor, program_number: u16) -> PmtProcessor {
        PmtProcessor {
            program_number,
            current_version: None,
            stream_constructor,
            filters_registered: fixedbitset::FixedBitSet::with_capacity(0x2000),
        }
    }

    fn new_table(&mut self, ctx: &mut DemuxContext, header: &psi::SectionCommonHeader, table_syntax_header: &psi::TableSyntaxHeader, sect: &PmtSection) {
        if 0x02 != header.table_id {
            println!("Expected PMT to have table id 0x2, but got {:#x}", header.table_id);
            return;
        }
        // pass the table_id value this far!
        let mut pids_seen = HashSet::new();
        for stream_info in sect.streams() {
            println!("new PMT entry PID {} (in program_number {})", stream_info.elementary_pid(), self.program_number);
            let pes_packet_consumer = self.stream_constructor.construct(&sect, &stream_info);
            ctx.changeset.insert(stream_info.elementary_pid(), pes_packet_consumer);
            pids_seen.insert(stream_info.elementary_pid());
            self.filters_registered.insert(stream_info.elementary_pid() as usize);
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        for pid in 0..0x1fff {
            if self.filters_registered.contains(pid) && !pids_seen.contains(&(pid as u16)) {
                ctx.changeset.remove(pid as u16);
                self.filters_registered.set(pid, false);
            }
        }
        self.current_version = Some(table_syntax_header.version());
    }
}

impl psi::WholeSectionSyntaxPayloadParser for PmtProcessor {
    type Context = DemuxContext;

    fn section<'a>(&mut self, ctx: &mut Self::Context, header: &psi::SectionCommonHeader, table_syntax_header: &psi::TableSyntaxHeader, data: &'a [u8]) {
        let start = psi::SectionCommonHeader::SIZE+psi::TableSyntaxHeader::SIZE;
        let end = data.len() - 4;  // remove CRC bytes
        self.new_table(ctx, header, table_syntax_header, &PmtSection::new(&data[start..end]));
    }
}

pub struct StreamInfo<'buf> {
    data: &'buf[u8],
}

impl<'buf> StreamInfo<'buf> {
    const HEADER_SIZE: usize = 5;

    fn from_bytes(data: &'buf[u8]) -> Option<(StreamInfo<'buf>, usize)> {
        if data.len() < Self::HEADER_SIZE {
            println!("only {} bytes remaining for stream info, at least {} required {:?}", data.len(), Self::HEADER_SIZE, data);
            return None;
        }
        let result = StreamInfo {
            data,
        };

        let descriptor_end = Self::HEADER_SIZE + result.es_info_length() as usize;
        if descriptor_end > data.len() {
            print!("PMT section of size {} is not large enough to contain es_info_length of {}", data.len(), result.es_info_length());
            return None;
        }
        Some((result, descriptor_end))
    }

    pub fn stream_type(&self) -> StreamType {
        self.data[0].into()
    }
    pub fn reserved1(&self) -> u8 {
        self.data[1] >> 5
    }
    pub fn elementary_pid(&self) -> u16 {
       u16::from(self.data[1] & 0b00011111) << 8 | u16::from(self.data[2])
    }
    pub fn reserved2(&self) -> u8 {
        self.data[3] >> 4
    }
    pub fn es_info_length(&self) -> u16 {
        u16::from(self.data[3] & 0b00001111) << 8 | u16::from(self.data[4])
    }

    pub fn descriptors(&self) -> descriptor::DescriptorIter {
        let descriptor_end = Self::HEADER_SIZE + self.es_info_length() as usize;
        descriptor::DescriptorIter::new(&self.data[Self::HEADER_SIZE..descriptor_end])
    }
}
impl<'buf> fmt::Debug for StreamInfo<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.debug_struct("StreamInfo")
            .field("stream_type", &self.stream_type())
            .field("elementry_pid", &self.elementary_pid())
            .field("es_info_length", &self.es_info_length())
            .finish()
    }
}

#[derive(Debug)]
pub struct PmtSection<'buf> {
    data: &'buf[u8],
}

impl<'buf> PmtSection<'buf> {
    fn new(data: &'buf[u8]) -> PmtSection<'buf> {
        PmtSection {
            data,
        }
    }
}

impl<'buf> PmtSection<'buf> {
    const HEADER_SIZE: usize = 4;

    pub fn reserved1(&self) -> u8 {
        self.data[0] >> 5
    }
    pub fn pcr_pid(&self) -> u16 {
        u16::from(self.data[0] & 0b00011111) << 8 | u16::from(self.data[1])
    }
    pub fn reserved2(&self) -> u8 {
        self.data[2] >> 4
    }
    pub fn program_info_length(&self) -> u16 {
        u16::from(self.data[2] & 0b00001111) << 8 | u16::from(self.data[3])
    }
    pub fn descriptors(&self) -> descriptor::DescriptorIter {
        let descriptor_end = Self::HEADER_SIZE + self.program_info_length() as usize;
        let descriptor_data = &self.data[Self::HEADER_SIZE..descriptor_end];
        descriptor::DescriptorIter::new(descriptor_data)
    }
    pub fn streams(&self) -> StreamInfoIter {
        let descriptor_end = Self::HEADER_SIZE + self.program_info_length() as usize;
        if descriptor_end > self.data.len() {
            panic!("program_info_length={} extends beyond end of PMT section (section_length={})", self.program_info_length(), self.data.len());
        }
        StreamInfoIter::new(&self.data[descriptor_end..])
    }
}
pub struct StreamInfoIter<'buf> {
    buf: &'buf[u8],
}
impl<'buf> StreamInfoIter<'buf> {
   fn new(buf: &'buf[u8]) -> StreamInfoIter<'buf> {
       StreamInfoIter { buf }
   }
}
impl<'buf> Iterator for StreamInfoIter<'buf> {
    type Item = StreamInfo<'buf>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }
        if let Some((stream_info, info_len)) = StreamInfo::from_bytes(self.buf) {
            self.buf = &self.buf[info_len..];
            Some(stream_info)
        } else {
            None
        }
    }
}

// ---- PAT ----

pub struct PatProcessor {
    current_version: Option<u8>,
    stream_constructor: StreamConstructor,
    filters_registered: fixedbitset::FixedBitSet,
}

impl PatProcessor {
    pub fn new(stream_constructor: StreamConstructor) -> PatProcessor {
        PatProcessor {
            current_version: None,
            stream_constructor,
            filters_registered: fixedbitset::FixedBitSet::with_capacity(0x2000),
        }
    }

    fn new_table(&mut self, ctx: &mut DemuxContext, header: &psi::SectionCommonHeader, table_syntax_header: &psi::TableSyntaxHeader, sect: &PatSection) {
        if 0x00 != header.table_id {
            println!("Expected PAT to have table id 0x0, but got {:#x}", header.table_id);
            return;
        }
        let mut pids_seen = HashSet::new();
        // add or update filters for descriptors we've not seen before,
        for desc in sect.programs() {
            println!("new table for pid {}, program {}", desc.pid(), desc.program_number());
            let pmt_proc = PmtProcessor::new(self.stream_constructor.clone(), desc.program_number());
            let pmt_section_packet_consumer = psi::SectionPacketConsumer::new(
                psi::SectionSyntaxSectionProcessor::new(
                    psi::DedupSectionSyntaxPayloadParser::new(
                        psi::BufferSectionSyntaxParser::new(
                            psi::CrcCheckWholeSectionSyntaxPayloadParser::new(
                                pmt_proc
                            )
                        )
                    )
                )
            );
            ctx.changeset.insert(desc.pid(), Box::new(RefCell::new(pmt_section_packet_consumer)));
            pids_seen.insert(desc.pid());
            self.filters_registered.insert(desc.pid() as usize);
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        for pid in 0..0x1fff {
            if self.filters_registered.contains(pid) && !pids_seen.contains(&(pid as u16)) {
                ctx.changeset.remove(pid as u16);
                self.filters_registered.set(pid, false);
            }
        }

        self.current_version = Some(table_syntax_header.version());
    }
}


impl psi::WholeSectionSyntaxPayloadParser for PatProcessor {
    type Context = DemuxContext;

    fn section<'a>(&mut self, ctx: &mut Self::Context, header: &psi::SectionCommonHeader, table_syntax_header: &psi::TableSyntaxHeader, data: &'a [u8]) {
        let start = psi::SectionCommonHeader::SIZE+psi::TableSyntaxHeader::SIZE;
        let end = data.len() - 4;  // remove CRC bytes
        self.new_table(ctx, header, table_syntax_header, &PatSection::new(&data[start..end]));
    }
}

#[derive(Clone,Debug)]
struct ProgramDescriptor<'buf> {
    data: &'buf[u8],
}

impl<'buf> ProgramDescriptor<'buf> {
    /// panics if fewer than 4 bytes are provided
    pub fn from_bytes(data: &'buf[u8]) -> ProgramDescriptor<'buf> {
        ProgramDescriptor {
            data,
        }
    }

    pub fn program_number(&self) -> u16 {
        (u16::from(self.data[0]) << 8) | u16::from(self.data[1])

    }

    pub fn pid(&self) -> u16 {
        (u16::from(self.data[2]) & 0b00011111) << 8 | u16::from(self.data[3])
    }
}

#[derive(Clone,Debug)]
pub struct PatSection<'buf> {
    data: &'buf[u8],
}
impl<'buf> PatSection<'buf> {
    fn new(data: &'buf[u8]) -> PatSection<'buf> {
        PatSection {
            data,
        }
    }
    fn programs(&self) -> ProgramIter {
        ProgramIter { buf: &self.data[..] }
    }
}
struct ProgramIter<'buf> {
    buf: &'buf[u8],
}
impl<'buf> Iterator for ProgramIter<'buf> {
    type Item = ProgramDescriptor<'buf>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }
        let (head, tail) = self.buf.split_at(4);
        self.buf = tail;
        Some(ProgramDescriptor::from_bytes(head))
    }
}

// ---- demux ----

/// PAT / PMT processing
pub struct Demultiplex {
    processor_by_pid: Filters,
    default_processor: Box<RefCell<PacketFilter>>,
}

struct UnhandledPid {
    pids_seen: fixedbitset::FixedBitSet,
}
impl UnhandledPid {
    fn new() -> UnhandledPid {
        UnhandledPid { pids_seen: fixedbitset::FixedBitSet::with_capacity(0x2000) }
    }
    fn seen(&mut self, pid: u16) {
        if !self.pids_seen[pid as usize] {
            println!("unhandled pid {}", pid);
            self.pids_seen.put(pid as usize);
        }
    }
}
impl PacketFilter for UnhandledPid {
    fn consume(&mut self, _ctx: &mut DemuxContext, pk: packet::Packet) {
        self.seen(pk.pid());
    }
}

pub struct DemuxContext {
    changeset: FilterChangeset,
}
impl DemuxContext {
    pub fn new() -> DemuxContext {
        DemuxContext {
            changeset: FilterChangeset::new(),
        }
    }
}

impl Demultiplex {
    pub fn new(stream_constructor: StreamConstructor) -> Demultiplex {
        let mut result = Demultiplex {
            processor_by_pid: Filters::new(),
            default_processor: Box::new(RefCell::new(UnhandledPid::new())),
        };

        let pat_proc = PatProcessor::new(stream_constructor);
        let pat_section_packet_consumer = psi::SectionPacketConsumer::new(
            psi::SectionSyntaxSectionProcessor::new(
                psi::DedupSectionSyntaxPayloadParser::new(
                    psi::BufferSectionSyntaxParser::new(
                        psi::CrcCheckWholeSectionSyntaxPayloadParser::new(pat_proc)
                    )
                )
            )
        );

        result.processor_by_pid.insert(0, Box::new(RefCell::new(pat_section_packet_consumer)));

        result
    }

    pub fn push(&mut self, buf: &[u8]) {
        // TODO: simplify
        let mut i=0;
        let mut ctx = DemuxContext::new();
        loop {
            let end = i+packet::PACKET_SIZE;
            if end > buf.len() {
                break;
            }
            let mut pk_buf = &buf[i..end];
            if packet::Packet::is_sync_byte(pk_buf[0]) {
                {
                    let mut pk = packet::Packet::new(pk_buf);
                    let this_pid = pk.pid();
                    let mut this_proc = match self.processor_by_pid.get(this_pid) {
                        &Some(ref processor) => processor.borrow_mut(),
                        &None => self.default_processor.borrow_mut(),
                    };
                    while ctx.changeset.is_empty() {
                        this_proc.consume(&mut ctx, pk);
                        i += packet::PACKET_SIZE;
                        let end = i+packet::PACKET_SIZE;
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
                            i -= packet::PACKET_SIZE;
                            break;
                        }
                    }
                }
                if !ctx.changeset.is_empty() {
                    ctx.changeset.apply(&mut self.processor_by_pid);
                }
                debug_assert!(ctx.changeset.is_empty());
            } else {
                // TODO: attempt to resynchronise
                return
            }
            i += packet::PACKET_SIZE;
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use data_encoding::base16;
    use bitstream_io::{BE, BitWriter};
    use std::io;

    use demultiplex;
    use psi;
    use psi::WholeSectionSyntaxPayloadParser;

    fn empty_stream_constructor() -> demultiplex::StreamConstructor {
        demultiplex::StreamConstructor::new(demultiplex::NullPacketFilter::construct, HashMap::new())
    }

    #[test]
    fn demux_empty() {
        let mut deplex = demultiplex::Demultiplex::new(empty_stream_constructor());
        deplex.push(&[0x0; 0][..]);
    }

    #[test]
    fn pat() {
        // TODO: better
        let buf = base16::decode(b"474000150000B00D0001C100000001E1E02D507804FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").unwrap();
        let mut deplex = demultiplex::Demultiplex::new(empty_stream_constructor());
        deplex.push(&buf[..]);
    }

    #[test]
    fn pat_no_existing_program() {
        let mut processor = demultiplex::PatProcessor::new(empty_stream_constructor());
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
        let mut ctx = demultiplex::DemuxContext::new();
        processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        let mut changes = ctx.changeset.updates.into_iter();
        assert_matches!(changes.next(), Some(demultiplex::FilterChange::Insert(101, _)));
    }

    #[test]
    fn pat_remove_existing_program() {
        let mut ctx = demultiplex::DemuxContext::new();
        let mut processor = demultiplex::PatProcessor::new(empty_stream_constructor());
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
        let program_number = 1001;
        let mut processor = demultiplex::PmtProcessor::new(empty_stream_constructor(), program_number);
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
        let mut ctx = demultiplex::DemuxContext::new();
        processor.section(&mut ctx, &header, &table_syntax_header, &section[..]);
        let mut changes = ctx.changeset.updates.into_iter();
        assert_matches!(changes.next(), Some(demultiplex::FilterChange::Insert(201,_)));
    }
}
