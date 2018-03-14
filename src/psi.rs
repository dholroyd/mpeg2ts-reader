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
///    [`SectionSyntaxSectionProcessor`](struct.SectionSyntaxSectionProcessor.html) implementation of this trait
///    can be used.
///  - This trait should be implemented directly for PSI tables that use 'compact' syntax (i.e.
///    they lack the 5-bytes-worth of fields represented by [`TableSyntaxHeader`](struct.TableSyntaxHeader.html))
pub trait SectionProcessor {
    type Ret;

    /// Note that the first 3 bytes of `section_data` contain the header fields that have also
    /// been supplied to this call in the `header` parameter.  This is to allow implementers to
    /// calculate a CRC over the whole section if required.
    fn start_section<'a>(&mut self, header: &SectionCommonHeader, section_data: &'a[u8]) -> Option<Self::Ret>;
    fn continue_section<'a>(&mut self, section_data: &'a[u8]) -> Option<Self::Ret>;
    fn reset(&mut self);
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


impl<'buf> TableSyntaxHeader<'buf> {
    pub const SIZE: usize = 5;

    pub fn new(buf: &'buf[u8]) -> TableSyntaxHeader {
        assert!(buf.len() >= Self::SIZE);
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

pub struct CrcCheckWholeSectionSyntaxPayloadParser<P>
where
    P: WholeSectionSyntaxPayloadParser
{
    inner: P,
}
impl<P> CrcCheckWholeSectionSyntaxPayloadParser<P>
    where
        P: WholeSectionSyntaxPayloadParser
{
    pub fn new(inner: P) -> CrcCheckWholeSectionSyntaxPayloadParser<P> {
        CrcCheckWholeSectionSyntaxPayloadParser {
            inner,
        }
    }
}

impl<P> WholeSectionSyntaxPayloadParser for CrcCheckWholeSectionSyntaxPayloadParser<P>
where
    P: WholeSectionSyntaxPayloadParser
{
    type Ret = P::Ret;

    fn section<'a>(&mut self, header: &SectionCommonHeader, table_syntax_header: &TableSyntaxHeader, data: &'a [u8]) -> Option<Self::Ret> {
        assert!(header.section_syntax_indicator);
        if CRC_CHECK && mpegts_crc::sum32(data) != 0 {
            println!(
                "section crc check failed for table_id {}",
                header.table_id,
            );
            hexdump::hexdump(data);
            return None;
        }
        self.inner.section(header, table_syntax_header, data)
    }
}

pub trait WholeSectionSyntaxPayloadParser {
    type Ret;

    fn section<'a>(&mut self, header: &SectionCommonHeader, table_syntax_header: &TableSyntaxHeader, data: &'a [u8]) -> Option<Self::Ret>;
}

pub fn section_syntax_payload(buf: &[u8]) -> &[u8] { &buf[SectionCommonHeader::SIZE+TableSyntaxHeader::SIZE..] }

enum BufferSectionState {
    Buffering(usize),
    Complete,
}

/// Implements `BufferSectionSyntaxParser` so that any sections that cross TS-packet boundaries
/// are collected into a single byte-buffer for easier parsing.  In the common case that the
/// section fits entirely in a single TS packet, the implementation is zero-copy.
pub struct BufferSectionSyntaxParser<P>
where
    P: WholeSectionSyntaxPayloadParser
{
    buf: Vec<u8>,
    state: BufferSectionState,
    parser: P,
}
impl<P> BufferSectionSyntaxParser<P>
    where
        P: WholeSectionSyntaxPayloadParser
{
    pub fn new(parser: P) -> BufferSectionSyntaxParser<P> {
        BufferSectionSyntaxParser {
            buf: vec!(),
            state: BufferSectionState::Complete,
            parser,
        }
    }
}
impl<P> SectionSyntaxPayloadParser for BufferSectionSyntaxParser<P>
where
    P: WholeSectionSyntaxPayloadParser
{
    type Ret = P::Ret;

    fn start_syntax_section<'a>(&mut self, header: &SectionCommonHeader, table_syntax_header: &TableSyntaxHeader, data: &'a [u8]) -> Option<Self::Ret> {
        if header.section_length <=  data.len() - SectionCommonHeader::SIZE {
            self.state = BufferSectionState::Complete;
            self.parser.section(header, table_syntax_header, &data[..header.section_length + SectionCommonHeader::SIZE])
        } else {
            let to_read = if data.len() > header.section_length {
                header.section_length
            } else {
                header.section_length - data.len()
            };
            self.state = BufferSectionState::Buffering(to_read);
            self.buf.clear();
            self.buf.extend_from_slice(data);
            None
        }
    }

    fn continue_syntax_section<'a>(&mut self, data: &'a [u8]) -> Option<Self::Ret> {
        match self.state {
            BufferSectionState::Complete => {
                println!("attempt to add extra data when section already complete");
                None
            },
            BufferSectionState::Buffering(remaining) => {
                assert!(remaining >= data.len());
                let new_remaining = remaining - data.len();
                if new_remaining == 0 {
                    let payload = section_syntax_payload(&self.buf[..]);
                    self.state = BufferSectionState::Complete;
                    let header = SectionCommonHeader::new(&self.buf[..]);
                    let table_syntax_header = TableSyntaxHeader::new(&self.buf[SectionCommonHeader::SIZE..]);
                    self.parser.section(&header, &table_syntax_header, payload)
                } else {
                    None
                }
            }
        }
    }
    fn reset(&mut self) {
        self.buf.clear();
        self.state = BufferSectionState::Complete;
    }
}

/// A wrapper around some other implementation of `SectionSyntaxPayloadParser` that passes-through
/// section data, unless the `TableSyntaxHeader` indicates a version_number which is the same as
/// the last data that was passed though.
///
/// This avoids the underlying code needing to re-parse duplicate copies of the section, which are
/// usually inserted periodically in the Transport Stream.
pub struct DedupSectionSyntaxPayloadParser<SSPP>
where
    SSPP: SectionSyntaxPayloadParser
{
    inner: SSPP,
    last_version: Option<u8>,
    ignore_rest: bool,
}
impl<SSPP> DedupSectionSyntaxPayloadParser<SSPP>
    where
        SSPP: SectionSyntaxPayloadParser
{
    pub fn new(inner: SSPP) -> DedupSectionSyntaxPayloadParser<SSPP> {
        DedupSectionSyntaxPayloadParser {
            inner,
            last_version: None,
            ignore_rest: false,
        }
    }
}
impl<SSPP> SectionSyntaxPayloadParser for DedupSectionSyntaxPayloadParser<SSPP>
where
    SSPP: SectionSyntaxPayloadParser
{
    type Ret = SSPP::Ret;

    fn start_syntax_section<'a>(&mut self, header: &SectionCommonHeader, table_syntax_header: &TableSyntaxHeader, data: &'a [u8]) -> Option<Self::Ret> {
        if let Some(last) = self.last_version {
            if last == table_syntax_header.version() {
                self.ignore_rest = true;
                return None;
            }
        }
        self.ignore_rest = false;
        self.last_version = Some(table_syntax_header.version());
        self.inner.start_syntax_section(header, table_syntax_header, data)
    }

    fn continue_syntax_section<'a>(&mut self, data: &'a [u8]) -> Option<Self::Ret> {
        if self.ignore_rest {
            None
        } else {
            self.inner.continue_syntax_section(data)
        }
    }
    fn reset(&mut self) {
        self.inner.reset();
        self.last_version = None;
        self.ignore_rest = false;
    }
}

/// Trait for types that will handle MPEGTS PSI table sections with 'section syntax'.
pub trait SectionSyntaxPayloadParser {
    type Ret;

    /// NB the `data` buffer passed to _will_ include the bytes which are represented by `header`
    /// and `table_syntax_header` (in order that the called code can check any CRC that covers the
    /// whole section).
    fn start_syntax_section<'a>(&mut self,
                            header: &SectionCommonHeader,
                            table_syntax_header: &TableSyntaxHeader, data: &'a [u8])
        -> Option<Self::Ret>;

    fn continue_syntax_section<'a>(&mut self, data: &'a [u8]) -> Option<Self::Ret>;

    fn reset(&mut self);
}

pub struct SectionSyntaxSectionProcessor<SP>
where
    SP: SectionSyntaxPayloadParser
{
    payload_parser: SP,
    ignore_rest: bool,
}
impl<SP> SectionSyntaxSectionProcessor<SP>
    where
        SP: SectionSyntaxPayloadParser
{
    const SECTION_LIMIT: usize = 1021;

    pub fn new(payload_parser: SP) -> SectionSyntaxSectionProcessor<SP> {
        SectionSyntaxSectionProcessor {
            payload_parser,
            ignore_rest: false,
        }
    }
}
impl<SP> SectionProcessor for SectionSyntaxSectionProcessor<SP>
where
    SP: SectionSyntaxPayloadParser
{
    type Ret = SP::Ret;

    fn start_section<'a>(&mut self, header: &SectionCommonHeader, data: &'a [u8]) -> Option<Self::Ret> {
        if !header.section_syntax_indicator {
            println!(
                "SectionSyntaxSectionProcessor requires that section_syntax_indicator be set in the section header"
            );
            self.ignore_rest = true;
            return None;
        }
        if data.len() < SectionCommonHeader::SIZE + TableSyntaxHeader::SIZE {
            println!("data too short for header (TODO: implement buffering)");
            self.ignore_rest = true;
            return None;
        }
        if header.section_length > Self::SECTION_LIMIT {
            println!("section_length={} is too large (limit {})", header.section_length, Self::SECTION_LIMIT);
            self.ignore_rest = true;
            return None;
        }
        self.ignore_rest = false;
        let table_syntax_header = TableSyntaxHeader::new(&data[SectionCommonHeader::SIZE..]);
        self.payload_parser.start_syntax_section(header, &table_syntax_header, data)
    }

    fn continue_section<'a>(&mut self, data: &'a [u8]) -> Option<Self::Ret> {
        if !self.ignore_rest {
            self.payload_parser.continue_syntax_section(data)
        } else {
            None
        }
    }
    fn reset(&mut self) {
        self.payload_parser.reset()
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
    pub const SIZE: usize = 3;
    pub fn new(buf: &[u8]) -> SectionCommonHeader {
        assert_eq!(buf.len(), Self::SIZE);
        SectionCommonHeader {
            table_id: buf[0],
            section_syntax_indicator: buf[1] & 0b10000000 != 0,
            private_indicator: buf[1] & 0b01000000 != 0,
            section_length: ((u16::from(buf[1] & 0b00001111) << 8) | u16::from(buf[2])) as usize,
        }
    }
}

/// A `PacketConsumer` for buffering Program Specific Information, which may be split across
/// multiple TS packets, and passing a complete PSI table to the given `SectionProcessor` when a
/// complete, valid section has been received.
pub struct SectionPacketConsumer<P>
where
    P: SectionProcessor + 'static,
{
    parser: P,
}


#[cfg(not(fuzz))]
const CRC_CHECK: bool = true;
#[cfg(fuzz)]
const CRC_CHECK: bool = false;

impl<P> SectionPacketConsumer<P>
where
    P: SectionProcessor<Ret=demultiplex::FilterChangeset>,
{
    pub fn new(parser: P) -> SectionPacketConsumer<P> {
        SectionPacketConsumer {
            parser,
        }
    }
}

impl<P> packet::PacketConsumer<demultiplex::FilterChangeset> for SectionPacketConsumer<P>
where
    P: SectionProcessor<Ret=demultiplex::FilterChangeset>,
{
    fn consume(&mut self, pk: packet::Packet) -> Option<demultiplex::FilterChangeset> {
        match pk.payload() {
            Some(pk_buf) => {
                if pk.payload_unit_start_indicator() {
                    // this packet payload contains the start of a new PSI section
                    let pointer = pk_buf[0] as usize;
                    let section_data = &pk_buf[1..];
                    if pointer > 0 {
                        if pointer >= section_data.len() {
                            println!("PSI pointer beyond end of packet payload");
                            self.parser.reset();
                            return None;
                        }
                        let remainder = &section_data[..pointer];
                        // TODO: need way to produce this intermediate result
                        let _res = self.parser.continue_section(remainder);
                        // the following call to begin_new_section() will assert that
                        // append_to_current() just finalised the preceding section
                    }
                    let next_sect = &section_data[pointer..];
                    if next_sect.len() < SectionCommonHeader::SIZE {
                        println!("TODO: not enough bytes to read section header - implement buffering");
                        self.parser.reset();
                        return None;
                    }
                    let header = SectionCommonHeader::new(&next_sect[..SectionCommonHeader::SIZE]);
                    self.parser.start_section(&header, next_sect)
                } else {
                    // this packet is a continuation of an existing PSI section
                    self.parser.continue_section(pk_buf)
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
    use packet::Packet;
    use packet::PacketConsumer;
    use demultiplex;

    struct NullSectionProcessor;
    impl SectionProcessor for NullSectionProcessor {
        type Ret = demultiplex::FilterChangeset;
        fn start_section<'a>(&mut self, _header: &SectionCommonHeader, _section_data: &'a [u8]) -> Option<Self::Ret> { None }
        fn continue_section<'a>(&mut self, _section_data: &'a [u8]) -> Option<Self::Ret> { None }
        fn reset(&mut self) { }
    }

    #[test]
    fn continuation_outside_section() {
        let mut buf = [0u8; 188];
        buf[0] = 0x47;
        buf[3] |= 0b00010000; // PayloadOnly
        let pk = Packet::new(&buf[..]);
        let mut psi_buf = SectionPacketConsumer::new(NullSectionProcessor);
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
        let mut psi_buf = SectionPacketConsumer::new(NullSectionProcessor);
        psi_buf.consume(pk);
    }
}
