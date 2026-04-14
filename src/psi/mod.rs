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
pub mod tsdt;

use crate::error::{DemuxError, ErrorSink};
use crate::mpegts_crc;
use crate::packet;
use std::fmt;
use std::marker::PhantomData;

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
impl fmt::Debug for TableSyntaxHeader<'_> {
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
    pid: packet::Pid,
    inner: P,
}
impl<P> CrcCheckWholeSectionSyntaxPayloadParser<P>
where
    P: WholeSectionSyntaxPayloadParser,
{
    const CRC_SIZE: usize = 4;

    /// create a new CrcCheckWholeSectionSyntaxPayloadParser which wraps and delegates to the given
    /// `WholeSectionSyntaxPayloadParser` instance
    pub fn new(pid: packet::Pid, inner: P) -> CrcCheckWholeSectionSyntaxPayloadParser<P> {
        CrcCheckWholeSectionSyntaxPayloadParser { pid, inner }
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
            ctx.error(DemuxError::SectionTooSmallForCrc {
                pid: self.pid,
                table_id: header.table_id,
                actual: data.len(),
            });
            return;
        }
        // don't apply CRC checks when fuzzing, to give more chances of test data triggering
        // parser bugs,
        if !cfg!(fuzzing) && mpegts_crc::sum32(data) != 0 {
            ctx.error(DemuxError::CrcCheckFailed {
                pid: self.pid,
                table_id: header.table_id,
            });
            return;
        }
        self.inner.section(ctx, header, table_syntax_header, data);
    }

    fn reset(&mut self) {
        self.inner.reset();
    }
}

/// Trait for types that parse fully reconstructed PSI table sections (which requires the caller
/// to have buffered section data if it spanned multiple TS packets.
pub trait WholeSectionSyntaxPayloadParser {
    /// Type of the context object that will be passed to all methods.
    type Context: ErrorSink;

    /// Method that will receive a complete PSI table section, where the `data` parameter will
    /// be `header.section_length` bytes long
    fn section<'a>(
        &mut self,
        _: &mut Self::Context,
        header: &SectionCommonHeader,
        table_syntax_header: &TableSyntaxHeader<'a>,
        data: &'a [u8],
    );

    /// Called when any in-progress parser state should be discarded, typically because a
    /// transport stream discontinuity has been detected.  The default implementation does
    /// nothing.
    fn reset(&mut self) {}
}

/// Trait for types that parse fully reconstructed PSI table sections.
///
/// This requires the caller to have buffered section data if it spanned multiple TS packets;
/// the [`CompactSyntaxFramer`] type will perform such buffering.
pub trait WholeCompactSyntaxPayloadParser {
    /// Type of the context object that will be passed to all methods.
    type Context: ErrorSink;

    /// Method that will receive a complete PSI table section, where the `data` parameter will
    /// be `header.section_length` bytes long
    fn section(&mut self, _: &mut Self::Context, header: &SectionCommonHeader, data: &[u8]);

    /// Called when any in-progress parser state should be discarded, typically because a
    /// transport stream discontinuity has been detected.  The default implementation does
    /// nothing.
    fn reset(&mut self) {}
}

/// Generic upper bound on `section_length`; see ISO/IEC 13818-1 §2.4.4.11.
const SECTION_LIMIT: usize = 4093;

/// The byte value used for the stuffing bytes which follow the last section within a TS packet
/// payload, and which is also used as an out-of-band `table_id` marker meaning "no more sections
/// in this payload".
const STUFFING_BYTE: u8 = 0xff;

/// Internal framer state, shared between the section-syntax and compact-syntax framers.
#[derive(Debug)]
enum FramerState {
    /// Not currently inside a section.  A new section may start as soon as we see a non-0xff
    /// byte at the `table_id` position.
    Idle,
    /// Collecting bytes into `buf`, expecting `target` total bytes before delivery.  Initially
    /// `target` is `SectionCommonHeader::SIZE`; once the header has been parsed it is grown to
    /// the full section length.
    Collecting { target: usize },
    /// A previous section was rejected; the remaining `remaining` bytes of its body must still
    /// be discarded before a new section can be recognised.
    Ignoring { remaining: usize },
}

/// Trait abstracting the differences between section-syntax and compact-syntax framing.
/// Implementations are zero-sized marker types that provide validation and dispatch logic.
#[doc(hidden)]
pub trait SectionSyntax {
    type Parser;
    type Context: ErrorSink;

    /// Validate a parsed `SectionCommonHeader`.  Return `false` to reject the section
    /// (the framer will skip its body).
    fn validate_header(
        pid: packet::Pid,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
    ) -> bool;

    /// Deliver a complete section to the wrapped parser.
    fn deliver_section(
        parser: &mut Self::Parser,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        data: &[u8],
    );

    /// Forward a reset to the wrapped parser.
    fn reset_parser(parser: &mut Self::Parser);
}

#[doc(hidden)]
pub struct SectionSyntaxMode<P: WholeSectionSyntaxPayloadParser>(PhantomData<P>);

impl<P: WholeSectionSyntaxPayloadParser> SectionSyntax for SectionSyntaxMode<P> {
    type Parser = P;
    type Context = P::Context;

    fn validate_header(
        pid: packet::Pid,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
    ) -> bool {
        if !header.section_syntax_indicator {
            ctx.error(DemuxError::UnexpectedSectionSyntaxIndicator {
                pid,
                table_id: header.table_id,
            });
            return false;
        }
        if header.section_length > SECTION_LIMIT {
            ctx.error(DemuxError::PsiSectionTooLarge {
                pid,
                table_id: header.table_id,
                length: header.section_length,
                limit: SECTION_LIMIT,
            });
            return false;
        }
        if header.section_length < TableSyntaxHeader::SIZE {
            ctx.error(DemuxError::SectionDataTooShort {
                pid,
                table_id: header.table_id,
                actual: SectionCommonHeader::SIZE + header.section_length,
                minimum: SectionCommonHeader::SIZE + TableSyntaxHeader::SIZE,
            });
            return false;
        }
        true
    }

    fn deliver_section(
        parser: &mut P,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        data: &[u8],
    ) {
        let table_syntax_header = TableSyntaxHeader::new(&data[SectionCommonHeader::SIZE..]);
        parser.section(ctx, header, &table_syntax_header, data);
    }

    fn reset_parser(parser: &mut P) {
        parser.reset();
    }
}

#[doc(hidden)]
pub struct CompactSyntaxMode<P: WholeCompactSyntaxPayloadParser>(PhantomData<P>);

impl<P: WholeCompactSyntaxPayloadParser> SectionSyntax for CompactSyntaxMode<P> {
    type Parser = P;
    type Context = P::Context;

    fn validate_header(
        pid: packet::Pid,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
    ) -> bool {
        if header.section_syntax_indicator {
            ctx.error(DemuxError::UnexpectedSectionSyntaxIndicator {
                pid,
                table_id: header.table_id,
            });
            return false;
        }
        if header.section_length > SECTION_LIMIT {
            ctx.error(DemuxError::PsiSectionTooLarge {
                pid,
                table_id: header.table_id,
                length: header.section_length,
                limit: SECTION_LIMIT,
            });
            return false;
        }
        true
    }

    fn deliver_section(
        parser: &mut P,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        data: &[u8],
    ) {
        parser.section(ctx, header, data);
    }

    fn reset_parser(parser: &mut P) {
        parser.reset();
    }
}

/// Generic PSI section framer that reconstructs whole sections from a stream of transport
/// stream packets.
///
/// Sections that fit entirely within a single TS packet payload are delivered without copying
/// any data.  Sections that span multiple packets - or whose headers straddle a packet boundary -
/// are buffered internally.  Multiple sections packed back-to-back in the same packet payload
/// are supported, as is the ISO/IEC 13818-1 convention that a `table_id` of `0xff` in the
/// between-sections position marks the remainder of the payload as stuffing.
///
/// This type is not used directly; see [`SectionSyntaxFramer`] and [`CompactSyntaxFramer`].
pub struct Framer<S: SectionSyntax> {
    pid: packet::Pid,
    buf: Vec<u8>,
    state: FramerState,
    parser: S::Parser,
    synced: bool,
    _phantom: PhantomData<S>,
}

/// Converts a stream of transport stream packets carrying 'section syntax' PSI table sections
/// into a stream of whole sections, delivered to the wrapped
/// [`WholeSectionSyntaxPayloadParser`] implementation.
pub type SectionSyntaxFramer<P> = Framer<SectionSyntaxMode<P>>;

/// Converts a stream of transport stream packets carrying 'compact syntax' PSI table sections
/// into a stream of whole sections, delivered to the wrapped
/// [`WholeCompactSyntaxPayloadParser`] implementation.
///
/// See [`SectionSyntaxFramer`] for a description of the buffering behaviour; the compact variant
/// differs only in that there is no `TableSyntaxHeader` to parse.
pub type CompactSyntaxFramer<P> = Framer<CompactSyntaxMode<P>>;

impl<S: SectionSyntax> Framer<S> {
    /// Wraps the given parser in a new framer for the given PID.
    pub fn new(pid: packet::Pid, parser: S::Parser) -> Framer<S> {
        Framer {
            pid,
            buf: Vec::with_capacity(SectionCommonHeader::SIZE + SECTION_LIMIT),
            state: FramerState::Idle,
            parser,
            synced: false,
            _phantom: PhantomData,
        }
    }

    /// Process the given transport stream packet, invoking the underlying parser's `section()`
    /// method for each complete PSI section found within the packet (possibly together with
    /// data buffered from earlier packets).
    pub fn consume(&mut self, ctx: &mut S::Context, pk: &packet::Packet<'_>) {
        match pk.payload() {
            Ok(Some(pk_buf)) => {
                if pk.payload_unit_start_indicator() {
                    if pk_buf.is_empty() {
                        ctx.error(DemuxError::SectionHeaderTooShort { pid: self.pid });
                        self.reset();
                        return;
                    }
                    let pointer = pk_buf[0] as usize;
                    let section_data = &pk_buf[1..];
                    if pointer > section_data.len() {
                        ctx.error(DemuxError::PsiPointerOutOfBounds { pid: self.pid });
                        self.reset();
                        return;
                    }
                    if self.synced && pointer > 0 && !matches!(self.state, FramerState::Idle) {
                        // pre-pointer bytes belong to a section already in progress.
                        // If we have no section in progress (Idle), the pre-pointer
                        // bytes are stray data the upstream encoder should not have
                        // emitted - drop them silently rather than let feed() parse
                        // them as a fresh section start.
                        self.feed(ctx, &section_data[..pointer]);
                    }
                    // After the pre-pointer phase the framer must be Idle, otherwise the
                    // upstream section's reported length disagrees with pointer_field.
                    if !matches!(self.state, FramerState::Idle) {
                        ctx.error(DemuxError::ExtraDataAfterSectionComplete { pid: self.pid });
                        self.reset();
                    }
                    // We have now seen a PUSI=1 packet, so any subsequent pre-pointer bytes
                    // really do belong to a section we are tracking.
                    self.synced = true;
                    self.feed(ctx, &section_data[pointer..]);
                } else if self.synced {
                    if matches!(self.state, FramerState::Idle) {
                        ctx.error(DemuxError::ExtraDataAfterSectionComplete { pid: self.pid });
                        return;
                    }
                    self.feed(ctx, pk_buf);
                }
                // If !self.synced and !PUSI, drop the packet silently - it's continuation
                // data for a section that started before our capture began.
            }
            Ok(None) => {
                ctx.error(DemuxError::NoPayloadInPsiPacket { pid: self.pid });
            }
            Err(e) => {
                ctx.error(DemuxError::MalformedPayload {
                    pid: self.pid,
                    error: e,
                });
            }
        }
    }

    /// Discard any in-progress section data, e.g. after a continuity counter discontinuity.
    pub fn reset(&mut self) {
        // NB self.synced is intentionally preserved - reset() only discards in-flight
        // section bytes; the framer's view of "we have observed at least one PUSI=1 packet
        // on this PID" stays valid across resets.
        self.buf.clear();
        self.state = FramerState::Idle;
        S::reset_parser(&mut self.parser);
    }

    /// Core state machine: consume a contiguous slice of PSI bytes.
    fn feed(&mut self, ctx: &mut S::Context, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            match self.state {
                FramerState::Idle => {
                    // Stuffing at the start-of-section position marks the end of usable data
                    // in this payload.
                    if bytes[0] == STUFFING_BYTE {
                        return;
                    }
                    if bytes.len() >= SectionCommonHeader::SIZE {
                        let header = SectionCommonHeader::new(&bytes[..SectionCommonHeader::SIZE]);
                        let total = SectionCommonHeader::SIZE + header.section_length;
                        if !S::validate_header(self.pid, ctx, &header) {
                            // skip the remainder of the (rejected) section body
                            let skip = total.min(bytes.len());
                            bytes = &bytes[skip..];
                            if total > skip {
                                self.state = FramerState::Ignoring {
                                    remaining: total - skip,
                                };
                            }
                            continue;
                        }
                        if bytes.len() >= total {
                            // zero-copy fast path: whole section is in the caller's buffer
                            let (section, rest) = bytes.split_at(total);
                            S::deliver_section(&mut self.parser, ctx, &header, section);
                            bytes = rest;
                        } else {
                            // partial section - buffer what we have and wait for more
                            self.buf.clear();
                            self.buf.extend_from_slice(bytes);
                            self.state = FramerState::Collecting { target: total };
                            return;
                        }
                    } else {
                        // not enough bytes to parse even the common header - buffer them
                        self.buf.clear();
                        self.buf.extend_from_slice(bytes);
                        self.state = FramerState::Collecting {
                            target: SectionCommonHeader::SIZE,
                        };
                        return;
                    }
                }
                FramerState::Collecting { target } => {
                    let needed = target - self.buf.len();
                    let take = needed.min(bytes.len());
                    self.buf.extend_from_slice(&bytes[..take]);
                    bytes = &bytes[take..];

                    if self.buf.len() == target && target == SectionCommonHeader::SIZE {
                        // we were waiting for the common header; now parse it and promote
                        // target to the full section length
                        let header =
                            SectionCommonHeader::new(&self.buf[..SectionCommonHeader::SIZE]);
                        let full_len = SectionCommonHeader::SIZE + header.section_length;
                        if !S::validate_header(self.pid, ctx, &header) {
                            self.buf.clear();
                            self.state = FramerState::Ignoring {
                                remaining: header.section_length,
                            };
                            continue;
                        }
                        if full_len == SectionCommonHeader::SIZE {
                            // compact section with section_length==0: the header *is* the
                            // whole section, so deliver immediately rather than re-entering
                            // the Collecting arm (which would otherwise re-trigger this
                            // branch and loop forever).
                            S::deliver_section(&mut self.parser, ctx, &header, &self.buf[..]);
                            self.buf.clear();
                            self.state = FramerState::Idle;
                            continue;
                        }
                        self.state = FramerState::Collecting { target: full_len };
                        continue;
                    }

                    if self.buf.len() == target {
                        // we have a whole section buffered
                        let header =
                            SectionCommonHeader::new(&self.buf[..SectionCommonHeader::SIZE]);
                        S::deliver_section(&mut self.parser, ctx, &header, &self.buf[..target]);
                        self.buf.clear();
                        self.state = FramerState::Idle;
                    }
                }
                FramerState::Ignoring { remaining } => {
                    let drop = remaining.min(bytes.len());
                    bytes = &bytes[drop..];
                    if drop == remaining {
                        self.state = FramerState::Idle;
                    } else {
                        self.state = FramerState::Ignoring {
                            remaining: remaining - drop,
                        };
                    }
                }
            }
        }
    }
}

/// A wrapper around some other implementation of `WholeSectionSyntaxPayloadParser` that
/// passes-through section data, unless the `TableSyntaxHeader` indicates a version_number which
/// is the same as the last data that was passed though.
///
/// This avoids the underlying code needing to re-parse duplicate copies of the section, which
/// are usually inserted periodically in the Transport Stream.
pub struct DedupSectionSyntaxPayloadParser<P>
where
    P: WholeSectionSyntaxPayloadParser,
{
    inner: P,
    last_version: Option<u8>,
}
impl<P> DedupSectionSyntaxPayloadParser<P>
where
    P: WholeSectionSyntaxPayloadParser,
{
    /// Wraps the given `WholeSectionSyntaxPayloadParser` in a new
    /// `DedupSectionSyntaxPayloadParser` instance.
    pub fn new(inner: P) -> DedupSectionSyntaxPayloadParser<P> {
        DedupSectionSyntaxPayloadParser {
            inner,
            last_version: None,
        }
    }
}
impl<P> WholeSectionSyntaxPayloadParser for DedupSectionSyntaxPayloadParser<P>
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
        let version = table_syntax_header.version();
        if self.last_version == Some(version) {
            return;
        }
        self.last_version = Some(version);
        self.inner.section(ctx, header, table_syntax_header, data);
    }

    fn reset(&mut self) {
        self.last_version = None;
        self.inner.reset();
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::packet::Packet;
    use hex_literal::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// `WholeSectionSyntaxPayloadParser` mock that discards sections, used as a no-op sink for
    /// smoke-tests that only care about whether the framer panics on a given input.
    struct NullSyntaxSink;
    impl WholeSectionSyntaxPayloadParser for NullSyntaxSink {
        type Context = ();
        fn section<'a>(
            &mut self,
            _ctx: &mut Self::Context,
            _header: &SectionCommonHeader,
            _table_syntax_header: &TableSyntaxHeader<'a>,
            _data: &'a [u8],
        ) {
        }
    }

    #[test]
    fn continuation_outside_section() {
        // A non-PUSI packet arriving when the framer is idle: no section data is in progress,
        // so the framer should warn and discard the bytes rather than panic.
        let mut buf = [0u8; 188];
        buf[0] = 0x47;
        buf[3] |= 0b00010000; // PayloadOnly
        let pk = Packet::new(&buf[..]);
        let mut framer = SectionSyntaxFramer::new(packet::Pid::new(0), NullSyntaxSink);
        framer.consume(&mut (), &pk);
    }

    #[test]
    fn small_section() {
        // A PUSI packet whose section_length field claims just 3 bytes - too short to contain
        // a TableSyntaxHeader.  The framer should reject it via validate_header() rather than
        // panic when trying to build the TableSyntaxHeader.
        let mut buf = [0u8; 188];
        buf[0] = 0x47;
        buf[1] |= 0b01000000; // payload_unit_start_indicator
        buf[3] |= 0b00010000; // PayloadOnly
                              // buf[4] = pointer_field = 0
                              // buf[5] = table_id = 0
        buf[6] = 0b1000_0000; // section_syntax_indicator = 1, section_length high = 0
        buf[7] = 3; // section_length low = 3
        let pk = Packet::new(&buf[..]);
        let mut framer = SectionSyntaxFramer::new(packet::Pid::new(0), NullSyntaxSink);
        framer.consume(&mut (), &pk);
    }

    struct MockWholeSectParse {
        state: Rc<RefCell<bool>>,
    }
    impl WholeSectionSyntaxPayloadParser for MockWholeSectParse {
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

    /// Build a 188-byte TS packet whose payload is `prefix` followed by `0xff` stuffing.
    /// Use this for packets that contain the *end* of a section (or are entirely stuffing),
    /// where the trailing 0xff bytes legitimately mean "no more data in this payload".
    ///
    /// Do NOT use this for packets carrying *mid-section* continuation data - for that, use
    /// [`build_full_packet`] which requires the payload to fill all 184 bytes, since the
    /// framer cannot distinguish between mid-section data bytes that happen to be 0xff and
    /// stuffing.
    fn build_packet(pusi: bool, pid: u16, prefix: &[u8]) -> [u8; 188] {
        assert!(prefix.len() <= 184);
        let mut buf = [0xffu8; 188];
        buf[0] = 0x47;
        buf[1] = ((pid >> 8) & 0x1f) as u8;
        if pusi {
            buf[1] |= 0b0100_0000;
        }
        buf[2] = (pid & 0xff) as u8;
        buf[3] = 0b0001_0000; // adaptation_field_control = payload only, CC = 0
        buf[4..4 + prefix.len()].copy_from_slice(prefix);
        buf
    }

    /// Build a 188-byte TS packet whose payload is exactly the 184 supplied bytes - no
    /// trailing stuffing.  Use this for packets that carry mid-section continuation data,
    /// where every byte of the payload is part of the in-progress section.
    fn build_full_packet(pusi: bool, pid: u16, payload: &[u8]) -> [u8; 188] {
        assert_eq!(payload.len(), 184);
        let mut buf = [0u8; 188];
        buf[0] = 0x47;
        buf[1] = ((pid >> 8) & 0x1f) as u8;
        if pusi {
            buf[1] |= 0b0100_0000;
        }
        buf[2] = (pid & 0xff) as u8;
        buf[3] = 0b0001_0000;
        buf[4..188].copy_from_slice(payload);
        buf
    }

    /// Build a minimal valid section-syntax section with `body_len` bytes of payload after the
    /// `TableSyntaxHeader`.  The body is filled with a repeating pattern, not a real table -
    /// the framer does not care about its contents.
    fn make_syntax_section(table_id: u8, body_len: usize) -> Vec<u8> {
        let section_length = TableSyntaxHeader::SIZE + body_len;
        assert!(section_length <= SECTION_LIMIT);
        let mut out = Vec::with_capacity(SectionCommonHeader::SIZE + section_length);
        out.push(table_id);
        out.push(0b1000_0000 | ((section_length >> 8) as u8 & 0x0f));
        out.push(section_length as u8);
        // id (2), reserved|version|current_next (1), section_number (1), last_section_number (1)
        out.extend_from_slice(&[0x12, 0x34, 0b1100_0001, 0x00, 0x00]);
        for i in 0..body_len {
            out.push(i as u8);
        }
        out
    }

    fn make_compact_section(table_id: u8, body_len: usize) -> Vec<u8> {
        assert!(body_len <= SECTION_LIMIT);
        let mut out = Vec::with_capacity(SectionCommonHeader::SIZE + body_len);
        out.push(table_id);
        out.push(((body_len >> 8) as u8) & 0x0f); // section_syntax_indicator = 0
        out.push(body_len as u8);
        for i in 0..body_len {
            out.push((i ^ 0x55) as u8);
        }
        out
    }

    /// `WholeSectionSyntaxPayloadParser` mock that records each delivered section's data.
    struct SyntaxSink {
        sections: Rc<RefCell<Vec<Vec<u8>>>>,
    }
    impl WholeSectionSyntaxPayloadParser for SyntaxSink {
        type Context = ();
        fn section<'a>(
            &mut self,
            _ctx: &mut Self::Context,
            _header: &SectionCommonHeader,
            _table_syntax_header: &TableSyntaxHeader<'a>,
            data: &'a [u8],
        ) {
            self.sections.borrow_mut().push(data.to_vec());
        }
    }

    /// `WholeCompactSyntaxPayloadParser` mock that records each delivered section's data.
    struct CompactSink {
        sections: Rc<RefCell<Vec<Vec<u8>>>>,
    }
    impl WholeCompactSyntaxPayloadParser for CompactSink {
        type Context = ();
        fn section(
            &mut self,
            _ctx: &mut Self::Context,
            _header: &SectionCommonHeader,
            data: &[u8],
        ) {
            self.sections.borrow_mut().push(data.to_vec());
        }
    }

    #[allow(clippy::type_complexity)]
    fn syntax_framer() -> (SectionSyntaxFramer<SyntaxSink>, Rc<RefCell<Vec<Vec<u8>>>>) {
        let sink_out = Rc::new(RefCell::new(vec![]));
        let framer = SectionSyntaxFramer::new(
            packet::Pid::new(0),
            SyntaxSink {
                sections: sink_out.clone(),
            },
        );
        (framer, sink_out)
    }

    #[allow(clippy::type_complexity)]
    fn compact_framer() -> (CompactSyntaxFramer<CompactSink>, Rc<RefCell<Vec<Vec<u8>>>>) {
        let sink_out = Rc::new(RefCell::new(vec![]));
        let framer = CompactSyntaxFramer::new(
            packet::Pid::new(0),
            CompactSink {
                sections: sink_out.clone(),
            },
        );
        (framer, sink_out)
    }

    #[test]
    fn section_spanning_packets() {
        // A section with a body longer than one TS packet payload will fit: the framer should
        // receive data across two packets and deliver exactly one whole section.  Packet 1 is
        // entirely full of section data (pointer_field + 183 section bytes); packet 2 carries
        // the remaining section bytes followed by trailing stuffing.
        let (mut framer, sink) = syntax_framer();
        let section = make_syntax_section(0x42, 300);
        let mut first = vec![0u8]; // pointer_field = 0
        first.extend_from_slice(&section[..183]);
        let pkt1 = build_full_packet(true, 0, &first);
        framer.consume(&mut (), &Packet::new(&pkt1));
        assert_eq!(sink.borrow().len(), 0); // not yet complete
        let pkt2 = build_packet(false, 0, &section[183..]);
        framer.consume(&mut (), &Packet::new(&pkt2));
        assert_eq!(sink.borrow().len(), 1);
        assert_eq!(&sink.borrow()[0], &section);
    }

    #[test]
    fn framer_zero_copy_single_packet() {
        let (mut framer, sink) = syntax_framer();
        let section = make_syntax_section(0x42, 16);
        let mut payload = vec![0u8];
        payload.extend_from_slice(&section);
        let pkt = build_packet(true, 0, &payload);
        framer.consume(&mut (), &Packet::new(&pkt));
        assert_eq!(sink.borrow().len(), 1);
        assert_eq!(&sink.borrow()[0], &section);
        // fast path: framer did not need to allocate into buf
        assert_eq!(framer.buf.len(), 0);
    }

    #[test]
    fn framer_section_spanning_three_packets() {
        let (mut framer, sink) = syntax_framer();
        let section = make_syntax_section(0x42, 500);
        let mut first = vec![0u8];
        first.extend_from_slice(&section[..183]);
        framer.consume(&mut (), &Packet::new(&build_full_packet(true, 0, &first)));
        framer.consume(
            &mut (),
            &Packet::new(&build_full_packet(false, 0, &section[183..183 + 184])),
        );
        assert_eq!(sink.borrow().len(), 0);
        framer.consume(
            &mut (),
            &Packet::new(&build_packet(false, 0, &section[183 + 184..])),
        );
        assert_eq!(sink.borrow().len(), 1);
        assert_eq!(&sink.borrow()[0], &section);
    }

    #[test]
    fn framer_header_split_after_one_byte() {
        // Pack section A so that exactly 1 byte of section B's 8-byte header fits at the end
        // of packet 1; the remaining 7 header bytes (plus B's body) arrive in packet 2.  This
        // models the natural way a header gets split across a TS packet boundary in a real
        // stream - there is no spec-legal way to send a packet whose payload is shorter than
        // 184 bytes without using an adaptation field.
        let (mut framer, sink) = syntax_framer();
        // packet 1 layout: pointer(1) + sec_a (8 + body_a) + sec_b[..1] = 184
        // → body_a = 184 - 1 - 8 - 1 = 174
        let sec_a = make_syntax_section(0x42, 174);
        let sec_b = make_syntax_section(0x43, 16);
        let mut first = vec![0u8];
        first.extend_from_slice(&sec_a);
        first.extend_from_slice(&sec_b[..1]);
        framer.consume(&mut (), &Packet::new(&build_full_packet(true, 0, &first)));
        assert_eq!(sink.borrow().len(), 1);
        framer.consume(&mut (), &Packet::new(&build_packet(false, 0, &sec_b[1..])));
        assert_eq!(sink.borrow().len(), 2);
        assert_eq!(&sink.borrow()[0], &sec_a);
        assert_eq!(&sink.borrow()[1], &sec_b);
    }

    #[test]
    fn framer_header_split_after_three_bytes() {
        // Pack section A so that section B's 3-byte SectionCommonHeader fits at the end of
        // packet 1, but its 5-byte TableSyntaxHeader is forced into packet 2.
        let (mut framer, sink) = syntax_framer();
        // body_a = 184 - 1 - 8 - 3 = 172
        let sec_a = make_syntax_section(0x42, 172);
        let sec_b = make_syntax_section(0x43, 16);
        let mut first = vec![0u8];
        first.extend_from_slice(&sec_a);
        first.extend_from_slice(&sec_b[..3]);
        framer.consume(&mut (), &Packet::new(&build_full_packet(true, 0, &first)));
        assert_eq!(sink.borrow().len(), 1);
        framer.consume(&mut (), &Packet::new(&build_packet(false, 0, &sec_b[3..])));
        assert_eq!(sink.borrow().len(), 2);
        assert_eq!(&sink.borrow()[0], &sec_a);
        assert_eq!(&sink.borrow()[1], &sec_b);
    }

    #[test]
    fn framer_multi_section_single_packet() {
        // Two small sections back-to-back in one packet, followed by stuffing.
        let (mut framer, sink) = syntax_framer();
        let sec_a = make_syntax_section(0x42, 8);
        let sec_b = make_syntax_section(0x43, 10);
        let mut payload = vec![0u8];
        payload.extend_from_slice(&sec_a);
        payload.extend_from_slice(&sec_b);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
        assert_eq!(sink.borrow().len(), 2);
        assert_eq!(&sink.borrow()[0], &sec_a);
        assert_eq!(&sink.borrow()[1], &sec_b);
    }

    #[test]
    fn framer_multi_section_with_spill() {
        // A short section A completely within one packet, followed by the
        // start of a longer section B whose tail lands in the next packet.
        let (mut framer, sink) = syntax_framer();
        let sec_a = make_syntax_section(0x42, 8);
        let sec_b = make_syntax_section(0x43, 300);
        let mut first = vec![0u8];
        first.extend_from_slice(&sec_a);
        let space_left = 184 - first.len();
        first.extend_from_slice(&sec_b[..space_left]);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &first)));
        assert_eq!(sink.borrow().len(), 1);
        framer.consume(
            &mut (),
            &Packet::new(&build_packet(false, 0, &sec_b[space_left..])),
        );
        assert_eq!(sink.borrow().len(), 2);
        assert_eq!(&sink.borrow()[0], &sec_a);
        assert_eq!(&sink.borrow()[1], &sec_b);
    }

    #[test]
    fn framer_stuffing_terminator_after_section() {
        // A section followed by trailing 0xff stuffing bytes in the same packet.
        let (mut framer, sink) = syntax_framer();
        let section = make_syntax_section(0x42, 8);
        let mut payload = vec![0u8];
        payload.extend_from_slice(&section);
        payload.extend_from_slice(&[0xff; 4]);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
        assert_eq!(sink.borrow().len(), 1);
        assert_eq!(&sink.borrow()[0], &section);
    }

    #[test]
    fn framer_stuffing_only() {
        // pointer_field=0 but the first byte is 0xff - no sections delivered, no warnings.
        let (mut framer, sink) = syntax_framer();
        let payload = vec![0u8, 0xff, 0xff, 0xff, 0xff];
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
        assert_eq!(sink.borrow().len(), 0);
    }

    #[test]
    fn framer_drops_pre_pointer_until_synced() {
        // A capture started mid-section: the very first PUSI=1 packet a framer ever sees
        // has pointer_field>0, and the pre-pointer bytes are the tail of a section that
        // started before the capture window.  The framer must discard them silently rather
        // than try to parse them as a fresh section header.
        let (mut framer, sink) = syntax_framer();
        let sec_b = make_syntax_section(0x42, 16);
        // 10 arbitrary bytes that look nothing like a valid section header,
        let pre_pointer = [0x33u8, 0x36, 0x38, 0x32, 0x30, 0x37, 0x1b, 0x58, 0x56, 0x36];
        let mut payload = vec![pre_pointer.len() as u8];
        payload.extend_from_slice(&pre_pointer);
        payload.extend_from_slice(&sec_b);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
        // first valid section was delivered, no warnings expected
        assert_eq!(sink.borrow().len(), 1);
        assert_eq!(&sink.borrow()[0], &sec_b);
    }

    #[test]
    fn framer_drops_continuation_packets_until_synced() {
        // The first packet a framer ever sees on its PID is a non-PUSI continuation packet -
        // we have no idea what section it belongs to, so it must be silently discarded.
        let (mut framer, sink) = syntax_framer();
        let payload = vec![0x12u8; 100]; // arbitrary garbage
        framer.consume(&mut (), &Packet::new(&build_packet(false, 0, &payload)));
        assert_eq!(sink.borrow().len(), 0);
        // and a subsequent valid PUSI=1 packet is now processed normally,
        let section = make_syntax_section(0x42, 16);
        let mut full = vec![0u8];
        full.extend_from_slice(&section);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &full)));
        assert_eq!(sink.borrow().len(), 1);
        assert_eq!(&sink.borrow()[0], &section);
    }

    #[test]
    fn framer_pointer_field_splits_sections() {
        // First packet starts section A but doesn't complete it.  Second packet is PUSI=1
        // with pointer_field pointing past the remainder of section A, then contains section B.
        let (mut framer, sink) = syntax_framer();
        let sec_a = make_syntax_section(0x42, 300);
        let mut first = vec![0u8];
        first.extend_from_slice(&sec_a[..183]);
        framer.consume(&mut (), &Packet::new(&build_full_packet(true, 0, &first)));
        let tail_len = sec_a.len() - 183;
        let sec_b = make_syntax_section(0x43, 10);
        let mut second = vec![tail_len as u8];
        second.extend_from_slice(&sec_a[183..]);
        second.extend_from_slice(&sec_b);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &second)));
        assert_eq!(sink.borrow().len(), 2);
        assert_eq!(&sink.borrow()[0], &sec_a);
        assert_eq!(&sink.borrow()[1], &sec_b);
    }

    #[test]
    fn framer_rejects_wrong_syntax_indicator() {
        // A section with section_syntax_indicator=0 must be rejected by the section-syntax framer.
        let (mut framer, sink) = syntax_framer();
        let mut section = make_syntax_section(0x42, 16);
        section[1] &= 0b0111_1111; // clear section_syntax_indicator
        let mut payload = vec![0u8];
        payload.extend_from_slice(&section);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
        assert_eq!(sink.borrow().len(), 0);
    }

    #[test]
    fn framer_rejects_oversize_section() {
        // Synthetic header claiming section_length=4094 (one over the spec limit).
        let (mut framer, sink) = syntax_framer();
        let payload = vec![0u8, 0x42, 0x8f, 0xfe];
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
        assert_eq!(sink.borrow().len(), 0);
    }

    #[test]
    fn framer_compact_single_packet() {
        let (mut framer, sink) = compact_framer();
        let section = make_compact_section(0x72, 16);
        let mut payload = vec![0u8];
        payload.extend_from_slice(&section);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
        assert_eq!(sink.borrow().len(), 1);
        assert_eq!(&sink.borrow()[0], &section);
    }

    #[test]
    fn framer_compact_spanning_packets() {
        let (mut framer, sink) = compact_framer();
        let section = make_compact_section(0x72, 300);
        let mut first = vec![0u8];
        first.extend_from_slice(&section[..183]);
        framer.consume(&mut (), &Packet::new(&build_full_packet(true, 0, &first)));
        framer.consume(
            &mut (),
            &Packet::new(&build_packet(false, 0, &section[183..])),
        );
        assert_eq!(sink.borrow().len(), 1);
        assert_eq!(&sink.borrow()[0], &section);
    }

    #[test]
    fn framer_compact_zero_length_header_split() {
        // Regression: a compact section with section_length=0 whose 3-byte
        // SectionCommonHeader was split across two packets used to cause an
        // infinite loop in Framer::feed's Collecting arm, because promoting
        // `target` to SectionCommonHeader::SIZE re-triggered the header-parse
        // branch on the next iteration.  Requires a short payload on packet
        // 1 to force the header to actually straddle a boundary, which means
        // using an adaptation field (a payload-only TS packet always carries
        // the full 184 bytes).
        let (mut framer, sink) = compact_framer();

        // Packet 1: adaptation_field_control=0b11, adaptation_field_length=180,
        // leaving a 3-byte payload that contains pointer_field + 2 of 3 header
        // bytes.
        let mut buf1 = [0u8; 188];
        buf1[0] = 0x47;
        buf1[1] = 0b0100_0000; // PUSI=1, PID=0
        buf1[2] = 0;
        buf1[3] = 0b0011_0000; // AF + payload, CC=0
        buf1[4] = 180; // adaptation_field_length
        buf1[5] = 0; // AF flags byte (no optional fields)
                     // bytes 6..185 are zero stuffing for the AF body; content_offset = 185
        buf1[185] = 0x00; // pointer_field
        buf1[186] = 0x72; // table_id (not stuffing)
        buf1[187] = 0x00; // section_syntax_indicator=0, section_length high nibble=0
        framer.consume(&mut (), &Packet::new(&buf1[..]));
        assert_eq!(sink.borrow().len(), 0);

        // Packet 2: continuation carrying the final header byte (section_length
        // low byte = 0), followed by 0xff stuffing so the framer stops cleanly
        // once the zero-length section has been delivered.
        let mut payload = [0xffu8; 184];
        payload[0] = 0x00;
        let pkt2 = build_full_packet(false, 0, &payload);
        framer.consume(&mut (), &Packet::new(&pkt2));

        // Exactly one zero-length compact section should have been delivered.
        assert_eq!(sink.borrow().len(), 1);
        assert_eq!(sink.borrow()[0], vec![0x72, 0x00, 0x00]);
    }

    #[test]
    fn framer_pre_pointer_ignored_when_idle() {
        // Regression: after a section had completed cleanly, the next PUSI=1
        // packet with pointer_field>0 would feed those stray bytes into an
        // Idle framer, which treated them as a fresh section start.  For a
        // compact section with section_length==0 this even produced a bogus
        // zero-length section delivery.
        let (mut framer, sink) = compact_framer();

        // Packet 1: synchronize and deliver a complete section, then trailing
        // stuffing leaves the framer Idle.
        let section_a = make_compact_section(0x70, 8);
        let mut payload1 = vec![0u8];
        payload1.extend_from_slice(&section_a);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload1)));
        assert_eq!(sink.borrow().len(), 1);

        // Packet 2: PUSI=1 with pointer_field=3.  The 3 pre-pointer bytes
        // form a syntactically valid compact section header (table_id=0x7e,
        // section_syntax_indicator=0, section_length=0).  The buggy code used
        // to deliver this as a fake section; the correct behaviour is to
        // silently discard the pre-pointer bytes because we have no section
        // in progress.
        let section_b = make_compact_section(0x71, 4);
        let mut payload2 = vec![3u8, 0x7e, 0x00, 0x00];
        payload2.extend_from_slice(&section_b);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload2)));

        // Only the two real sections must be delivered - no bogus 0x7e section.
        assert_eq!(sink.borrow().len(), 2);
        assert_eq!(sink.borrow()[0], section_a);
        assert_eq!(sink.borrow()[1], section_b);
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
    fn table_next_syntax() {
        let sect = hex!("4084e80000");
        let header = TableSyntaxHeader::new(&sect);
        assert_eq!(header.current_next_indicator(), CurrentNext::Next);
    }

    #[test]
    fn dedup_section() {
        struct CallCounts {
            section: usize,
            reset: usize,
        }
        struct Mock {
            inner: Rc<RefCell<CallCounts>>,
        }
        let counts = Rc::new(RefCell::new(CallCounts {
            section: 0,
            reset: 0,
        }));
        impl WholeSectionSyntaxPayloadParser for Mock {
            type Context = ();

            fn section<'a>(
                &mut self,
                _ctx: &mut Self::Context,
                _header: &SectionCommonHeader,
                _table_syntax_header: &TableSyntaxHeader<'a>,
                _data: &'a [u8],
            ) {
                self.inner.borrow_mut().section += 1;
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
        dedup.section(ctx, &common_header, &table_header, &sect);
        assert_eq!(counts.borrow().section, 1);
        // submit a section with the same version - should be suppressed,
        dedup.section(ctx, &common_header, &table_header, &sect);
        assert_eq!(counts.borrow().section, 1);

        // now use the same section header as above but with an updated version,
        let sect = hex!("42f131 4084ea0000");
        let common_header = SectionCommonHeader::new(&sect[..SectionCommonHeader::SIZE]);
        let table_header = TableSyntaxHeader::new(&sect[SectionCommonHeader::SIZE..]);
        assert_eq!(table_header.version(), 21);

        dedup.section(ctx, &common_header, &table_header, &sect);
        assert_eq!(counts.borrow().section, 2);

        // if we now reset, then the deduplication should no longer be in effect and the
        // submission of the same section version again should now be passed through,
        assert_eq!(counts.borrow().reset, 0);
        dedup.reset();
        assert_eq!(counts.borrow().reset, 1);

        dedup.section(ctx, &common_header, &table_header, &sect);
        assert_eq!(counts.borrow().section, 3);
    }

    #[test]
    fn framer_compact_validation_rejections() {
        // A section with section_syntax_indicator=1 must be rejected by the compact framer.
        let (mut framer, sink) = compact_framer();
        let mut payload = vec![0u8];
        payload.extend_from_slice(&hex!("42f131")); // section_syntax_indicator = 1
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
        assert_eq!(sink.borrow().len(), 0);

        // section_length of 4094 exceeds the generic 4093 limit - reject,
        let (mut framer, sink) = compact_framer();
        let payload = vec![0u8, 0x42, 0x0f, 0xfe];
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
        assert_eq!(sink.borrow().len(), 0);

        // A valid compact section with section_length=0 should be delivered,
        let (mut framer, sink) = compact_framer();
        let payload = vec![0u8, 0x42, 0x70, 0x00];
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
        assert_eq!(sink.borrow().len(), 1);
    }

    #[test]
    fn framer_compact_reset_mid_section() {
        // Feed the start of a multi-packet compact section, then reset() before the rest
        // arrives, verifying that reset() correctly discards the in-progress state and that
        // a fresh section can then be parsed normally.
        let (mut framer, sink) = compact_framer();
        let big = make_compact_section(0x70, 300);

        // partial delivery - section is incomplete after this packet,
        let mut partial = vec![0u8];
        partial.extend_from_slice(&big[..183]);
        framer.consume(&mut (), &Packet::new(&build_full_packet(true, 0, &partial)));
        assert_eq!(sink.borrow().len(), 0);
        framer.reset();
        assert_eq!(sink.borrow().len(), 0);

        // now feed a fresh complete section that fits entirely in one packet - should be
        // delivered cleanly despite the previous reset,
        let small = make_compact_section(0x71, 4);
        let mut full = vec![0u8];
        full.extend_from_slice(&small);
        framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &full)));
        assert_eq!(sink.borrow().len(), 1);
        assert_eq!(&sink.borrow()[0], &small);
    }

    #[test]
    fn framer_syntax_validation_rejections() {
        // A section with section_syntax_indicator=0 must be rejected by the section-syntax framer.
        {
            let (mut framer, sink) = syntax_framer();
            let mut section = make_syntax_section(0x42, 16);
            section[1] &= 0b0111_1111; // clear section_syntax_indicator
            let mut payload = vec![0u8];
            payload.extend_from_slice(&section);
            framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
            assert_eq!(sink.borrow().len(), 0);
        }

        // A section whose section_length is smaller than TableSyntaxHeader::SIZE (5) must be
        // rejected - there's not enough room for a table_syntax_header.
        {
            let (mut framer, sink) = syntax_framer();
            // table_id=0x42, section_syntax_indicator=1, section_length=4 (< 5)
            let payload = vec![0u8, 0x42, 0x80, 0x04];
            framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
            assert_eq!(sink.borrow().len(), 0);
        }

        // section_length > 4093 (the spec limit) must be rejected.
        {
            let (mut framer, sink) = syntax_framer();
            let payload = vec![0u8, 0x42, 0x8f, 0xfe];
            framer.consume(&mut (), &Packet::new(&build_packet(true, 0, &payload)));
            assert_eq!(sink.borrow().len(), 0);
        }
    }

    #[test]
    fn should_reject_section_too_small_for_crc() {
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
            04000023 3a7e01
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

        let mut sect_length_too_small = sect.to_vec();
        let bad_section_length: u16 = 11;
        assert_eq!(bad_section_length >> 8 & 0b1111_0000, 0);
        sect_length_too_small[1] =
            sect_length_too_small[1] & 0b1111_0000 | (bad_section_length >> 8) as u8;
        sect_length_too_small[2] = (bad_section_length & 0xff) as u8;
        sect_length_too_small.truncate(bad_section_length as usize);
        let state = Rc::new(RefCell::new(false));
        let mut crc_check = CrcCheckWholeSectionSyntaxPayloadParser::new(
            packet::Pid::new(0),
            MockWholeSectParse {
                state: state.clone(),
            },
        );
        let common_header =
            SectionCommonHeader::new(&sect_length_too_small[..SectionCommonHeader::SIZE]);
        let table_header =
            TableSyntaxHeader::new(&sect_length_too_small[SectionCommonHeader::SIZE..]);
        let ctx = &mut ();
        crc_check.section(ctx, &common_header, &table_header, &sect_length_too_small);
        assert!(!*state.borrow());
    }
}
