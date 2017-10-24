use std::collections::HashMap;
use std::collections::HashSet;
use std::cell::RefCell;
use std::rc::Rc;
use std::ops::DerefMut;
use amphora;
use bitreader::BitReader;
use packet;
use psi;
use std;
use hexdump;
use fixedbitset;
use StreamType;

// TODO: Pid = u16;

pub type PacketFilter = packet::PacketConsumer<FilterChangeset>;

// TODO: rather than Box all filters, have an enum for internal implementations, and allow
//       extension via one of the enum variants (that presumably then carries a boxed trait)
type PidTable = Rc<RefCell<Filters>>;

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
    Update(u16, Box<RefCell<PacketFilter>>),
    Remove(u16),
}
impl FilterChange {
    fn apply(self, filters: &mut Filters) {
        match self {
            // TODO: if the update case is always going to be the same as insert, just drop the
            // Update from the enum
            FilterChange::Insert(pid, filter) | FilterChange::Update(pid, filter) => filters.insert(pid, filter),
            FilterChange::Remove(pid) => filters.remove(pid),
        };
    }
}
impl std::fmt::Debug for FilterChange {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        let pid = match *self {
            FilterChange::Insert(pid, _) | FilterChange::Update(pid, _) | FilterChange::Remove(pid) => pid,
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
    fn update(&mut self, pid: u16, filter: Box<RefCell<PacketFilter>>) {
        self.updates.push(FilterChange::Update(pid, filter))
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
    ctors_by_type: Rc<HashMap<StreamType, fn(&[Box<amphora::descriptor::Descriptor>])->Box<RefCell<PacketFilter>>>>
}
impl StreamConstructor {
    pub fn new(ctors_by_type: HashMap<StreamType, fn(&[Box<amphora::descriptor::Descriptor>])->Box<RefCell<PacketFilter>>>) -> StreamConstructor {
        StreamConstructor {
            ctors_by_type: Rc::new(ctors_by_type),
        }
    }
    fn construct(&self, stream_info: &StreamInfo) -> Option<Box<RefCell<PacketFilter>>> {
        self.ctors_by_type.get(&stream_info.stream_type).map(|ctor| ctor(&stream_info.descriptors[..]) )
    }
}

pub struct PmtProcessor {
    program_number: u16,
    pid_table: PidTable,
    current_version: Option<u8>,
    stream_constructor: StreamConstructor,
}

impl PmtProcessor {
    pub fn new(pid_table: PidTable, stream_constructor: StreamConstructor, program_number: u16) -> PmtProcessor {
        PmtProcessor {
            program_number,
            pid_table,
            current_version: None,
            stream_constructor,
        }
    }

    fn new_table(&mut self, table: psi::Table<PmtSection>) -> Option<FilterChangeset> {
        let pid_table = self.pid_table.borrow();
        let mut changeset = FilterChangeset::new();
        let mut pids_seen = HashSet::new();
        for sect in table.section_iter() {
            for stream_info in &sect.streams {
                match *pid_table.get(stream_info.elementary_pid) {
                    Some(_) => {
                        println!("updated table for pid {}, type {:?}, but pid is already being processed", stream_info.elementary_pid, stream_info.stream_type);
                        if let Some(pes_packet_consumer) = self.stream_constructor.construct(stream_info) {
                            changeset.insert(stream_info.elementary_pid, pes_packet_consumer);
                        }
                    },
                    None => {
                        println!("new PMT entry PID {} (in program_number {})", stream_info.elementary_pid, self.program_number);
                        if let Some(pes_packet_consumer) = self.stream_constructor.construct(stream_info) {
                            changeset.insert(stream_info.elementary_pid, pes_packet_consumer);
                        }
                    },
                }
                pids_seen.insert(stream_info.elementary_pid);
            }
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        for removed_pid in pid_table.pids().iter().filter(|pid| !pids_seen.contains(pid) ) {
            changeset.remove(*removed_pid);
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
    stream_type: StreamType,    // 8 bits
    reserved1: u8,      // 3 bits
    elementary_pid: u16, // 13 bits
    reserved2: u8,      // 4 bits
    es_info_length: u16,// 12 bits
    descriptors: Vec<Box<amphora::descriptor::Descriptor>>,
}

use amphora::base::Deserialize;
fn parse_descriptor_list(descriptors: &mut Vec<Box<amphora::descriptor::Descriptor>>, descriptor_data: &[u8]) -> Option<()> {
    let mut remaining = &descriptor_data[..];
    let mut count = 0;
    while remaining.len() > 0 {
        if remaining.len() < 2 {
            println!("not enough data left for a descriptor");
            return None;
        }
        let mut reader = BitReader::new(remaining);
        match amphora::descriptor::deserialize_descriptor(&mut reader) {
            Ok(desc) => {
                descriptors.push(desc);
                remaining = &remaining[(reader.position()/8) as usize..];
            },
            Err(err) => {
                println!("problem deserialising descriptor {}: {:?}", count, err);
                match amphora::descriptor::basic::UnknownDescriptor::from_bytes(remaining) {
                    Ok(desc) => {
                        hexdump::hexdump(&remaining[..desc.descriptor_length as usize]);
                        remaining = &remaining[desc.descriptor_length as usize..];
                        descriptors.push(Box::new(desc));
                    },
                    Err(_) => {
                        hexdump::hexdump(remaining);
                        return None;
                    }
                }
            },
        }
        count += 1;
    }
    Some(())
}

impl StreamInfo {
    fn from_bytes(data: &[u8]) -> Option<(StreamInfo, usize)> {
        let header_size = 5;
        if data.len() < header_size {
            println!("only {} bytes remaining for stream info, lat least {} required", data.len(), header_size);
            return None;
        }
        let mut result = StreamInfo {
            stream_type: data[0].into(),
            reserved1: data[1] >> 5,
            elementary_pid: u16::from(data[1] & 0b00011111) << 8 | u16::from(data[2]),
            reserved2: data[3] >> 4,
            es_info_length: u16::from(data[3] & 0b00001111) << 8 | u16::from(data[4]),
            descriptors: vec!(),
        };

        let descriptor_end = header_size + result.es_info_length as usize;
        if descriptor_end > data.len() {
            print!("PMT section of size {} is not large enough to contain es_info_length of {}", data.len(), result.es_info_length);
            return None;
        }
        let descriptor_data = &data[header_size..descriptor_end];
        if parse_descriptor_list(&mut result.descriptors, descriptor_data).is_none() {
            return None;
        }
        Some((result, descriptor_end))
    }
}

#[derive(Debug)]
pub struct PmtSection {
    reserved1: u8,              // 3 bits
    pcr_pid: u16,               // 13 bits
    reserved2: u8,              // 4 bits
    program_info_length: u16,   // 12 bits
    descriptors: Vec<Box<amphora::descriptor::Descriptor>>,
    streams: Vec<StreamInfo>,
}

impl psi::TableSection<PmtSection> for PmtSection {
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
            descriptors: vec!(),
            streams: vec!(),
        };

        if header.private_indicator {
            println!("private PMT section - most unexpected! {:?}", header);
        }
        let descriptor_end = header_size + result.program_info_length as usize;
        if descriptor_end > data.len() {
            print!("PMT section of size {} is not large enough to contain program_info_length of {}", data.len(), result.program_info_length);
            return None;
        }
        if result.program_info_length > 0 {
            let descriptor_data = &data[header_size..descriptor_end];
            if parse_descriptor_list(&mut result.descriptors, descriptor_data).is_none() {
                return None;
            }
        }

        let mut pos = descriptor_end;
        while pos < data.len() {
            let stream_data = &data[pos..];
            if let Some((stream_info, info_len)) = StreamInfo::from_bytes(stream_data) {
                result.streams.push(stream_info);
                pos += info_len;
            } else  {
                return None;
            }
        }

        Some(result)
    }
}

// ---- PAT ----

pub struct PatProcessor {
    pid_table: PidTable,
    current_version: Option<u8>,
    stream_constructor: StreamConstructor,
}

impl PatProcessor {
    pub fn new(pid_table: PidTable, stream_constructor: StreamConstructor) -> PatProcessor {
        PatProcessor {
            pid_table,
            current_version: None,
            stream_constructor,
        }
    }

    fn new_table(&mut self, table: psi::Table<PatSection>) -> Option<FilterChangeset> {
        let pid_table = self.pid_table.borrow();
        let mut changeset = FilterChangeset::new();
        let mut pids_seen = HashSet::new();
        // add or update filters for descriptors we've not seen before,
        for sect in table.section_iter() {
            for desc in &sect.programs {
                match *pid_table.get(desc.pid) {
                    Some(_) => {
                        println!("updated table for pid {}, program {}, but pid is already being processed", desc.pid, desc.program_number);
                        let pmt_proc = PmtProcessor::new(Rc::clone(&self.pid_table), self.stream_constructor.clone(), desc.program_number);
                        let pmt_section_packet_consumer = psi::SectionPacketConsumer::new(psi::TableSectionConsumer::new(pmt_proc));
                        changeset.update(desc.pid, Box::new(RefCell::new(pmt_section_packet_consumer)));
                    },
                    None => {
                        println!("new table for pid {}, program {}", desc.pid, desc.program_number);
                        let pmt_proc = PmtProcessor::new(Rc::clone(&self.pid_table), self.stream_constructor.clone(), desc.program_number);
                        let pmt_section_packet_consumer = psi::SectionPacketConsumer::new(psi::TableSectionConsumer::new(pmt_proc));
                        changeset.insert(desc.pid, Box::new(RefCell::new(pmt_section_packet_consumer)));
                    },
                }
                pids_seen.insert(desc.pid);
            }
        }
        // remove filters for descriptors we've seen before that are not present in this updated
        // table,
        for removed_pid in pid_table.pids().iter().filter(|pid| !pids_seen.contains(pid) ) {
            changeset.remove(*removed_pid);
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

impl psi::TableSection<PatSection> for PatSection {
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
    processor_by_pid: PidTable,
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
        let result = Demultiplex {
            processor_by_pid: Rc::new(RefCell::new(Filters::new())),
            default_processor: Box::new(RefCell::new(UnhandledPid::new())),
        };

        let map_ref = result.processor_by_pid.clone();
        let pat_section_packet_consumer = psi::SectionPacketConsumer::new(psi::TableSectionConsumer::new(PatProcessor::new(map_ref, stream_constructor)));

        result.processor_by_pid.borrow_mut().insert(0, Box::new(RefCell::new(pat_section_packet_consumer)));

        result
    }
}

impl packet::PacketConsumer<()> for Demultiplex {
    fn consume(&mut self, pk: packet::Packet) -> Option<()> {
        let maybe_changeset = match self.processor_by_pid.borrow().get(pk.pid()) {
            &Some(ref processor) => processor.borrow_mut().consume(pk),
            &None => self.default_processor.borrow_mut().consume(pk),
        };
        match maybe_changeset {
            None => (),
            Some(changeset) => changeset.apply(self.processor_by_pid.borrow_mut().deref_mut()),
        }
        None
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::cell::RefCell;
    use std::rc::Rc;
    use data_encoding::base16;
    use bitstream_io::{BE, BitWriter};
    use std::io;

    use demultiplex;
    use packet;
    use packet::PacketConsumer;
    use psi;
    use psi::TableProcessor;
    use psi::TableSection;

    fn empty_stream_constructor() -> demultiplex::StreamConstructor {
        demultiplex::StreamConstructor::new(HashMap::new())
    }

    #[test]
    fn pat() {
        // TODO: better
        let buf = base16::decode(b"474000150000B00D0001C100000001E1E02D507804FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").unwrap();
        let pk = packet::Packet::new(&buf[..]);
        let mut deplex = demultiplex::Demultiplex::new(empty_stream_constructor());
        deplex.consume(pk);
    }

    #[test]
    fn pat_new_program() {
        let pid_table = Rc::new(RefCell::new(demultiplex::Filters::new()));
        let mut processor = demultiplex::PatProcessor::new(pid_table, empty_stream_constructor());
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
            // Note that since this test is not using a Demultiplex instance, nothing updates
            // pid_table with the changes produced earlier in the test, and so an Insert is
            // produced rather than an Update (as would be the case in normal use),
            assert_matches!(i.next(), Some(demultiplex::FilterChange::Insert(100, _)));
        }
    }

    #[derive(Default)]
    struct NullConsumer { }
    impl packet::PacketConsumer<demultiplex::FilterChangeset> for NullConsumer {
        fn consume(&mut self, _pk: packet::Packet) -> Option<demultiplex::FilterChangeset> {
            None
        }
    }

    fn null_proc() -> Box<RefCell<packet::PacketConsumer<demultiplex::FilterChangeset>>>{
        Box::new(RefCell::new(NullConsumer::default()))
    }

    #[test]
    fn pat_no_existing_program() {
        let pid_table = Rc::new(RefCell::new(demultiplex::Filters::new()));
        // arrange for the filter table to already contain an entry for PID 101
        pid_table.borrow_mut().insert(101u16, null_proc());
        let mut processor = demultiplex::PatProcessor::new(pid_table, empty_stream_constructor());
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
        assert_matches!(changes.next(), Some(demultiplex::FilterChange::Update(101, _)));
    }

    #[test]
    fn pat_remove_existing_program() {
        let pid_table = Rc::new(RefCell::new(demultiplex::Filters::new()));
        // arrange for the filter table to already contain an entry for PID 101
        pid_table.borrow_mut().insert(101u16, null_proc());
        let mut processor = demultiplex::PatProcessor::new(pid_table, empty_stream_constructor());
        let version = 0;
        let descriptors = vec!(
            // empty PMT - simulate removal of PID 101
        );
        let sections = [
            Some(demultiplex::PatSection::new(descriptors))
        ];
        let pat_table = psi::Table::new(version, &sections);
        let mut changes = processor.process(pat_table).unwrap().into_iter();
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
        let pid_table = Rc::new(RefCell::new(demultiplex::Filters::new()));
        // arrange for the filter table to already contain an entry for PID 101
        pid_table.borrow_mut().insert(101u16, null_proc());
        let program_number = 1001;
        let mut processor = demultiplex::PmtProcessor::new(pid_table, empty_stream_constructor(), program_number);
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
        let changes = processor.process(pat_table).unwrap().into_iter();
        //assert_matches!(changes.next(), Some(demultiplex::FilterChange::Insert(201,_)));
    }
}
