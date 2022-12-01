//! Types for processing tables of *Program Specific Information* in a transport stream.
//!
//! # Concepts
//!
//! * There are multiple standard types of Program Specific Information, like the *Program
//!   Association Table* and *Program Map Table*.  Standards derived from mpegts may define their
//!   own table types.
//! * A PSI *Table* can split into *Sections*
//! * A Section can be split across a small number of individual transport stream *Packets*
//! * The payload of a section may use *section-syntax* or *compact-syntax*, as indicated by the
//!   [`section_syntax_indicator`](struct.SectionCommonHeader.html#structfield.section_syntax_indicator)
//!   attribute
//!   * *Section-syntax* sections have additional header data, represented by the
//!     `TableSyntaxHeader` type
//!   * *Compact-syntax* sections lack this extra header data
//!
//! # Core types
//!
//! * [`SectionPacketConsumer`](struct.SectionPacketConsumer.html) converts *Packets* into *Sections*
//!
//! Note that the specific types of table such as Program Association Table are defined elsewhere
//! with only the generic functionality in this module.

pub mod pat;
pub mod pmt;

use crate::mpegts_crc;
use crate::packet;
use log::warn;
use std::fmt;

// TODO: there is quite some duplication between XxxSectionSyntaxYyy and XxxCompactSyntaxYyy types
//       refactor to reduce the repeated code.

/// Trait for types which process the data within a PSI section following the 12-byte
/// `section_length` field (which is one of the items available in the `SectionCommonHeader` that
/// is passed in.
///
///  - For PSI tables that use 'section syntax', the existing
///    [`SectionSyntaxSectionProcessor`](struct.SectionSyntaxSectionProcessor.html) implementation of this trait
///    can be used.
///  - This trait should be implemented directly for PSI tables that use 'compact' syntax (i.e.
///    they lack the 5-bytes-worth of fields represented by [`TableSyntaxHeader`](struct.TableSyntaxHeader.html))
///
/// Implementations of this trait will need to use the `section_length` method of the `header`
/// param passed to `start_section()` to determine when the complete section has been supplied
/// (if the complete header is not supplied in the call to `start_section()` more data may be
/// supplied in one or more subsequent calls to `continue_section()`.
pub trait SectionProcessor {
    /// The type of the context object that the caller will pass through to the methods of this
    /// trait
    type Context;

    /// Note that the first 3 bytes of `section_data` contain the header fields that have also
    /// been supplied to this call in the `header` parameter.  This is to allow implementers to
    /// calculate a CRC over the whole section if required.
    fn start_section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        section_data: &'a [u8],
    );
    /// may be called to pass the implementation additional slices of section data, if the
    /// complete section was not already passed.
    fn continue_section<'a>(&mut self, ctx: &mut Self::Context, section_data: &'a [u8]);

    /// called if there is a problem in the transport stream that means any in-progress section
    /// data should be discarded.
    fn reset(&mut self);
}

/// Represents the value of the Transport Stream `current_next_indicator` field.
#[derive(Debug, PartialEq, Eq)]
pub enum CurrentNext {
    /// The section version number applies to the currently applicable section data
    Current,
    /// The section version number applies to the next applicable section data
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

/// Represents the fields that appear within table sections that use the common 'section syntax'.
///
/// This will only be used for a table section if the
/// [`section_syntax_indicator`](struct.SectionCommonHeader.html#structfield.section_syntax_indicator)
/// field in the `SectionCommonHeader` of the section is `true`.
pub struct TableSyntaxHeader<'buf> {
    buf: &'buf [u8],
}

impl<'buf> TableSyntaxHeader<'buf> {
    /// The size of the header; 5 bytes
    pub const SIZE: usize = 5;

    /// Constructs a new TableSyntaxHeader, wrapping the given slice, which will all parsing of
    /// the header's fields.
    ///
    /// Panics if the given slice is less than `TableSyntaxHeader::SIZE` bytes long.
    pub fn new(buf: &'buf [u8]) -> TableSyntaxHeader<'buf> {
        assert!(buf.len() >= Self::SIZE);
        TableSyntaxHeader { buf }
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
        (self.buf[2] >> 1) & 0b0001_1111
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
impl<'buf> fmt::Debug for TableSyntaxHeader<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("TableSyntaxHeader")
            .field("id", &self.id())
            .field("version", &self.version())
            .field("current_next_indicator", &self.current_next_indicator())
            .field("section_number", &self.section_number())
            .field("last_section_number", &self.last_section_number())
            .finish()
    }
}

/// An implementation of `WholeSectionSyntaxPayloadParser` which will delegate to another
/// instance of `WholeSectionSyntaxPayloadParser` only if the CRC of the section data is
/// correct.
pub struct CrcCheckWholeSectionSyntaxPayloadParser<P>
where
    P: WholeSectionSyntaxPayloadParser,
{
    inner: P,
}
impl<P> CrcCheckWholeSectionSyntaxPayloadParser<P>
where
    P: WholeSectionSyntaxPayloadParser,
{
    const CRC_SIZE: usize = 4;

    /// create a new CrcCheckWholeSectionSyntaxPayloadParser which wraps and delegates to the given
    /// `WholeSectionSyntaxPayloadParser` instance
    pub fn new(inner: P) -> CrcCheckWholeSectionSyntaxPayloadParser<P> {
        CrcCheckWholeSectionSyntaxPayloadParser { inner }
    }
}

impl<P> WholeSectionSyntaxPayloadParser for CrcCheckWholeSectionSyntaxPayloadParser<P>
where
    P: WholeSectionSyntaxPayloadParser,
{
    type Context = P::Context;

    fn section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        table_syntax_header: &TableSyntaxHeader<'a>,
        data: &'a [u8],
    ) {
        assert!(header.section_syntax_indicator);
        if data.len() < SectionCommonHeader::SIZE + TableSyntaxHeader::SIZE + Self::CRC_SIZE {
            // must be big enough to hold the CRC!
            warn!(
                "section data length too small for table_id {}: {}",
                header.table_id,
                data.len()
            );
            return;
        }
        // don't apply CRC checks when fuzzing, to give more chances of test data triggering
        // parser bugs,
        if !cfg!(fuzzing) && mpegts_crc::sum32(data) != 0 {
            warn!("section crc check failed for table_id {}", header.table_id,);
            return;
        }
        self.inner.section(ctx, header, table_syntax_header, data);
    }
}

/// Trait for types that parse fully reconstructed PSI table sections (which requires the caller
/// to have buffered section data if it spanned multiple TS packets.
pub trait WholeSectionSyntaxPayloadParser {
    /// Type of the context object that will be passed to all methods.
    type Context;

    /// Method that will receive a complete PSI table section, where the `data` parameter will
    /// be `header.section_length` bytes long
    fn section<'a>(
        &mut self,
        _: &mut Self::Context,
        header: &SectionCommonHeader,
        table_syntax_header: &TableSyntaxHeader<'a>,
        data: &'a [u8],
    );
}

/// Trait for types that parse fully reconstructed PSI table sections.
///
/// This requires the caller to have buffered section data if it spanned multiple TS packets,
/// and the `BufferCompactSyntaxParser` type is available to perform such buffering.
pub trait WholeCompactSyntaxPayloadParser {
    /// Type of the context object that will be passed to all methods.
    type Context;

    /// Method that will receive a complete PSI table section, where the `data` parameter will
    /// be `header.section_length` bytes long
    fn section<'a>(&mut self, _: &mut Self::Context, header: &SectionCommonHeader, data: &'a [u8]);
}

enum BufferSectionState {
    Buffering(usize),
    Complete,
}

/// Implements `SectionSyntaxPayloadParser` so that any sections that cross TS-packet boundaries
/// are collected into a single byte-buffer for easier parsing.  In the common case that the
/// section fits entirely in a single TS packet, the implementation is zero-copy.
pub struct BufferSectionSyntaxParser<P>
where
    P: WholeSectionSyntaxPayloadParser,
{
    buf: Vec<u8>,
    state: BufferSectionState,
    parser: P,
}
impl<P> BufferSectionSyntaxParser<P>
where
    P: WholeSectionSyntaxPayloadParser,
{
    /// wraps the given `WholeSectionSyntaxPayloadParser` instance in a new
    /// `BufferSectionSyntaxParser`.
    pub fn new(parser: P) -> BufferSectionSyntaxParser<P> {
        BufferSectionSyntaxParser {
            buf: vec![],
            state: BufferSectionState::Complete,
            parser,
        }
    }
}
impl<P> SectionSyntaxPayloadParser for BufferSectionSyntaxParser<P>
where
    P: WholeSectionSyntaxPayloadParser,
{
    type Context = P::Context;

    fn start_syntax_section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        table_syntax_header: &TableSyntaxHeader<'a>,
        data: &'a [u8],
    ) {
        let section_length_with_header = header.section_length + SectionCommonHeader::SIZE;
        if section_length_with_header <= data.len() {
            // section data is entirely within this packet,
            self.state = BufferSectionState::Complete;
            self.parser.section(
                ctx,
                header,
                table_syntax_header,
                &data[..section_length_with_header],
            )
        } else {
            // we will need to wait for continuation packets before we have the whole section,
            self.buf.clear();
            self.buf.extend_from_slice(data);
            let to_read = section_length_with_header - data.len();
            self.state = BufferSectionState::Buffering(to_read);
        }
    }

    fn continue_syntax_section<'a>(&mut self, ctx: &mut Self::Context, data: &'a [u8]) {
        match self.state {
            BufferSectionState::Complete => {
                warn!("attempt to add extra data when section already complete");
            }
            BufferSectionState::Buffering(remaining) => {
                let new_remaining = if data.len() > remaining {
                    0
                } else {
                    remaining - data.len()
                };
                if new_remaining == 0 {
                    self.buf.extend_from_slice(&data[..remaining]);
                    self.state = BufferSectionState::Complete;
                    let header = SectionCommonHeader::new(&self.buf[..SectionCommonHeader::SIZE]);
                    let table_syntax_header =
                        TableSyntaxHeader::new(&self.buf[SectionCommonHeader::SIZE..]);
                    self.parser
                        .section(ctx, &header, &table_syntax_header, &self.buf[..]);
                } else {
                    self.buf.extend_from_slice(data);
                    self.state = BufferSectionState::Buffering(new_remaining);
                }
            }
        }
    }
    fn reset(&mut self) {
        self.buf.clear();
        self.state = BufferSectionState::Complete;
    }
}

/// Implements `CompactSyntaxPayloadParser` so that any sections that cross TS-packet boundaries
/// are collected into a single byte-buffer for easier parsing.  In the common case that the
/// section fits entirely in a single TS packet, the implementation is zero-copy.
pub struct BufferCompactSyntaxParser<P>
where
    P: WholeCompactSyntaxPayloadParser,
{
    buf: Vec<u8>,
    state: BufferSectionState,
    parser: P,
}
impl<P> BufferCompactSyntaxParser<P>
where
    P: WholeCompactSyntaxPayloadParser,
{
    /// wraps the given `WholeSectionSyntaxPayloadParser` instance in a new
    /// `BufferSectionSyntaxParser`.
    pub fn new(parser: P) -> BufferCompactSyntaxParser<P> {
        BufferCompactSyntaxParser {
            buf: vec![],
            state: BufferSectionState::Complete,
            parser,
        }
    }
}
impl<P> CompactSyntaxPayloadParser for BufferCompactSyntaxParser<P>
where
    P: WholeCompactSyntaxPayloadParser,
{
    type Context = P::Context;

    fn start_compact_section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        data: &'a [u8],
    ) {
        let section_length_with_header = header.section_length + SectionCommonHeader::SIZE;
        if section_length_with_header <= data.len() {
            // section data is entirely within this packet,
            self.state = BufferSectionState::Complete;
            self.parser
                .section(ctx, header, &data[..section_length_with_header])
        } else {
            // we will need to wait for continuation packets before we have the whole section,
            self.buf.clear();
            self.buf.extend_from_slice(data);
            let to_read = section_length_with_header - data.len();
            self.state = BufferSectionState::Buffering(to_read);
        }
    }

    fn continue_compact_section<'a>(&mut self, ctx: &mut Self::Context, data: &'a [u8]) {
        match self.state {
            BufferSectionState::Complete => {
                warn!("attempt to add extra data when section already complete");
            }
            BufferSectionState::Buffering(remaining) => {
                let new_remaining = if data.len() > remaining {
                    0
                } else {
                    remaining - data.len()
                };
                if new_remaining == 0 {
                    self.buf.extend_from_slice(&data[..remaining]);
                    self.state = BufferSectionState::Complete;
                    let header = SectionCommonHeader::new(&self.buf[..SectionCommonHeader::SIZE]);
                    self.parser.section(ctx, &header, &self.buf[..]);
                } else {
                    self.buf.extend_from_slice(data);
                    self.state = BufferSectionState::Buffering(new_remaining);
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
    SSPP: SectionSyntaxPayloadParser,
{
    inner: SSPP,
    last_version: Option<u8>,
    ignore_rest: bool,
}
impl<SSPP> DedupSectionSyntaxPayloadParser<SSPP>
where
    SSPP: SectionSyntaxPayloadParser,
{
    /// Wraps the given `SectionSyntaxPayloadParser` in a new `DedupSectionSyntaxPayloadParser`
    /// instance.
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
    SSPP: SectionSyntaxPayloadParser,
{
    type Context = SSPP::Context;

    fn start_syntax_section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        table_syntax_header: &TableSyntaxHeader<'a>,
        data: &'a [u8],
    ) {
        if let Some(last) = self.last_version {
            if last == table_syntax_header.version() {
                self.ignore_rest = true;
                return;
            }
        }
        self.ignore_rest = false;
        self.last_version = Some(table_syntax_header.version());
        self.inner
            .start_syntax_section(ctx, header, table_syntax_header, data);
    }

    fn continue_syntax_section<'a>(&mut self, ctx: &mut Self::Context, data: &'a [u8]) {
        if !self.ignore_rest {
            self.inner.continue_syntax_section(ctx, data)
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
    /// The type of the context object passed to all methods
    type Context;

    /// NB the `data` buffer passed to _will_ include the bytes which are represented by `header`
    /// and `table_syntax_header` (in order that the called code can check any CRC that covers the
    /// whole section).
    fn start_syntax_section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        table_syntax_header: &TableSyntaxHeader<'a>,
        data: &'a [u8],
    );

    /// may be called to pass the implementation additional slices of section data, if the
    /// complete section was not already passed.
    fn continue_syntax_section<'a>(&mut self, ctx: &mut Self::Context, data: &'a [u8]);

    /// called if there is a problem in the transport stream that means any in-progress section
    /// data should be discarded.
    fn reset(&mut self);
}

/// Trait for types that will handle MPEGTS PSI table sections with 'compact syntax'.
pub trait CompactSyntaxPayloadParser {
    /// The type of the context object passed to all methods
    type Context;

    /// NB the `data` buffer passed to _will_ include the bytes which are represented by `header`
    /// (in order that the called code can check any CRC that covers the
    /// whole section).
    fn start_compact_section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        data: &'a [u8],
    );

    /// may be called to pass the implementation additional slices of section data, if the
    /// complete section was not already passed.
    fn continue_compact_section<'a>(&mut self, ctx: &mut Self::Context, data: &'a [u8]);

    /// called if there is a problem in the transport stream that means any in-progress section
    /// data should be discarded.
    fn reset(&mut self);
}

/// An implementation of `SectionProcessor` to be used for sections that implement 'compact syntax'
/// (rather than 'section syntax').
///
/// Delegates handling to the `CompactSyntaxPayloadParser` instance given at construction time.
pub struct CompactSyntaxSectionProcessor<SP>
where
    SP: CompactSyntaxPayloadParser,
{
    payload_parser: SP,
    ignore_rest: bool,
}
impl<SP> CompactSyntaxSectionProcessor<SP>
where
    SP: CompactSyntaxPayloadParser,
{
    const SECTION_LIMIT: usize = 1021;

    /// Wraps the given `CompactSyntaxPayloadParser` instance in a new
    /// `CompactSyntaxSectionProcessor`.
    pub fn new(payload_parser: SP) -> CompactSyntaxSectionProcessor<SP> {
        CompactSyntaxSectionProcessor {
            payload_parser,
            ignore_rest: false,
        }
    }
}
impl<SP> SectionProcessor for CompactSyntaxSectionProcessor<SP>
where
    SP: CompactSyntaxPayloadParser,
{
    type Context = SP::Context;

    fn start_section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        data: &'a [u8],
    ) {
        if header.section_syntax_indicator {
            // Maybe this should actually be allowed in some cases?
            warn!(
                "CompactSyntaxSectionProcessor requires that section_syntax_indicator NOT be set in the section header"
            );
            self.ignore_rest = true;
            return;
        }
        if data.len() < SectionCommonHeader::SIZE {
            warn!("CompactSyntaxSectionProcessor data {} too short for header {} (TODO: implement buffering)", data.len(), SectionCommonHeader::SIZE + TableSyntaxHeader::SIZE);
            self.ignore_rest = true;
            return;
        }
        if header.section_length > Self::SECTION_LIMIT {
            warn!(
                "CompactSyntaxSectionProcessor section_length={} is too large (limit {})",
                header.section_length,
                Self::SECTION_LIMIT
            );
            self.ignore_rest = true;
            return;
        }
        self.ignore_rest = false;
        self.payload_parser.start_compact_section(ctx, header, data)
    }

    fn continue_section<'a>(&mut self, ctx: &mut Self::Context, data: &'a [u8]) {
        if !self.ignore_rest {
            self.payload_parser.continue_compact_section(ctx, data)
        }
    }
    fn reset(&mut self) {
        self.payload_parser.reset()
    }
}

/// An implementation of `SectionProcessor` to be used for sections that implement 'section syntax'
/// (rather than 'compact syntax').
///
/// Parses the `TableSyntaxHeader` at the front of the section data, and then delegates handling
/// to the `SectionSyntaxPayloadParser` instance given at construction time.
pub struct SectionSyntaxSectionProcessor<SP>
where
    SP: SectionSyntaxPayloadParser,
{
    payload_parser: SP,
    ignore_rest: bool,
}
impl<SP> SectionSyntaxSectionProcessor<SP>
where
    SP: SectionSyntaxPayloadParser,
{
    const SECTION_LIMIT: usize = 1021;

    /// Wraps the given `SectionSyntaxPayloadParser` instance in a new
    /// `SectionSyntaxSectionProcessor`.
    pub fn new(payload_parser: SP) -> SectionSyntaxSectionProcessor<SP> {
        SectionSyntaxSectionProcessor {
            payload_parser,
            ignore_rest: false,
        }
    }
}
impl<SP> SectionProcessor for SectionSyntaxSectionProcessor<SP>
where
    SP: SectionSyntaxPayloadParser,
{
    type Context = SP::Context;

    fn start_section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        data: &'a [u8],
    ) {
        if !header.section_syntax_indicator {
            warn!(
                "SectionSyntaxSectionProcessor requires that section_syntax_indicator be set in the section header"
            );
            self.ignore_rest = true;
            return;
        }
        if data.len() < SectionCommonHeader::SIZE + TableSyntaxHeader::SIZE {
            warn!("SectionSyntaxSectionProcessor data {} too short for header {} (TODO: implement buffering)", data.len(), SectionCommonHeader::SIZE + TableSyntaxHeader::SIZE);
            self.ignore_rest = true;
            return;
        }
        if header.section_length > Self::SECTION_LIMIT {
            warn!(
                "SectionSyntaxSectionProcessor section_length={} is too large (limit {})",
                header.section_length,
                Self::SECTION_LIMIT
            );
            self.ignore_rest = true;
            return;
        }
        self.ignore_rest = false;
        let table_syntax_header = TableSyntaxHeader::new(&data[SectionCommonHeader::SIZE..]);
        self.payload_parser
            .start_syntax_section(ctx, header, &table_syntax_header, data)
    }

    fn continue_section<'a>(&mut self, ctx: &mut Self::Context, data: &'a [u8]) {
        if !self.ignore_rest {
            self.payload_parser.continue_syntax_section(ctx, data)
        }
    }
    fn reset(&mut self) {
        self.payload_parser.reset()
    }
}

/// Header common to all PSI sections, whether they then use 'section syntax' or 'compact syntax'.
#[derive(Debug)]
pub struct SectionCommonHeader {
    /// The type of table of which this is a section
    pub table_id: u8,
    /// `true` for 'section syntax`, `false` for 'compact syntax'.
    pub section_syntax_indicator: bool,
    /// indicates that the data in the table is for private use not defined in _ISO/IEC 13818-1_
    /// (section types implemented in this crate are to be used with data that has `e` in
    /// this field, but other crates might be written to support private table sections).
    pub private_indicator: bool,
    /// the number of bytes in the section data immediately following this field (which may be
    /// more bytes than will fit into a single TS packet).
    pub section_length: usize,
}

impl SectionCommonHeader {
    /// The fixed size of the CommonSectionHeader data in the Transport Stream; 3 bytes.
    pub const SIZE: usize = 3;

    /// Parses the data in the given slice into a new `SectionCommonHeader`.
    ///
    /// Panics if the slice is not exactly 3 bytes long.
    pub fn new(buf: &[u8]) -> SectionCommonHeader {
        assert_eq!(buf.len(), Self::SIZE);
        SectionCommonHeader {
            table_id: buf[0],
            section_syntax_indicator: buf[1] & 0b1000_0000 != 0,
            private_indicator: buf[1] & 0b0100_0000 != 0,
            section_length: ((u16::from(buf[1] & 0b0000_1111) << 8) | u16::from(buf[2])) as usize,
        }
    }
}

/// A type for locating the headers of PSI sections, which may be split across multiple TS packets,
/// and passing each piece to the given `SectionProcessor` as it is discovered.
pub struct SectionPacketConsumer<P>
where
    P: SectionProcessor,
{
    parser: P,
}

// TODO: maybe just implement PacketFilter directly

impl<P, Ctx> SectionPacketConsumer<P>
where
    P: SectionProcessor<Context = Ctx>,
{
    /// Construct a new instance that will delegate processing of section data found in TS packet
    /// payloads to the given `SectionProcessor` instance.
    pub fn new(parser: P) -> SectionPacketConsumer<P> {
        SectionPacketConsumer { parser }
    }

    /// process the payload of the given TS packet, passing each piece of section data discovered
    /// to the `SectionProcessor` instance given at time of construction.
    pub fn consume(&mut self, ctx: &mut Ctx, pk: &packet::Packet<'_>) {
        match pk.payload() {
            Some(pk_buf) => {
                if pk.payload_unit_start_indicator() {
                    // this packet payload contains the start of a new PSI section
                    let pointer = pk_buf[0] as usize;
                    let section_data = &pk_buf[1..];
                    if pointer > 0 {
                        if pointer >= section_data.len() {
                            warn!("PSI pointer beyond end of packet payload");
                            self.parser.reset();
                            return;
                        }
                        let remainder = &section_data[..pointer];
                        self.parser.continue_section(ctx, remainder);
                        // the following call to begin_new_section() will assert that
                        // append_to_current() just finalised the preceding section
                    }
                    let next_sect = &section_data[pointer..];
                    if next_sect.len() < SectionCommonHeader::SIZE {
                        warn!(
                            "TODO: not enough bytes to read section header - implement buffering"
                        );
                        self.parser.reset();
                        return;
                    }
                    let header = SectionCommonHeader::new(&next_sect[..SectionCommonHeader::SIZE]);
                    self.parser.start_section(ctx, &header, next_sect);
                } else {
                    // this packet is a continuation of an existing PSI section
                    self.parser.continue_section(ctx, pk_buf);
                }
            }
            None => {
                warn!("no payload present in PSI packet");
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::demultiplex;
    use crate::demultiplex::PacketFilter;
    use crate::packet::Packet;
    use hex_literal::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    pub struct NullFilterSwitch;
    impl PacketFilter for NullFilterSwitch {
        type Ctx = NullDemuxContext;
        fn consume(&mut self, _ctx: &mut Self::Ctx, _pk: &Packet<'_>) {
            unimplemented!()
        }
    }

    demux_context!(NullDemuxContext, NullFilterSwitch);
    impl NullDemuxContext {
        fn do_construct(&mut self, _req: demultiplex::FilterRequest<'_, '_>) -> NullFilterSwitch {
            unimplemented!()
        }
    }

    struct NullSectionProcessor;
    impl SectionProcessor for NullSectionProcessor {
        type Context = NullDemuxContext;
        fn start_section<'a>(
            &mut self,
            _ctx: &mut Self::Context,
            _header: &SectionCommonHeader,
            _section_data: &'a [u8],
        ) {
        }
        fn continue_section<'a>(&mut self, _ctx: &mut Self::Context, _section_data: &'a [u8]) {}
        fn reset(&mut self) {}
    }

    #[test]
    fn continuation_outside_section() {
        let mut buf = [0u8; 188];
        buf[0] = 0x47;
        buf[3] |= 0b00010000; // PayloadOnly
        let pk = Packet::new(&buf[..]);
        let mut psi_buf = SectionPacketConsumer::new(NullSectionProcessor);
        let mut ctx = NullDemuxContext::new();
        psi_buf.consume(&mut ctx, &pk);
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
        let mut ctx = NullDemuxContext::new();
        psi_buf.consume(&mut ctx, &pk);
    }

    #[test]
    fn section_spanning_packets() {
        // state to track if MockSectParse.section() got called,
        let state = Rc::new(RefCell::new(false));
        struct MockSectParse {
            state: Rc<RefCell<bool>>,
        }
        impl WholeSectionSyntaxPayloadParser for MockSectParse {
            type Context = ();
            fn section<'a>(
                &mut self,
                _: &mut Self::Context,
                _header: &SectionCommonHeader,
                _table_syntax_header: &TableSyntaxHeader<'_>,
                _data: &[u8],
            ) {
                *self.state.borrow_mut() = true;
            }
        }
        let mut p = BufferSectionSyntaxParser::new(CrcCheckWholeSectionSyntaxPayloadParser::new(
            MockSectParse {
                state: state.clone(),
            },
        ));
        let ctx = &mut ();
        {
            let sect = hex!(
                "
            42f13040 84e90000 233aff44 40ff8026
            480d1900 0a424243 2054574f 20484473
            0c66702e 6262632e 636f2e75 6b5f0400
            00233a7e 01f744c4 ff802148 09190006
            49545620 4844730b 7777772e 6974762e
            636f6d5f 04000023 3a7e01f7 4500ff80
            2c480f19 000c4368 616e6e65 6c203420
            48447310 7777772e 6368616e 6e656c34
            2e636f6d 5f040000 233a7e01 f74484ff
            8026480d 19000a42 4243204f 4e452048
            44730c66 702e6262 632e636f 2e756b5f
            04000023 3a7e01"
            );

            let common_header = SectionCommonHeader::new(&sect[..SectionCommonHeader::SIZE]);
            let table_header = TableSyntaxHeader::new(&sect[SectionCommonHeader::SIZE..]);
            p.start_syntax_section(ctx, &common_header, &table_header, &sect[..]);
        }
        {
            let sect = hex!(
                "
                f746c0ff 8023480a 19000743 42424320
                4844730c 66702e62 62632e63 6f2e756b
                5f040000 233a7e01 f74f80ff 801e480a
                16000746 696c6d34 2b317310 7777772e
                6368616e 6e656c34 2e636f6d 4540ff80
                27480f19 000c4368 616e6e65 6c203520
                4844730b 7777772e 66697665 2e74765f
                04000023 3a7e01f7 f28b26c4 ffffffff
                ffffffff ffffffff ffffffff ffffffff
                ffffffff ffffffff ffffffff ffffffff
                ffffffff ffffffff ffffffff ffffffff
                ffffffff ffffffff"
            );

            p.continue_syntax_section(ctx, &sect[..]);
        }

        assert!(*state.borrow());
    }

    #[test]
    fn table_syntax() {
        let sect = hex!("4084e90000");
        let header = TableSyntaxHeader::new(&sect);
        assert_eq!(header.current_next_indicator(), CurrentNext::Current);
        assert_eq!(header.id(), 16516);
        assert_eq!(header.section_number(), 0);
        assert_eq!(header.last_section_number(), 0);
        assert_eq!(header.version(), 20);
        // smoke test Debug impl (e.g. should not panic!)
        assert!(!format!("{:?}", header).is_empty());
    }

    #[test]
    fn dedup_section() {
        struct CallCounts {
            start: usize,
            cont: usize,
            reset: usize,
        }
        struct Mock {
            inner: Rc<RefCell<CallCounts>>,
        }
        let counts = Rc::new(RefCell::new(CallCounts {
            start: 0,
            cont: 0,
            reset: 0,
        }));
        impl SectionSyntaxPayloadParser for Mock {
            type Context = ();

            fn start_syntax_section<'a>(
                &mut self,
                _ctx: &mut Self::Context,
                _header: &SectionCommonHeader,
                _table_syntax_header: &TableSyntaxHeader<'a>,
                _data: &'a [u8],
            ) {
                self.inner.borrow_mut().start += 1;
            }

            fn continue_syntax_section<'a>(&mut self, _ctx: &mut Self::Context, _data: &'a [u8]) {
                self.inner.borrow_mut().cont += 1;
            }

            fn reset(&mut self) {
                self.inner.borrow_mut().reset += 1;
            }
        }
        let mut dedup = DedupSectionSyntaxPayloadParser::new(Mock {
            inner: counts.clone(),
        });

        let sect = hex!("42f130 4084e90000");

        let common_header = SectionCommonHeader::new(&sect[..SectionCommonHeader::SIZE]);
        let table_header = TableSyntaxHeader::new(&sect[SectionCommonHeader::SIZE..]);
        assert_eq!(table_header.version(), 20);

        let ctx = &mut ();
        dedup.start_syntax_section(ctx, &common_header, &table_header, &[]);
        dedup.continue_syntax_section(ctx, &[]);
        assert_eq!(counts.borrow().start, 1);
        assert_eq!(counts.borrow().cont, 1);
        // now we submit a table header with the same version,
        dedup.start_syntax_section(ctx, &common_header, &table_header, &[]);
        dedup.continue_syntax_section(ctx, &[]);
        // still 1
        assert_eq!(counts.borrow().start, 1);
        assert_eq!(counts.borrow().cont, 1);

        // now lets use the same section header as above but with an updated version
        let sect = hex!("42f131 4084ea0000");

        let common_header = SectionCommonHeader::new(&sect[..SectionCommonHeader::SIZE]);
        let table_header = TableSyntaxHeader::new(&sect[SectionCommonHeader::SIZE..]);
        assert_eq!(table_header.version(), 21);

        dedup.start_syntax_section(ctx, &common_header, &table_header, &[]);
        dedup.continue_syntax_section(ctx, &[]);
        // now 2
        assert_eq!(counts.borrow().start, 2);
        assert_eq!(counts.borrow().cont, 2);

        // if we now reset, then the deduplication should no longer be in effect and the submission
        // of the same section version again should now be passed through to our callback

        assert_eq!(counts.borrow().reset, 0);
        dedup.reset();
        assert_eq!(counts.borrow().reset, 1);

        // now 3
        dedup.start_syntax_section(ctx, &common_header, &table_header, &[]);
        dedup.continue_syntax_section(ctx, &[]);
        assert_eq!(counts.borrow().start, 3);
        assert_eq!(counts.borrow().cont, 3);
    }

    #[test]
    fn compact_section_syntax() {
        struct CallCounts {
            start: usize,
        }
        struct Mock {
            inner: Rc<RefCell<CallCounts>>,
        }
        impl CompactSyntaxPayloadParser for Mock {
            type Context = ();

            fn start_compact_section<'a>(
                &mut self,
                _ctx: &mut Self::Context,
                _header: &SectionCommonHeader,
                _data: &'a [u8],
            ) {
                self.inner.borrow_mut().start += 1;
            }

            fn continue_compact_section<'a>(&mut self, _ctx: &mut Self::Context, _data: &'a [u8]) {
                todo!()
            }

            fn reset(&mut self) {
                todo!()
            }
        }
        let counts = Rc::new(RefCell::new(CallCounts { start: 0 }));
        let mut proc = CompactSyntaxSectionProcessor::new(Mock {
            inner: counts.clone(),
        });

        let ctx = &mut ();

        // section_syntax_indicator is 0 in the table header, so this still not be passed through
        // to the mock
        let sect = hex!("42f131");
        let common_header = SectionCommonHeader::new(&sect[..SectionCommonHeader::SIZE]);
        assert!(common_header.section_syntax_indicator);
        proc.start_section(ctx, &common_header, &sect);
        assert_eq!(0, counts.borrow().start);

        let sect = hex!("427131");
        let common_header = SectionCommonHeader::new(&sect[..SectionCommonHeader::SIZE]);
        assert!(!common_header.section_syntax_indicator);
        // we trim the data slice down to 2 bytes which should cause the length check inside
        // CompactSyntaxSectionProcessor to fail,
        proc.start_section(ctx, &common_header, &sect[..2]);
        assert_eq!(0, counts.borrow().start);

        // section_length of 1022 (0x3fe) in this header is too long
        let header = hex!("4273fe");
        let mut sect = vec![];
        sect.extend_from_slice(&header);
        sect.resize(header.len() + 1022, 0); // fill remainder with zeros so we can accidentally fail because the buffer is too short
        let common_header = SectionCommonHeader::new(&sect[..SectionCommonHeader::SIZE]);
        assert!(!common_header.section_syntax_indicator);
        // we trim the data slice down to 2 bytes which should cause the length check inside
        // CompactSyntaxSectionProcessor to fail,
        proc.start_section(ctx, &common_header, &sect);
        assert_eq!(0, counts.borrow().start);

        // not too long, so this should now be accepted and we should see counts.start increment
        let sect = hex!("427000");
        let common_header = SectionCommonHeader::new(&sect[..SectionCommonHeader::SIZE]);
        assert!(!common_header.section_syntax_indicator);
        proc.start_section(ctx, &common_header, &sect);
        assert_eq!(1, counts.borrow().start);
    }

    #[test]
    fn buffer_compact() {
        const SECT: [u8; 7] = hex!("427003 01020304");
        struct Mock {
            section_count: usize,
        }
        impl WholeCompactSyntaxPayloadParser for Mock {
            type Context = ();

            fn section<'a>(
                &mut self,
                _: &mut Self::Context,
                header: &SectionCommonHeader,
                data: &'a [u8],
            ) {
                assert_eq!(0x42, header.table_id);
                // trim of the last byte which we've deliberately supplied, but which is not
                // supposed to be part of the section
                assert_eq!(data, &SECT[0..SECT.len() - 1]);
                self.section_count += 1;
            }
        }
        let mock = Mock { section_count: 0 };
        let mut parser = BufferCompactSyntaxParser::new(mock);
        let ctx = &mut ();

        let common_header = SectionCommonHeader::new(&SECT[..SectionCommonHeader::SIZE]);

        // we supply the inital section data, but then reset the BufferCompactSyntaxParser. this
        // should nave no ill effect on the second attempt where we supply the complete data,
        parser.start_compact_section(ctx, &common_header, &SECT[..SECT.len() - 3]);
        parser.reset();
        assert_eq!(0, parser.parser.section_count);

        parser.start_compact_section(ctx, &common_header, &SECT[..SECT.len() - 3]);
        parser.continue_compact_section(ctx, &SECT[SECT.len() - 3..SECT.len() - 2]);
        // this call will deliver 1 byte more than the 2 specified for our section_length, and
        // that second byte should be ignored
        parser.continue_compact_section(ctx, &SECT[SECT.len() - 2..]);
        assert_eq!(1, parser.parser.section_count);

        // the section is already complete, so BufferCompactSyntaxParser should drop any further
        // data supplied in error
        parser.continue_compact_section(ctx, &SECT[SECT.len() - 2..]);
        assert_eq!(1, parser.parser.section_count);
    }
}
