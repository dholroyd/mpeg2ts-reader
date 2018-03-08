use std::collections::HashMap;
use std::collections::HashSet;
use std::cell::RefCell;
use std::rc::Rc;
use packet;
use psi;
use descriptor;
use std;
use fixedbitset;
use StreamType;

// TODO: Pid = u16;

pub type PacketFilter = packet::PacketConsumer<FilterChangeset>;

pub struct NullPacketFilter { }
impl NullPacketFilter {
    pub fn construct(_pmt: &PmtSection, _stream_info: &StreamInfo) -> Box<RefCell<packet::PacketConsumer<FilterChangeset>>> {
        Box::new(RefCell::new(NullPacketFilter { }))
    }
}
impl packet::PacketConsumer<FilterChangeset> for NullPacketFilter {
    fn consume(&mut self, _pk: packet::Packet) -> Option<FilterChangeset> {
        None
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
        let pid = match *self {
            FilterChange::Insert(pid, _) | FilterChange::Remove(pid) => pid,
        };
        write!(f, "FilterChange {{ {}, ... }}", pid)
    }
}

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

    fn apply(self, filters: &mut Filters) {
        for update in self.updates {
            update.apply(filters);
        }
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
            .get(&stream_info.stream_type)
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

    fn new_table(&mut self, table: psi::Table<PmtSection>) -> Option<FilterChangeset> {
        // TODO: should probably assert that the table_id is 0x02 for PMT, but we've not managed to
        // pass the table_id value this far!
        let mut changeset = FilterChangeset::new();
        let mut pids_seen = HashSet::new();
        for sect in table.section_iter() {
            for stream_info in sect.streams() {
                println!("new PMT entry PID {} (in program_number {})", stream_info.elementary_pid, self.program_number);
                let pes_packet_consumer = self.stream_constructor.construct(&sect, &stream_info);
                changeset.insert(stream_info.elementary_pid, pes_packet_consumer);
                pids_seen.insert(stream_info.elementary_pid);
                self.filters_registered.insert(stream_info.elementary_pid as usize);
            }
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        for pid in 0..0x1fff {
            if self.filters_registered.contains(pid) && !pids_seen.contains(&(pid as u16)) {
                changeset.remove(pid as u16);
                self.filters_registered.set(pid, false);
            }
        }
        self.current_version = Some(table.ver());
        Some(changeset)
    }
}

impl psi::TableProcessor<PmtSection> for PmtProcessor {
    fn process(&mut self, table: psi::Table<PmtSection>) -> Option<FilterChangeset> {
        // don't process repetitions of the version of the table we've already seem
        // TODO: maybe move this logic into the caller
        if let Some(v) = self.current_version {
            if v != table.ver() {
                self.new_table(table)
            } else {
                None
            }
        } else {
            self.new_table(table)
        }
    }
}

#[derive(Debug)]
pub struct StreamInfo {
    pub stream_type: StreamType,    // 8 bits
    reserved1: u8,      // 3 bits
    pub elementary_pid: u16, // 13 bits
    reserved2: u8,      // 4 bits
    es_info_length: u16,// 12 bits
    pub descriptor_data: Vec<u8>,
}

impl StreamInfo {
    fn from_bytes(data: &[u8]) -> Option<(StreamInfo, usize)> {
        let header_size = 5;
        if data.len() < header_size {
            println!("only {} bytes remaining for stream info, at least {} required", data.len(), header_size);
            return None;
        }
        let mut result = StreamInfo {
            stream_type: data[0].into(),
            reserved1: data[1] >> 5,
            elementary_pid: u16::from(data[1] & 0b00011111) << 8 | u16::from(data[2]),
            reserved2: data[3] >> 4,
            es_info_length: u16::from(data[3] & 0b00001111) << 8 | u16::from(data[4]),
            descriptor_data: vec!(),
        };

        let descriptor_end = header_size + result.es_info_length as usize;
        if descriptor_end > data.len() {
            print!("PMT section of size {} is not large enough to contain es_info_length of {}", data.len(), result.es_info_length);
            return None;
        }
        result.descriptor_data.extend_from_slice(&data[header_size..descriptor_end]);
        Some((result, descriptor_end))
    }

    pub fn descriptors(&self) -> descriptor::DescriptorIter {
        descriptor::DescriptorIter::new(&self.descriptor_data[..])
    }
}

#[derive(Debug)]
pub struct PmtSection {
    reserved1: u8,              // 3 bits
    pcr_pid: u16,               // 13 bits
    reserved2: u8,              // 4 bits
    program_info_length: u16,   // 12 bits
    descriptor_data: Vec<u8>,
    stream_data: Vec<u8>,
}

impl psi::TableSection for PmtSection {
    fn from_bytes(header: &psi::SectionCommonHeader, _table_syntax_header: &psi::TableSyntaxHeader, data: &[u8]) -> Option<PmtSection> {
        let header_size = 4;
        if data.len() < header_size {
            println!("must be at least {} bytes in a PMT section: {}", header_size, data.len());
            return None;
        }
        let mut result = PmtSection {
            reserved1: data[0] >> 5,
            pcr_pid: u16::from(data[0] & 0b00011111) << 8 | u16::from(data[1]),
            reserved2: data[2] >> 4,
            program_info_length: u16::from(data[2] & 0b00001111) << 8 | u16::from(data[3]),
            descriptor_data: vec!(),
            stream_data: vec!(),
        };

        if header.private_indicator {
            println!("private PMT section - most unexpected! {:?}", header);
            return None;
        }
        let descriptor_end = header_size + result.program_info_length as usize;
        if descriptor_end > data.len() {
            print!("PMT section of size {} is not large enough to contain program_info_length of {}", data.len(), result.program_info_length);
            return None;
        }
        if result.program_info_length > 0 {
            result.descriptor_data.extend_from_slice(&data[header_size..descriptor_end]);
        }

        result.stream_data.extend_from_slice(&data[descriptor_end..]);

        Some(result)
    }
}
impl PmtSection {
    pub fn descriptors(&self) -> descriptor::DescriptorIter {
        descriptor::DescriptorIter::new(&self.descriptor_data[..])
    }
    pub fn streams(&self) -> StreamInfoIter {
        StreamInfoIter::new(&self.stream_data[..])
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
    type Item = StreamInfo;

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

    fn new_table(&mut self, table: psi::Table<PatSection>) -> Option<FilterChangeset> {
        let mut changeset = FilterChangeset::new();
        let mut pids_seen = HashSet::new();
        // add or update filters for descriptors we've not seen before,
        for sect in table.section_iter() {
            for desc in &sect.programs {
                println!("new table for pid {}, program {}", desc.pid, desc.program_number);
                let pmt_proc = PmtProcessor::new(self.stream_constructor.clone(), desc.program_number);
                let pmt_section_packet_consumer = psi::SectionPacketConsumer::new(psi::TableSectionConsumer::new(pmt_proc));
                changeset.insert(desc.pid, Box::new(RefCell::new(pmt_section_packet_consumer)));
                pids_seen.insert(desc.pid);
                self.filters_registered.insert(desc.pid as usize);
            }
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        for pid in 0..0x1fff {
            if self.filters_registered.contains(pid) && !pids_seen.contains(&(pid as u16)) {
                changeset.remove(pid as u16);
                self.filters_registered.set(pid, false);
            }
        }

        self.current_version = Some(table.ver());
        Some(changeset)
    }
}

impl psi::TableProcessor<PatSection> for PatProcessor {
    fn process(&mut self, table: psi::Table<PatSection>) -> Option<FilterChangeset> {
        // don't process repetitions of the version of the table we've already seem
        // TODO: maybe move this logic into the caller
        if let Some(v) = self.current_version {
            if v != table.ver() {
                self.new_table(table)
            } else {
                None
            }
        } else {
            self.new_table(table)
        }
    }
}

#[derive(Clone,Debug)]
struct ProgramDescriptor {
    pub program_number: u16,
    pub reserved: u8,
    pub pid: u16,
}

impl ProgramDescriptor {
    /// panics if fewer than 4 bytes are provided
    pub fn from_bytes(data: &[u8]) -> ProgramDescriptor {
        ProgramDescriptor {
            program_number: (u16::from(data[0]) << 8) | u16::from(data[1]),
            reserved: data[2] >> 5,
            pid: (u16::from(data[2]) & 0b00011111) << 8 | u16::from(data[3]),
        }
    }
}

#[derive(Clone,Debug)]
pub struct PatSection {
    programs: Vec<ProgramDescriptor>
}
impl PatSection {
    fn new(programs: Vec<ProgramDescriptor>) -> PatSection {
        PatSection {
            programs
        }
    }
}

impl psi::TableSection for PatSection {
    fn from_bytes(_header: &psi::SectionCommonHeader, _table_syntax_header: &psi::TableSyntaxHeader, data: &[u8]) -> Option<PatSection> {
        if data.len() % 4 != 0 {
            println!("section length invalid, must be multiple of 4: {} bytes", data.len());
            return None;
        }
        let descriptors = data
            .chunks(4)
            .map(ProgramDescriptor::from_bytes)
            .collect();
        Some(PatSection::new(descriptors))
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
impl packet::PacketConsumer<FilterChangeset> for UnhandledPid {
    fn consume(&mut self, pk: packet::Packet) -> Option<FilterChangeset> {
        self.seen(pk.pid());
        None
    }
}

impl Demultiplex {
    pub fn new(stream_constructor: StreamConstructor) -> Demultiplex {
        let mut result = Demultiplex {
            processor_by_pid: Filters::new(),
            default_processor: Box::new(RefCell::new(UnhandledPid::new())),
        };

        let pat_section_packet_consumer = psi::SectionPacketConsumer::new(psi::TableSectionConsumer::new(PatProcessor::new(stream_constructor)));

        result.processor_by_pid.insert(0, Box::new(RefCell::new(pat_section_packet_consumer)));

        result
    }

    pub fn push(&mut self, buf: &[u8]) {
        // TODO: simplify
        let mut i=0;
        loop {
            let end = i+packet::PACKET_SIZE;
            if end > buf.len() {
                break;
            }
            let mut pk_buf = &buf[i..end];
            if packet::Packet::is_sync_byte(pk_buf[0]) {
                let mut maybe_changeset = None;
                {
                    let mut pk = packet::Packet::new(pk_buf);
                    let this_pid = pk.pid();
                    let mut this_proc = match self.processor_by_pid.get(this_pid) {
                        &Some(ref processor) => processor.borrow_mut(),
                        &None => self.default_processor.borrow_mut(),
                    };
                    while maybe_changeset.is_none() {
                        maybe_changeset = this_proc.consume(pk);
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
                match maybe_changeset {
                    None => (),
                    Some(changeset) => changeset.apply(&mut self.processor_by_pid),
                }
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
    use psi::TableProcessor;
    use psi::TableSection;

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
    fn pat_new_program() {
        let mut processor = demultiplex::PatProcessor::new(empty_stream_constructor());
        let version = 0;

        {
            let descriptors = vec!(
                demultiplex::ProgramDescriptor::from_bytes(&[
                    0, 1,   // program_number
                    0, 100  // pid
                ])
            );
            let sections = [
                Some(demultiplex::PatSection::new(descriptors))
            ];
            let pat_table = psi::Table::new(version, &sections);

            // processing the PAT the first time should result in a FilterChange::Insert,
            let changes = processor.process(pat_table).unwrap();
            let mut i = changes.into_iter();
            assert_matches!(i.next(), Some(demultiplex::FilterChange::Insert(100, _)));
        }

        {
            let descriptors = vec!(
                demultiplex::ProgramDescriptor::from_bytes(&[
                    0, 1,   // program_number
                    0, 100  // pid
                ])
            );
            let sections = [
                Some(demultiplex::PatSection::new(descriptors))
            ];
            let pat_table = psi::Table::new(version, &sections);

            // processing PAT wih the same version a second time should mean no FilterChangeset
            let new_changes = processor.process(pat_table);
            assert!(new_changes.is_none());
        }

        {
            // New version!
            let version = 1;
            let descriptors = vec!(
                demultiplex::ProgramDescriptor::from_bytes(&[
                    0, 1,   // program_number
                    0, 100  // pid
                ])
            );
            let sections = [
                Some(demultiplex::PatSection::new(descriptors))
            ];
            let pat_table = psi::Table::new(version, &sections);

            // since the version has changed, this time the new table will not be filtered out
            let changes = processor.process(pat_table).unwrap();
            let mut i = changes.into_iter();
            assert_matches!(i.next(), Some(demultiplex::FilterChange::Insert(100, _)));
        }
    }

    #[test]
    fn pat_no_existing_program() {
        let mut processor = demultiplex::PatProcessor::new(empty_stream_constructor());
        let version = 0;
        let descriptors = vec!(
            demultiplex::ProgramDescriptor::from_bytes(&[
                0, 1,   // program_number
                0, 101  // pid
            ])
        );
        let sections = [
            Some(demultiplex::PatSection::new(descriptors))
        ];
        let pat_table = psi::Table::new(version, &sections);
        let mut changes = processor.process(pat_table).unwrap().into_iter();
        assert_matches!(changes.next(), Some(demultiplex::FilterChange::Insert(101, _)));
    }

    #[test]
    fn pat_remove_existing_program() {
        let mut processor = demultiplex::PatProcessor::new(empty_stream_constructor());
        let mut version = 0;
        {
            let descriptors = vec!(
                demultiplex::ProgramDescriptor::from_bytes(&[
                    // PAT with a single program; next version of the  table removes this,
                    0, 1,   // program_number
                    0, 101  // pid
                ])
            );
            let sections = [
                Some(demultiplex::PatSection::new(descriptors))
            ];
            let pat_table = psi::Table::new(version, &sections);
            let _changes = processor.process(pat_table).unwrap().into_iter();
        }
        version += 1;
        let mut changes = {
            let descriptors = vec!(
                // empty PMT - simulate removal of PID 101
            );
            let sections = [
                Some(demultiplex::PatSection::new(descriptors))
            ];
            let pat_table = psi::Table::new(version, &sections);
            processor.process(pat_table).unwrap().into_iter()
        };
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
        let header_data = make_test_data(|mut w| {
            w.write(8, 0)?;      // table_id
            w.write_bit(true)?;  // section_syntax_indicator
            w.write_bit(false)?; // private_indicator
            w.write(2, 3)?;      // reserved
            w.write(12, 20)      // section_length
        });
        let header = psi::SectionCommonHeader::new(&header_data[..]);
        let table_syntax_header_data = make_test_data(|mut w| {
            w.write(16, 0)?;    // id
            w.write(2, 3)?;     // reserved
            w.write(5, 0)?;     // version
            w.write(1, 1)?;     // current_next_indicator
            w.write(8, 0)?;     // section_number
            w.write(8, 0)       // last_section_number
        });
        let table_syntax_header = psi::TableSyntaxHeader::new(&table_syntax_header_data[..]);
        let section_data = make_test_data(|mut w| {
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
            // and now, two made-up descriiptors which nees to fill up es_info_length-bytes
            w.write(8, 0)?;     // descriptor_tag
            w.write(8, 1)?;     // descriptor_length
            w.write(8, 0)?;     // made-up descriptor data not following any spec
            // second descriiptor
            w.write(8, 0)?;     // descriptor_tag
            w.write(8, 1)?;     // descriptor_length
            w.write(8, 0)       // made-up descriptor data not following any spec
        });

        let sections = [
            demultiplex::PmtSection::from_bytes(&header, &table_syntax_header, &section_data[..])
        ];
        let version = 0;
        let pat_table = psi::Table::new(version, &sections);
        let mut changes = processor.process(pat_table).unwrap().into_iter();
        assert_matches!(changes.next(), Some(demultiplex::FilterChange::Insert(201,_)));
    }
}
