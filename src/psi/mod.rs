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

pub mod pat;
pub mod pmt;

use hexdump;
use mpegts_crc;
use packet;

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
    fn continue_section<'a>(&mut self, ctx: &mut Self::Context, section_data: &'a [u8]);
    fn reset(&mut self);
}

#[derive(Debug, PartialEq)]
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
    buf: &'buf [u8],
}

impl<'buf> TableSyntaxHeader<'buf> {
    pub const SIZE: usize = 5;

    pub fn new(buf: &'buf [u8]) -> TableSyntaxHeader {
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
        table_syntax_header: &TableSyntaxHeader,
        data: &'a [u8],
    ) {
        assert!(header.section_syntax_indicator);
        // don't apply CRC checks when fuzzing, to give more chances of test data triggering
        // parser bugs,
        if !cfg!(fuzz) && mpegts_crc::sum32(data) != 0 {
            warn!("section crc check failed for table_id {}", header.table_id,);
            hexdump::hexdump(data);
            return;
        }
        self.inner.section(ctx, header, table_syntax_header, data);
    }
}

pub trait WholeSectionSyntaxPayloadParser {
    type Context;

    fn section<'a>(
        &mut self,
        &mut Self::Context,
        header: &SectionCommonHeader,
        table_syntax_header: &TableSyntaxHeader,
        data: &'a [u8],
    );
}

enum BufferSectionState {
    Buffering(usize),
    Complete,
}

/// Implements `BufferSectionSyntaxParser` so that any sections that cross TS-packet boundaries
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
        table_syntax_header: &TableSyntaxHeader,
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
        table_syntax_header: &TableSyntaxHeader,
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
    type Context;

    /// NB the `data` buffer passed to _will_ include the bytes which are represented by `header`
    /// and `table_syntax_header` (in order that the called code can check any CRC that covers the
    /// whole section).
    fn start_syntax_section<'a>(
        &mut self,
        ctx: &mut Self::Context,
        header: &SectionCommonHeader,
        table_syntax_header: &TableSyntaxHeader,
        data: &'a [u8],
    );

    fn continue_syntax_section<'a>(&mut self, ctx: &mut Self::Context, data: &'a [u8]);

    fn reset(&mut self);
}

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
            section_syntax_indicator: buf[1] & 0b1000_0000 != 0,
            private_indicator: buf[1] & 0b0100_0000 != 0,
            section_length: ((u16::from(buf[1] & 0b0000_1111) << 8) | u16::from(buf[2])) as usize,
        }
    }
}

/// A `PacketConsumer` for buffering Program Specific Information, which may be split across
/// multiple TS packets, and passing a complete PSI table to the given `SectionProcessor` when a
/// complete, valid section has been received.
pub struct SectionPacketConsumer<P>
where
    P: SectionProcessor,
{
    parser: P,
}

impl<P, Ctx> SectionPacketConsumer<P>
where
    P: SectionProcessor<Context = Ctx>,
{
    pub fn new(parser: P) -> SectionPacketConsumer<P> {
        SectionPacketConsumer { parser }
    }

    pub fn consume(&mut self, ctx: &mut Ctx, pk: &packet::Packet) {
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
    use demultiplex;
    use packet::Packet;
    use std::cell::RefCell;
    use std::rc::Rc;

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
        let mut ctx = NullDemuxContext::new(NullStreamConstructor);
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
        let mut ctx = NullDemuxContext::new(NullStreamConstructor);
        psi_buf.consume(&mut ctx, &pk);
    }

    #[test]
    fn section_spanning_packets() {
        // state to track if MockSectParse.section() got called,
        let state = Rc::new(RefCell::new(false));
        struct MockSectParse {
            state: Rc<RefCell<bool>>,
        };
        impl WholeSectionSyntaxPayloadParser for MockSectParse {
            type Context = ();
            fn section<'a>(
                &mut self,
                _: &mut Self::Context,
                _header: &SectionCommonHeader,
                _table_syntax_header: &TableSyntaxHeader,
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
}
