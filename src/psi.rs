//! Types for processing tables of *Programe Specific Information* in a transport stream.
//!
//! # Concepts
//!
//! * There are multiple standard types of Programme Specific Information, like the *Probram
//!   Association Table* and *Programe Map Table*.  Standards derived from mpegts may define their
//!   own table types.
//! * A PSI *Table* can split into *Sections*
//! * A Section can be split across a small number of individual transport stream *Packets*
//! * A Section may use a syntax common across a number of the standard table types, or may be an
//!   opaque bag of bytes within the transport stream whose intepretation is defined within a
//!   derived standard (and therefore not in this library).
//!
//! # Core types
//!
//! * [`SectionPacketConsumer`](struct.SectionPacketConsumer.html) converts *Packets* into *Sections*
//! * [`TableSectionConsumer`](struct.TableSectionConsumer.html) converts *Sections* into *Tables*
//!
//! Note that the specific types of table such as Program Association Table are defined elsewhere
//! with only the generic functionality in this module.

use std;
use packet;
use demultiplex;
use hexdump;
use mpegts_crc;


/// Trait to be implemented by types that will process sections of a Program Specific Information
/// table, provided by a `SectionPacketConsumer`.
///
/// ```rust
/// # use mpeg2ts_reader::psi::SectionPacketConsumer;
/// # use mpeg2ts_reader::psi::SectionProcessor;
/// # use mpeg2ts_reader::psi::SectionCommonHeader;
/// # use mpeg2ts_reader::demultiplex;
/// struct MyProcessor { }
/// # impl MyProcessor { pub fn new() -> MyProcessor { MyProcessor { } } }
///
/// impl SectionProcessor for MyProcessor {
///     fn process(&mut self, header: &SectionCommonHeader, section_data: &[u8]) -> Option<demultiplex::FilterChangeset> {
///         println!("Got table section with id {}", header.table_id);
///         None
///     }
/// }
///
/// let psi = SectionPacketConsumer::new(MyProcessor::new());
/// // feed some packets into psi.consume()
/// ```
///
/// This can be implemented directly to create parsers for private data that doesn't use the
/// standard section syntax.  Where the standard 'section syntax' is used, the
/// `TableSectionConsumer` implementation of this trait should be used.
pub trait SectionProcessor {
    /// Note that the first 3 bytes of `section_data` contain the header fields that have also
    /// been supplied to this call in the `header` parameter.  This is to allow implementors to
    /// calculate a CRC over the whole section if required.
    fn process(&mut self, header: &SectionCommonHeader, section_data: &[u8]) -> Option<demultiplex::FilterChangeset>;
}

#[derive(Debug,PartialEq)]
enum CurrentNext {
    Current,
    Next,
}

impl CurrentNext {
    fn from(v: u8) -> CurrentNext {
        match v {
            0 => CurrentNext::Next,
            1 => CurrentNext::Current,
            _ => panic!("invalid current_next_indicator value {}", v),
        }
    }
}

#[derive(Debug)]
pub struct TableSyntaxHeader {
    id: u16,                    // 16 bits
    reserved: u8,               // 2 bits
    version: u8,                // 5 bits
    current_next_indicator: CurrentNext, // 1 bit
    section_number: u8,         // 8 bits
    last_section_number: u8,    // bits
}

const TABLE_SYNTAX_HEADER_SIZE: usize = 5;

impl TableSyntaxHeader {
    pub fn new(data: &[u8]) -> TableSyntaxHeader {
        assert!(data.len() >= TABLE_SYNTAX_HEADER_SIZE);
        TableSyntaxHeader {
            id: u16::from(data[0]) << 8 | u16::from(data[1]),
            reserved: data[2] >> 6,
            version: (data[2] >> 1) & 0b00011111,
            current_next_indicator: CurrentNext::from(data[2] & 1),
            section_number: data[3],
            last_section_number: data[4],
        }
    }
}

#[derive(Debug)]
pub struct Table<'a, T>
where
    T: TableSection<T> + 'a,
{
    version: u8,
    sections: &'a [Option<T>],
}

pub struct TableSectionIter<'a, T: 'a> ( std::slice::Iter<'a, Option<T>> );

impl<'a, T> Iterator for TableSectionIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map( |o| o.as_ref().unwrap() )
    }
}

impl<'a, T> Table<'a, T>
where
    T: TableSection<T>
{
    pub fn new(version: u8, sections: &'a [Option<T>]) -> Table<T> {
        Table {
            version,
            sections,
        }
    }

    pub fn ver(&self) -> u8 {
        self.version
    }

    pub fn len(&'a self) -> usize {
        self.sections.len()
    }

    pub fn section_iter(&self) -> TableSectionIter<T> {
        TableSectionIter ( self.sections.iter() )
    }

    pub fn index(&'a self, index: usize) -> &'a T {
        self.sections[index].as_ref().unwrap()
    }
}


pub trait TableProcessor<T>
where
    T: TableSection<T>
{
    fn process(&mut self, table: Table<T>) -> Option<demultiplex::FilterChangeset>;
}

pub trait TableSection<T> {
    /// attempts to convert the given bytes into a table section, returning None if therw is a
    /// syntax or other error
    fn from_bytes(header: &SectionCommonHeader, table_syntax_header: &TableSyntaxHeader, data: &[u8]) -> Option<T>;  // TODO: Result instead of Option?
}

pub struct TableSectionConsumer<TP, T> {
    sections: Vec<Option<T>>,
    expected_last_section_number: Option<u8>,
    complete_section_count: u8,
    current_version: Option<u8>,
    table_processor: TP,
}

impl<TP, T> TableSectionConsumer<TP, T>
where
    TP: TableProcessor<T>,
    T: TableSection<T>
{
    pub fn new(table_processor: TP) -> TableSectionConsumer<TP, T> {
        TableSectionConsumer {
            sections: Vec::with_capacity(0xff),  // maximum number of table sections given 1 byte section_number
            expected_last_section_number: None,
            complete_section_count: 0,
            current_version: None,
            table_processor,
        }
    }

    pub fn reset(&mut self) {
        // TODO: can some of this code be shared with new() somehow?
        self.sections.clear();
        self.expected_last_section_number = None;
        self.complete_section_count = 0;
    }

    fn complete(&self) -> bool {
        self.complete_section_count == self.expected_last_section_number.unwrap() + 1
    }

    fn is_new_version(&self, table_syntax_header: &TableSyntaxHeader) -> bool {
        if let Some(ver) = self.current_version {
            ver == table_syntax_header.version
        } else {
            // there isn't yet a known version, so of course the given one is new to us
            true
        }
    }

    fn make_space_for(&mut self, table_syntax_header: &TableSyntaxHeader) {
        let required_size = table_syntax_header.last_section_number as usize + 1;
        // Vec<T>.resize() requires T:Clone, which we don't have, so go the long way around,
        while self.sections.len() < required_size {
            self.sections.push(None);
        }
    }

    fn maybe_complete_table(&mut self, version: u8) -> Option<demultiplex::FilterChangeset> {
        if self.complete() {
            let result = self.table_processor.process(Table::new(version, &self.sections[..]));
            self.reset();
            result
        } else {
            None
        }
    }

    fn insert_section(&mut self, header: &SectionCommonHeader, table_syntax_header: &TableSyntaxHeader, rest: &[u8]) -> Option<demultiplex::FilterChangeset> {
        if table_syntax_header.current_next_indicator == CurrentNext::Next {
            println!("skipping section where current_next_indicator indicates for future use, in table id {}", header.table_id);
            return None;
        }
        if self.is_new_version(table_syntax_header) {
            self.reset();
            self.current_version = Some(table_syntax_header.version);
            self.expected_last_section_number = Some(table_syntax_header.last_section_number);
            self.make_space_for(table_syntax_header);
        } else if table_syntax_header.last_section_number != self.expected_last_section_number.unwrap() {
            println!("last_section_number changed from {} to {}, but version remains {}", self.expected_last_section_number.unwrap(), table_syntax_header.last_section_number, table_syntax_header.version);
            self.reset();
            return None;
        }
        let this_section = table_syntax_header.section_number as usize;
        if let Some(s) = T::from_bytes(header, table_syntax_header, rest) {
            // track the number of complete sections so that we'll know when we have the whole
            // table,
            if self.sections[this_section].is_none() {
                self.complete_section_count += 1;
            }
            self.sections[this_section] = Some(s);
        } else {
            println!("insert_section() failed to parse {:?} {:?}", header, table_syntax_header);
        }

        self.maybe_complete_table(table_syntax_header.version)
    }
}

impl<TP, T> SectionProcessor for TableSectionConsumer<TP, T>
where
    TP: TableProcessor<T>,
    T: TableSection<T>
{
    fn process(&mut self, header: &SectionCommonHeader, payload: &[u8]) -> Option<demultiplex::FilterChangeset> {
        if !header.section_syntax_indicator {
            println!(
                "TableSectionConsumer requires that section_syntax_indicator be set in the section header"
            );
            return None;
        }
        // TODO: caller to strip-off CRC bytes?
        let crc_len = 4;
        if payload.len() < TABLE_SYNTAX_HEADER_SIZE + crc_len {
            println!("section too short {}", payload.len());
            return None;
        }
        let table_syntax_header = TableSyntaxHeader::new(payload);
        let rest = &payload[TABLE_SYNTAX_HEADER_SIZE..payload.len()-crc_len];
        self.insert_section(header, &table_syntax_header, rest)
    }
}

#[derive(Debug)]
pub struct SectionCommonHeader {
    pub table_id: u8,
    pub section_syntax_indicator: bool,
    pub private_indicator: bool,
    pub section_length: usize,
}

impl SectionCommonHeader {
    pub fn new(buf: &[u8]) -> SectionCommonHeader {
        assert_eq!(buf.len(), 3);
        SectionCommonHeader {
            table_id: buf[0],
            section_syntax_indicator: buf[1] & 0b10000000 != 0,
            private_indicator: buf[1] & 0b01000000 != 0,
            section_length: ((u16::from(buf[1] & 0b00001111) << 8) | u16::from(buf[2])) as usize,
        }
    }
}

const SECTION_LIMIT: usize = 1021;

#[derive(Eq, PartialEq, Debug)]
enum SectionParseState {
    LookingForStart,
    WaitingForEnd,
}

/// A `PacketConsumer` for buffering Programe Specific Information, which may be split across
/// multiple TS packets, and passing a complete PSI table to the given `SectionProcessor` when a
/// complete, valid section has been recieved.
pub struct SectionPacketConsumer<P>
where
    P: SectionProcessor,
{
    buf: Vec<u8>,
    parse_state: SectionParseState,
    common_header: Option<SectionCommonHeader>,
    processor: P,
}


#[cfg(not(fuzz))]
const CRC_CHECK: bool = true;
#[cfg(fuzz)]
const CRC_CHECK: bool = false;

impl<P> SectionPacketConsumer<P>
where
    P: SectionProcessor,
{
    pub fn new(processor: P) -> SectionPacketConsumer<P> {
        SectionPacketConsumer {
            buf: Vec::new(),
            parse_state: SectionParseState::LookingForStart,
            common_header: None,
            processor: processor,
        }
    }

    fn get_common_header(&self) -> &SectionCommonHeader {
        self.common_header.as_ref().unwrap()
    }

    fn expected_length(&self) -> usize {
        self.get_common_header().section_length
    }

    fn begin_new_section(&mut self, data: &[u8]) -> Option<demultiplex::FilterChangeset> {
        if self.parse_state == SectionParseState::WaitingForEnd {
            let expected = self.expected_length();
            println!(
                "previous table incomplete with {} of {} bytes when new table started",
                expected,
                self.buf.len()
            );
            self.reset();
        }
        // header, plus at least one byte of payload seems sensible,
        if data.len() < 4 {
            println!("section_length {} is too small", data.len());
            self.reset();
            return None;
        }
        let header = SectionCommonHeader::new(&data[..3]);
        if header.section_length > SECTION_LIMIT {
            println!(
                "section_length {} is too large, limit is {} bytes",
                header.section_length,
                SECTION_LIMIT
            );
            self.reset();
            return None;
        }
        self.common_header = Some(header);
        self.parse_state = SectionParseState::WaitingForEnd;
        self.append_to_current(data)
    }

    fn append_to_current(&mut self, data: &[u8]) -> Option<demultiplex::FilterChangeset> {
        if self.parse_state != SectionParseState::WaitingForEnd {
            println!("no current section, ignoring section continuation");
            return None;
        }
        let common_header_size = 3;
        let expected = self.expected_length() + common_header_size;
        if self.buf.len() + data.len() > expected {
            // if the size of the payload exceeds the section_length specified in the header, then
            // the spec says all remaining bytes within the packet payload beyond the
            // section_length should be 'stuffing' bytes with value 0xff
            let (section_data, stuffing) = data.split_at(expected - self.buf.len());
            self.check_stuffing_bytes(stuffing, "after end of PSI table");
            self.buf.extend(section_data);
        } else {
            // we have either got exactly the right number of bytes in this packet, or we are short
            // and will need to accumulate data from a further packet
            self.buf.extend(data);
        }
        if self.buf.len() == expected {
            self.finalise_current_section()
        } else {
            None
        }
    }

    fn check_stuffing_bytes(&self, stuffing: &[u8], label: &str) {
        if !stuffing.iter().all(|&b| b == 0xff) {
            println!(
                "invalid stuffing bytes {} (should all be value 0xff)",
                label
            );
            hexdump::hexdump(stuffing);
        }
    }

    fn finalise_current_section(&mut self) -> Option<demultiplex::FilterChangeset> {
        if self.get_common_header().section_syntax_indicator &&
            CRC_CHECK &&
            mpegts_crc::sum32(&self.buf[..]) != 0
        {
            println!(
                "section crc check failed for table_id {}",
                self.get_common_header().table_id
            );
            self.reset();
            return None;
        }
        let result = self.processor.process(
            self.common_header.as_ref().unwrap(),
            // skip the 3 bytes of the common header,
            &self.buf[3..],
        );
        self.reset();
        result
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.common_header = None;
        self.parse_state = SectionParseState::LookingForStart;
    }
}

impl<P> packet::PacketConsumer<demultiplex::FilterChangeset> for SectionPacketConsumer<P>
where
    P: SectionProcessor,
{
    fn consume(&mut self, pk: packet::Packet) -> Option<demultiplex::FilterChangeset> {
        match pk.payload() {
            Some(pk_buf) => {
                if pk.payload_unit_start_indicator() {
                    // this packet payload contains the start of a new PSI section
                    let pointer = pk_buf[0] as usize;
                    let section_data = &pk_buf[1..];
                    if pointer > 0 {
                        if pointer > section_data.len() {
                            println!("PSI pointer beyond end of packet payload");
                            self.reset();
                            return None;
                        }
                        let remainder = &section_data[..pointer];
                        self.append_to_current(remainder);
                        // the following call to begin_new_section() will assert that
                        // append_to_current() just finalised the preceding section
                    }
                    self.begin_new_section(&section_data[pointer..])
                } else {
                    // this packet is a continuation of an existing PSI section
                    self.append_to_current(pk_buf)
                }
            }
            None => {
                println!("no payload present in PSI packet");
                None
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::cell::RefCell;
    use data_encoding::base16;
    use psi::SectionPacketConsumer;
    use psi::TableSectionConsumer;
    use psi::SectionProcessor;
    use psi::SectionCommonHeader;
    use packet::Packet;
    use packet::PacketConsumer;
    use demultiplex;
    use demultiplex::PatProcessor;
    use demultiplex::FilterChange;

    struct NullSectionProcessor {}
    impl SectionProcessor for NullSectionProcessor {
        fn process(&mut self, _header: &SectionCommonHeader, _section_payload: &[u8]) -> Option<demultiplex::FilterChangeset> { None }
    }

    #[test]
    fn continuation_outside_section() {
        let mut buf = [0u8; 188];
        buf[0] = 0x47;
        buf[3] |= 0b00010000; // PayloadOnly
        let pk = Packet::new(&buf[..]);
        let mut psi_buf = SectionPacketConsumer::new(NullSectionProcessor {});
        psi_buf.consume(pk);
    }

    #[test]
    fn small_section() {
        let mut buf = [0u8; 188];
        buf[0] = 0x47;
        buf[1] |= 0b01000000; // payload_unit_start_indicator
        buf[3] |= 0b00010000; // PayloadOnly
        buf[7] = 3; // section_length
        let pk = Packet::new(&buf[..]);
        let mut psi_buf = SectionPacketConsumer::new(NullSectionProcessor {});
        psi_buf.consume(pk);
    }

    #[test]
    fn example() {
        let buf = base16::decode(b"474000150000B00D0001C100000001E1E02D507804FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").unwrap();
        let pk = Packet::new(&buf[..]);
        let processor_by_pid = Rc::new(RefCell::new(HashMap::new()));
        let table_sec = TableSectionConsumer::new(PatProcessor::new(processor_by_pid.clone()));
        let mut section_pk = SectionPacketConsumer::new(table_sec);
        if let Some(changeset) = section_pk.consume(pk) {
            let mut iter = changeset.into_iter();
            assert!(if let Some(FilterChange::Insert(pid, _)) = iter.next() { pid == 480 } else { false });
        } else {
            assert!(false, "consuming PAT packet should have created a new filter entry");
        }
    }
}
