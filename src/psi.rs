//! Types for processing tables of *Program Specific Information* in a transport stream.
//!
//! # Concepts
//!
//! * There are multiple standard types of Program Specific Information, like the *Program
//!   Association Table* and *Program Map Table*.  Standards derived from mpegts may define their
//!   own table types.
//! * A PSI *Table* can split into *Sections*
//! * A Section can be split across a small number of individual transport stream *Packets*
//! * A Section may use a syntax common across a number of the standard table types, or may be an
//!   opaque bag of bytes within the transport stream whose interpretation is defined within a
//!   derived standard (and therefore not in this library).
//!
//! # Core types
//!
//! * [`SectionPacketConsumer`](struct.SectionPacketConsumer.html) converts *Packets* into *Sections*
//! * [`TableSectionConsumer`](struct.TableSectionConsumer.html) converts *Sections* into *Tables*
//!
//! Note that the specific types of table such as Program Association Table are defined elsewhere
//! with only the generic functionality in this module.

use packet;
use demultiplex;
use hexdump;
use mpegts_crc;


/// Trait for types which process the data within a PSI section following the 12-byte
/// `section_length` field (which is one of the items available in the `SectionCommonHeader` that
/// is passed in.
///
///  - For PSI tables that use 'section syntax', the existing
///    [`TableSectionConsumer`](struct.TableSectionConsumer.html) implementation of this trait
///    can be used.
///  - This trait should be implemented directly for PSI tables that use 'compact' syntax (i.e.
///    they lack the 5-bytes-worth of fields represented by [`TableSyntaxHeader`](struct.TableSyntaxHeader.html))
pub trait SectionProcessor<T> {
    /// Note that the first 3 bytes of `section_data` contain the header fields that have also
    /// been supplied to this call in the `header` parameter.  This is to allow implementers to
    /// calculate a CRC over the whole section if required.
    fn process<'a>(&mut self, header: &SectionCommonHeader, section_data: &'a[u8]) -> Option<(TableSyntaxHeader<'a>, T)>;
}

#[derive(Debug,PartialEq)]
pub enum CurrentNext {
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
/// Represents the fields that appear within table sections that use the common 'section syntax'.
///
/// This will only be used for a table section if the
/// [`section_syntax_indicator`](struct.SectionCommonHeader.html#structfield.section_syntax_indicator)
/// field in the `SectionCommonHeader` of the section is `true`.
pub struct TableSyntaxHeader<'buf> {
    buf: &'buf[u8],
}

const TABLE_SYNTAX_HEADER_SIZE: usize = 5;

impl<'buf> TableSyntaxHeader<'buf> {
    pub fn new(buf: &'buf[u8]) -> TableSyntaxHeader {
        assert!(buf.len() >= TABLE_SYNTAX_HEADER_SIZE);
        TableSyntaxHeader {
            buf
        }
    }
    /// The initial 16-bit field within a 'section syntax' PSI table (which immediately follows the
    /// `section_length` field).
    /// _13818-1_ refers to this field as,
    ///  - `transport_stream_id` when it appears within a Program Association Section
    ///  - part of the `reserved` field when it appears within a Conditional Access Section
    ///  - `program_number` when it appears within a Program Map Section
    ///  - `table_id_extension` when it appears within a Private Section
    pub fn id(&self) -> u16 {
        u16::from(self.buf[0]) << 8 | u16::from(self.buf[1])
    }
    /// A 5-bit value that can be used to quickly check if this table has changed since the last
    /// time it was periodically inserted within the transport stream being read.
    pub fn version(&self) -> u8 {
        (self.buf[2] >> 1) & 0b00011111
    }
    /// Is this table applicable now, or will it become applicable at some future time.
    /// NB I've not seen sample data that uses anything other than `CurrentNext::Current`, so
    /// handling of tables with 'future' applicability may not actually work properly.
    pub fn current_next_indicator(&self) -> CurrentNext {
        CurrentNext::from(self.buf[2] & 1)
    }
    /// The number of this section, within a potentially multi-section table.
    ///
    /// It is common for only one section to appear within any PSI table, in which case this value
    /// will always be `0` within a given stream.  The value of `last_section_number()` can be
    /// used to tell if multiple sections are expected.
    pub fn section_number(&self) -> u8 {
        self.buf[3]
    }
    /// Indicates the value of `section_number()` that will appear within the last section within
    /// a table.  In many streams, this value is always `0`, however multiple table sections may
    /// need be used if the table needs to carry a large number of entries.
    pub fn last_section_number(&self) -> u8 {
        self.buf[4]
    }
}

pub trait TableProcessor<T>
where
    T: TableSection
{
    fn process(&mut self, table_syntax_header: &TableSyntaxHeader, sect: &T) -> Option<demultiplex::FilterChangeset>;
}

pub trait TableSection: Sized {
    /// attempts to convert the given bytes into a table section, returning None if there is a
    /// syntax or other error
    fn from_bytes(header: &SectionCommonHeader, table_syntax_header: &TableSyntaxHeader, data: &[u8]) -> Option<Self>;  // TODO: Result instead of Option?
}

use std::marker::PhantomData;

pub struct TableSectionConsumer<T> {
    phantom: PhantomData<T>,
    expected_last_section_number: Option<u8>,
    complete_section_count: u8,
    current_version: Option<u8>,
}

impl<T> TableSectionConsumer<T>
where
    T: TableSection
{
    const MAX_SECTIONS: usize = u8::max_value() as usize;  // given 1 byte section_number

    pub fn new() -> TableSectionConsumer<T> {
        TableSectionConsumer {
            phantom: PhantomData,
            expected_last_section_number: None,
            complete_section_count: 0,
            current_version: None,
        }
    }

    pub fn reset(&mut self) {
        // TODO: can some of this code be shared with new() somehow?
        self.expected_last_section_number = None;
        self.complete_section_count = 0;
    }

    fn complete(&self) -> bool {
        self.complete_section_count == self.expected_last_section_number.unwrap() + 1
    }

    fn is_new_version(&self, table_syntax_header: &TableSyntaxHeader) -> bool {
        if let Some(ver) = self.current_version {
            ver != table_syntax_header.version()
        } else {
            // there isn't yet a known version, so of course the given one is new to us
            true
        }
    }

    fn insert_section<'a>(&mut self, header: &SectionCommonHeader, table_syntax_header: TableSyntaxHeader<'a>, rest: &[u8]) -> Option<(TableSyntaxHeader<'a>, T)> {
        if table_syntax_header.current_next_indicator() == CurrentNext::Next {
            println!("skipping section where current_next_indicator indicates for future use, in table id {}", header.table_id);
            return None;
        }
        if self.is_new_version(&table_syntax_header) {
            self.reset();
            self.current_version = Some(table_syntax_header.version());
            self.expected_last_section_number = Some(table_syntax_header.last_section_number());
        } else {
            if let Some(current_last) = self.expected_last_section_number {
                if current_last != table_syntax_header.last_section_number() {
                    println!("last_section_number changed from {} to {}, but version remains {}", self.expected_last_section_number.unwrap(), table_syntax_header.last_section_number(), table_syntax_header.version());
                    self.reset();
                }
            }
            return None;
        }
        if let Some(section) = T::from_bytes(header, &table_syntax_header, rest) {
            // track the number of complete sections so that we'll know when we have the whole
            // table,
            Some((table_syntax_header, section))
        } else {
            println!("insert_section() failed to parse {:?} {:?}", header, table_syntax_header);
            None
        }
    }
}

impl<T> SectionProcessor<T> for TableSectionConsumer<T>
where
    T: TableSection
{
    fn process<'a>(&mut self, header: &SectionCommonHeader, payload: &'a[u8]) -> Option<(TableSyntaxHeader<'a>, T)> {
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
        self.insert_section(header, table_syntax_header, rest)
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

#[derive(Eq, PartialEq, Debug)]
enum SectionParseState {
    LookingForStart,
    WaitingForEnd,
    Finalised,
}

use std::marker;

/// Parser for MPEG TS PSI 'Section' syntax, which begins with the 8-bit `table_id` field.
pub struct SectionParser<T,P>
where
    P: SectionProcessor<T>
{
    phantom: marker::PhantomData<T>,
    buf: Vec<u8>,
    parse_state: SectionParseState,
    common_header: Option<SectionCommonHeader>,
    processor: P
}
impl<T,P> SectionParser<T,P>
where
    P: SectionProcessor<T>
{
    const SECTION_LIMIT: usize = 1021;

    pub fn new(cb: P) -> SectionParser<T,P> {
        SectionParser {
            phantom: marker::PhantomData,
            buf: Vec::new(),
            parse_state: SectionParseState::LookingForStart,
            common_header: None,
            processor: cb,
        }
    }

    fn get_common_header(&self) -> &SectionCommonHeader {
        self.common_header.as_ref().unwrap()
    }

    fn expected_length(&self) -> usize {
        self.get_common_header().section_length
    }

    fn maybe_reset(&mut self) {
        if self.parse_state == SectionParseState::Finalised {
            self.reset();
        }
    }

    pub fn begin_new_section(&mut self, data: &[u8]) -> Option<(TableSyntaxHeader, T)> {
        self.maybe_reset();
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
        if header.section_length > Self::SECTION_LIMIT {
            println!(
                "section_length {} is too large, limit is {} bytes",
                header.section_length,
                Self::SECTION_LIMIT
            );
            self.reset();
            return None;
        }
        self.common_header = Some(header);
        self.parse_state = SectionParseState::WaitingForEnd;
        self.append_to_current(data)
    }

    pub fn append_to_current(&mut self, data: &[u8]) -> Option<(TableSyntaxHeader, T)> {
        self.maybe_reset();
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

    fn finalise_current_section(&mut self) -> Option<(TableSyntaxHeader, T)> {
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
        self.parse_state = SectionParseState::Finalised;
        let result = self.processor.process(
            self.common_header.as_ref().unwrap(),
            // skip the 3 bytes of the common header,
            &self.buf[3..],
        );
        result
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.common_header = None;
        self.parse_state = SectionParseState::LookingForStart;
    }
}

/// A `PacketConsumer` for buffering Program Specific Information, which may be split across
/// multiple TS packets, and passing a complete PSI table to the given `SectionProcessor` when a
/// complete, valid section has been received.
pub struct SectionPacketConsumer<P,TP,T>
where
    P: SectionProcessor<T> + 'static,
    TP: TableProcessor<T>,
    T: TableSection
{
    phantom: marker::PhantomData<T>,
    parser: SectionParser<T,P>,
    table_processor: TP,
}


#[cfg(not(fuzz))]
const CRC_CHECK: bool = true;
#[cfg(fuzz)]
const CRC_CHECK: bool = false;

impl<P, TP, T> SectionPacketConsumer<P, TP, T>
where
    P: SectionProcessor<T> + 'static,
    TP: TableProcessor<T>,
    T: TableSection
{
    pub fn new(processor: P, table_processor: TP) -> SectionPacketConsumer<P,TP,T> {
        SectionPacketConsumer {
            phantom: marker::PhantomData,
            parser: SectionParser::new(processor),
            table_processor,
        }
    }
}

impl<P,TP,T> packet::PacketConsumer<demultiplex::FilterChangeset> for SectionPacketConsumer<P,TP,T>
where
    P: SectionProcessor<T> + 'static,
    TP: TableProcessor<T>,
    T: TableSection
{
    fn consume(&mut self, pk: packet::Packet) -> Option<demultiplex::FilterChangeset> {
        match pk.payload() {
            Some(pk_buf) => {
                let res = if pk.payload_unit_start_indicator() {
                    // this packet payload contains the start of a new PSI section
                    let pointer = pk_buf[0] as usize;
                    let section_data = &pk_buf[1..];
                    if pointer > 0 {
                        if pointer > section_data.len() {
                            println!("PSI pointer beyond end of packet payload");
                            self.parser.reset();
                            return None;
                        }
                        let remainder = &section_data[..pointer];
                        self.parser.append_to_current(remainder);
                        // the following call to begin_new_section() will assert that
                        // append_to_current() just finalised the preceding section
                    }
                    self.parser.begin_new_section(&section_data[pointer..])
                } else {
                    // this packet is a continuation of an existing PSI section
                    self.parser.append_to_current(pk_buf)
                };
                if let Some((table_syntax_header, section)) = res {
                    self.table_processor.process(&table_syntax_header, &section)
                } else {
                    None
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
    use super::*;
    use std::collections::HashMap;
    use data_encoding::base16;
    use packet::Packet;
    use packet::PacketConsumer;
    use demultiplex;
    use demultiplex::PatProcessor;
    use demultiplex::FilterChange;

    struct NullSection;
    impl TableSection for NullSection {
        fn from_bytes(header: &SectionCommonHeader, table_syntax_header: &TableSyntaxHeader, data: &[u8]) -> Option<Self> {
            Some(NullSection)
        }
    }
    struct NullSectionProcessor;
    impl SectionProcessor<NullSection> for NullSectionProcessor {
        fn process<'a>(&mut self, _header: &SectionCommonHeader, _section_payload: &'a[u8]) -> Option<(TableSyntaxHeader<'a>, NullSection)> { None }
    }
    struct NullTableProcessor;
    impl TableProcessor<NullSection> for NullTableProcessor {
        fn process(&mut self, table_syntax_header: &TableSyntaxHeader, sect: &NullSection) -> Option<demultiplex::FilterChangeset> {
            None
        }
    }

    fn empty_stream_constructor() -> demultiplex::StreamConstructor {
        demultiplex::StreamConstructor::new(demultiplex::NullPacketFilter::construct, HashMap::new())
    }

    #[test]
    fn continuation_outside_section() {
        let mut buf = [0u8; 188];
        buf[0] = 0x47;
        buf[3] |= 0b00010000; // PayloadOnly
        let pk = Packet::new(&buf[..]);
        let mut psi_buf = SectionPacketConsumer::new(NullSectionProcessor, NullTableProcessor);
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
        let mut psi_buf = SectionPacketConsumer::new(NullSectionProcessor, NullTableProcessor);
        psi_buf.consume(pk);
    }

    #[test]
    fn example() {
        let buf = base16::decode(b"474000150000B00D0001C100000001E1E02D507804FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").unwrap();
        let pk = Packet::new(&buf[..]);
        let pat_proc = PatProcessor::new(empty_stream_constructor());
        let table_sec = TableSectionConsumer::new();
        let mut section_pk = SectionPacketConsumer::new(table_sec, pat_proc);
        if let Some(changeset) = section_pk.consume(pk) {
            let mut iter = changeset.into_iter();
            assert!(if let Some(FilterChange::Insert(pid, _)) = iter.next() { pid == 480 } else { false });
        } else {
            assert!(false, "consuming PAT packet should have created a new filter entry");
        }
    }
}
