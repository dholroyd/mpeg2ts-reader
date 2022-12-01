//! Support for Packetised Elementary Stream syntax within Transport Stream packet payloads.
//! Elementary streams are split into 'elementary stream packets', which are then further split into
//! the payloads of transport stream packets.
//!
//! The types here allow the elementary stream to be reconstructed by passing all the extracted
//! pieces back to the calling application as they are encountered -- these pieces are not
//! reassembled, to avoid the cost of copying of the underlying data.
//!
//! To receive Elementary Stream data, create an implementation of
//! [`ElementaryStreamConsumer`](trait.ElementaryStreamConsumer.html), and register this with the
//! `StreamConstructor` object passed to the
//! [`Demultiplex`](../demultiplex/struct.Demultiplex.html) instance.

use crate::demultiplex;
use crate::packet;
use crate::packet::ClockRef;
use log::warn;
use std::marker;
use std::{fmt, num};

/// Trait for types that will receive call-backs as pieces of a specific elementary stream are
/// encounted within a transport stream.
///
/// Instances of this type are registered with a
/// [`PesPacketConsumer`](struct.PesPacketConsumer.html), which itself is responsible for
/// extracting elementary stream data from transport stream packets.
pub trait ElementaryStreamConsumer<Ctx> {
    /// called before the first call to `begin_packet()`
    fn start_stream(&mut self, ctx: &mut Ctx);

    /// called with the header at the start of the PES packet, which may not contain the complete
    /// PES payload
    fn begin_packet(&mut self, ctx: &mut Ctx, header: PesHeader<'_>);

    /// called after an earlier call to `begin_packet()` when another part of the packet's payload
    /// data is found in the Transport Stream.
    fn continue_packet(&mut self, ctx: &mut Ctx, data: &[u8]);

    /// called when a PES packet ends, prior to the next call to `begin_packet()` (if any).
    fn end_packet(&mut self, ctx: &mut Ctx);

    /// called when gap is seen in _continuity counter_ values for this stream, indicating that
    /// some data in the original Transport Stream did not reach the parser.
    fn continuity_error(&mut self, ctx: &mut Ctx);

    // TODO: end_stream() for symmetry?
}

#[derive(Debug, PartialEq)]
enum PesState {
    Begin,
    Started,
    IgnoreRest,
}

/// Implementation of [`demultiplex::PacketFilter`](../demultiplex/trait.PacketFilter.html) for
/// packets that contain PES data.
///
/// Delegates handling of PES data to an implementation of
/// [`ElementaryStreamConsumer`](trait.ElementaryStreamConsumer.html).
pub struct PesPacketFilter<Ctx, E>
where
    Ctx: demultiplex::DemuxContext,
    E: ElementaryStreamConsumer<Ctx>,
{
    stream_consumer: E,
    ccounter: Option<packet::ContinuityCounter>,
    state: PesState,
    phantom: marker::PhantomData<Ctx>,
}
impl<Ctx, E> PesPacketFilter<Ctx, E>
where
    Ctx: demultiplex::DemuxContext,
    E: ElementaryStreamConsumer<Ctx>,
{
    /// Construct a new `PesPacketFIlter` that will pass pieces of PES data to the given
    /// `ElementaryStreamConsumer`
    pub fn new(stream_consumer: E) -> PesPacketFilter<Ctx, E> {
        PesPacketFilter {
            stream_consumer,
            ccounter: None,
            state: PesState::Begin,
            phantom: marker::PhantomData,
        }
    }

    fn is_continuous(&self, packet: &packet::Packet<'_>) -> bool {
        if let Some(cc) = self.ccounter {
            // counter only increases if the packet has a payload,
            let result = if packet.adaptation_control().has_payload() {
                packet.continuity_counter().follows(cc)
            } else {
                packet.continuity_counter().count() == cc.count()
            };
            if !result {
                //warn!("discontinuity at packet with {:?} last={} this={} ({:?})", packet.pid(), cc.count(), packet.continuity_counter().count(), packet.adaptation_control());
            }
            result
        } else {
            true
        }
    }
}
impl<Ctx, E> demultiplex::PacketFilter for PesPacketFilter<Ctx, E>
where
    Ctx: demultiplex::DemuxContext,
    E: ElementaryStreamConsumer<Ctx>,
{
    type Ctx = Ctx;

    #[inline(always)]
    fn consume(&mut self, ctx: &mut Self::Ctx, packet: &packet::Packet<'_>) {
        if !self.is_continuous(packet) {
            self.stream_consumer.continuity_error(ctx);
            self.state = PesState::IgnoreRest;
        }
        self.ccounter = Some(packet.continuity_counter());
        if packet.payload_unit_start_indicator() {
            if self.state == PesState::Started {
                self.stream_consumer.end_packet(ctx);
            } else {
                // we might be in PesState::IgnoreRest, in which case we don't want to signal
                // a stream_start() to the consumer, which has already received stream_start()
                // and has presumably just been sent continuity_error() too,
                if self.state == PesState::Begin {
                    self.stream_consumer.start_stream(ctx);
                }
                self.state = PesState::Started;
            }
            if let Some(payload) = packet.payload() {
                if let Some(header) = PesHeader::from_bytes(payload) {
                    self.stream_consumer.begin_packet(ctx, header);
                }
            }
        } else {
            match self.state {
                PesState::Started => {
                    if let Some(payload) = packet.payload() {
                        if !payload.is_empty() {
                            self.stream_consumer.continue_packet(ctx, payload);
                        }
                    }
                }
                PesState::Begin => {
                    warn!("{:?}: Ignoring elementary stream content without a payload_start_indicator", packet.pid());
                }
                PesState::IgnoreRest => (),
            }
        }
    }
}

/// Type for the length of a PES packet
#[derive(Debug)]
pub enum PesLength {
    /// The PES packet continues until the next TS packet that has `payload_unit_start_indicator`
    /// set.  According to the spec, only valid for video streams (but really, it is needed in
    /// case the size of the pes packet will exceed the 16 bits of this field -- around 65k bytes).
    Unbounded,
    /// The PES packet's length (likely to exceed the size of the TS packet that contains the
    /// PES packet header).
    Bounded(num::NonZeroU16),
}

/// Values which may be returned by
/// [`PesHeader::stream_id()`](struct.PesHeader.html#method.stream_id) to identify the kind of
/// content within the Packetized Elementary Stream.
#[derive(Debug, PartialEq, Eq)]
pub enum StreamId {
    /// `program_stream_map`
    ProgramStreamMap,
    /// `private_stream_1`
    PrivateStream1,
    /// `padding_stream`
    PaddingStream,
    /// `private_stream_2`
    PrivateStream2,
    /// ISO/IEC 13818-3 or ISO/IEC 11172-3 or ISO/IEC 13818-7 or ISO/IEC 14496-3 audio stream
    Audio(u8),
    /// Rec. ITU-T H.262 | ISO/IEC 13818-2, ISO/IEC 11172-2, ISO/IEC 14496-2, Rec. ITU-T H.264 |
    /// ISO/IEC 14496-10 or Rec. ITU-T H.265 | ISO/IEC 23008-2 video stream
    Video(u8),
    /// `ECM_stream`
    EcmStream,
    /// `EMM_stream`
    EmmStream,
    /// Rec. ITU-T H.222.0 | ISO/IEC 13818-1 Annex B or ISO/IEC 13818-6_DSMCC_stream
    DsmCc,
    /// ISO/IEC_13522_stream
    Iso13522Stream,
    /// Rec. ITU-T H.222.1 type A
    H2221TypeA,
    /// Rec. ITU-T H.222.1 type B
    H2221TypeB,
    /// Rec. ITU-T H.222.1 type C
    H2221TypeC,
    /// Rec. ITU-T H.222.1 type D
    H2221TypeD,
    /// Rec. ITU-T H.222.1 type E
    H2221TypeE,
    /// `ancillary_stream`
    AncillaryStream,
    /// ISO/IEC 14496-1_SL-packetized_stream
    SlPacketizedStream,
    /// ISO/IEC 14496-1_FlexMux_stream
    FlexMuxStream,
    /// metadata stream
    MetadataStream,
    /// `extended_stream_id`
    ExtendedStreamId,
    /// reserved data stream
    ReservedDataStream,
    /// `program_stream_directory`
    ProgramStreamDirectory,
    /// Encapsulates a stream_id value not specified in _ISO/IEC 13818-1_
    Unknown(u8),
}
impl StreamId {
    fn is_parsed(&self) -> bool {
        !matches!(
            self,
            StreamId::ProgramStreamMap
                | StreamId::PaddingStream
                | StreamId::PrivateStream2
                | StreamId::EcmStream
                | StreamId::EmmStream
                | StreamId::ProgramStreamDirectory
                | StreamId::DsmCc
                | StreamId::H2221TypeE
        )
    }
}
impl From<u8> for StreamId {
    fn from(v: u8) -> Self {
        match v {
            0b1011_1100 => StreamId::ProgramStreamMap,
            0b1011_1101 => StreamId::PrivateStream1,
            0b1011_1110 => StreamId::PaddingStream,
            0b1011_1111 => StreamId::PrivateStream2,
            0b1100_0000..=0b1101_1111 => StreamId::Audio(v & 0b0001_1111),
            0b1110_0000..=0b1110_1111 => StreamId::Video(v & 0b0000_1111),
            0b1111_0000 => StreamId::EcmStream,
            0b1111_0001 => StreamId::EmmStream,
            0b1111_0010 => StreamId::DsmCc,
            0b1111_0011 => StreamId::Iso13522Stream,
            0b1111_0100 => StreamId::H2221TypeA,
            0b1111_0101 => StreamId::H2221TypeB,
            0b1111_0110 => StreamId::H2221TypeC,
            0b1111_0111 => StreamId::H2221TypeD,
            0b1111_1000 => StreamId::H2221TypeE,
            0b1111_1001 => StreamId::AncillaryStream,
            0b1111_1010 => StreamId::SlPacketizedStream,
            0b1111_1011 => StreamId::FlexMuxStream,
            0b1111_1100 => StreamId::MetadataStream,
            0b1111_1101 => StreamId::ExtendedStreamId,
            0b1111_1110 => StreamId::ReservedDataStream,
            0b1111_1111 => StreamId::ProgramStreamDirectory,
            _ => StreamId::Unknown(v),
        }
    }
}

/// Header at the start of every PES packet.
///
/// The header identifies,
///
///  * The stream identifier, returned by `stream_id()`
///  * The the size of the packet, returned by `pes_packet_length()`, which may well be larger than
///    the size of the payload buffer obtained from the header (the payload is likely split across
///    multiple Transport Stream packets)
///
/// In addition, the header may provide access to either
///
///  * an additional set of header data followed by a payload, when `contents()` returns `PesContents::Parsed`
///  * just a payload on its own, when `contents()` returns `PesContents::Payload`
pub struct PesHeader<'buf> {
    buf: &'buf [u8],
}
impl<'buf> PesHeader<'buf> {
    const FIXED_HEADER_SIZE: usize = 6;

    /// Wraps the given slice in a PesHeader, which will then provide method to parse the header
    /// fields within the slice.
    ///
    /// Returns `None` if the buffer is too small to hold the PES header, or if the PES
    /// 'start code prefix' is missing.
    ///
    /// TODO: should probably return `Result`.
    pub fn from_bytes(buf: &'buf [u8]) -> Option<PesHeader<'buf>> {
        // TODO: could the header straddle the boundary between TS packets?
        //       ..In which case we'd need to implement buffering.
        if buf.len() < Self::FIXED_HEADER_SIZE {
            warn!("Buffer size {} too small to hold PES header", buf.len());
            return None;
        }
        let packet_start_code_prefix =
            u32::from(buf[0]) << 16 | u32::from(buf[1]) << 8 | u32::from(buf[2]);
        if packet_start_code_prefix != 1 {
            warn!(
                "invalid packet_start_code_prefix 0x{:06x}, expected 0x000001",
                packet_start_code_prefix
            );
            return None;
        }
        Some(PesHeader { buf })
    }

    /// Indicator of the type of stream per _ISO/IEC 13818-1_, _Table 2-18_.
    pub fn stream_id(&self) -> StreamId {
        self.buf[3].into()
    }

    /// The overall length of the PES packet, once all pieces from the transport stream have
    /// been collected.
    pub fn pes_packet_length(&self) -> PesLength {
        let len = u16::from(self.buf[4]) << 8 | u16::from(self.buf[5]);
        match num::NonZeroU16::new(len) {
            None => PesLength::Unbounded,
            Some(l) => PesLength::Bounded(l),
        }
    }

    // maaaaaybe just have parsed_contents(&self) + payload(&self)

    /// Either `PesContents::Parsed`, or `PesContents::Payload`, depending on the value of
    /// `stream_id()`
    pub fn contents(&self) -> PesContents<'buf> {
        let rest = &self.buf[Self::FIXED_HEADER_SIZE..];
        if self.stream_id().is_parsed() {
            PesContents::Parsed(PesParsedContents::from_bytes(rest))
        } else {
            PesContents::Payload(rest)
        }
    }
}

/// Errors which may be encountered while processing PES data.
#[derive(Debug, PartialEq, Eq)]
pub enum PesError {
    /// The value of an optional field was requested, but the field is not actually present in the
    /// given PES data
    FieldNotPresent,
    /// The `pts_dts_flags` field of the PES packet signals that DTS is present and PTS is not,
    /// which not a valid combination
    PtsDtsFlagsInvalid,
    /// There is not enough data in the buffer to hold the expected syntax element
    NotEnoughData {
        /// the number of bytes required to hold the requested syntax element
        requested: usize,
        /// the number of bytes actually remaining in the buffer
        available: usize,
    },
    /// Marker bits are expected to always have the value `1` -- the value `0` presumably implies
    /// a parsing error.
    MarkerBitNotSet,
}

/// Either `PesContents::Payload`, when the `PesHeader` has no extra fields, or
/// `PesContents::Parsed`, when the header provides additional optional fields exposed in a
/// `ParsedPesContents` object.
///
/// # Example
///
/// To access initial portion of the payload of a PES packet, given the `PesHeader` (and ignoring
/// any fields that might be present in `ParsedPesContents`), you could
/// write,
///
/// ```rust
/// # use mpeg2ts_reader::pes::*;
/// # fn do_something(buf: &[u8]) {}
/// #
/// fn handle_payload(header: PesHeader) {
///     match header.contents() {
///         PesContents::Payload(buf) => do_something(buf),
///         PesContents::Parsed(Some(parsed)) => do_something(parsed.payload()),
///         _ => println!("error: couldn't access PES payload!"),
///     }
/// }
/// ```
pub enum PesContents<'buf> {
    /// payload with extra PES headers
    Parsed(Option<PesParsedContents<'buf>>),
    /// just payload without headers
    Payload(&'buf [u8]),
}

/// _Digital Storage Media_ 'trick mode' options.
///
/// Returned by
/// [PesParsedContents::dsm_trick_mode()](struct.PesParsedContents.html#method.dsm_trick_mode).
#[derive(Debug)]
#[allow(missing_docs)]
pub enum DsmTrickMode {
    FastForward {
        field_id: u8,
        intra_slice_refresh: bool,
        frequency_truncation: FrequencyTruncationCoefficientSelection,
    },
    SlowMotion {
        rep_cntrl: u8,
    },
    FreezeFrame {
        field_id: u8,
        reserved: u8,
    },
    FastReverse {
        field_id: u8,
        intra_slice_refresh: bool,
        frequency_truncation: FrequencyTruncationCoefficientSelection,
    },
    SlowReverse {
        rep_cntrl: u8,
    },
    Reserved {
        reserved: u8,
    },
}

/// Indication of the restricted set of coefficients that may have been used to encode the video,
/// specified within [`DsmTrickMode`](enum.DsmTrickMode.html).
#[derive(Debug)]
pub enum FrequencyTruncationCoefficientSelection {
    /// Only DC coefficients are non-zero
    DCNonZero,
    /// Only the first three coefficients are non-zero
    FirstThreeNonZero,
    /// Only the first six coefficients are non-zero
    FirstSixNonZero,
    /// All coefficients may be non-zero
    AllMaybeNonZero,
}
impl FrequencyTruncationCoefficientSelection {
    fn from_id(id: u8) -> Self {
        match id {
            0b00 => FrequencyTruncationCoefficientSelection::DCNonZero,
            0b01 => FrequencyTruncationCoefficientSelection::FirstThreeNonZero,
            0b10 => FrequencyTruncationCoefficientSelection::FirstSixNonZero,
            0b11 => FrequencyTruncationCoefficientSelection::AllMaybeNonZero,
            _ => panic!("Invalid id {}", id),
        }
    }
}

/// Indication of an Elementary Stream's data rate
#[derive(Debug)]
pub struct EsRate(u32);
impl EsRate {
    const RATE_BYTES_PER_SECOND: u32 = 50;
    /// panics if the given value is greater than _2^22 -1_.
    pub fn new(es_rate: u32) -> EsRate {
        assert!(es_rate < 1 << 22);
        EsRate(es_rate)
    }
    /// return the _es_rate_ value converted into a bytes-per-second quantity.
    pub fn bytes_per_second(&self) -> u32 {
        self.0 * Self::RATE_BYTES_PER_SECOND
    }
}
impl From<EsRate> for u32 {
    fn from(r: EsRate) -> Self {
        r.0
    }
}

/// TODO: not yet implemented
#[derive(Debug)] // TODO manual Debug
pub struct PesExtension<'buf> {
    _buf: &'buf [u8],
}

/// Extra data which may optionally be present in the `PesHeader`, potentially including
/// Presentation Timestamp (PTS) and Decode Timestamp (DTS) values.
pub struct PesParsedContents<'buf> {
    buf: &'buf [u8],
}
impl<'buf> PesParsedContents<'buf> {
    /// Wrap the given slice in a `ParsedPesContents` whose methods can parse the structure's
    /// fields
    ///
    /// Returns `None` if the buffer is too short to hold the expected structure, or if the
    /// 'check bit' values within the buffer do not have the expected values.
    ///
    /// TODO: return `Result`
    pub fn from_bytes(buf: &'buf [u8]) -> Option<PesParsedContents<'buf>> {
        if buf.len() < Self::FIXED_HEADER_SIZE {
            warn!(
                "buf not large enough to hold PES parsed header: {} bytes",
                buf.len()
            );
            return None;
        }
        let check_bits = buf[0] >> 6;
        if check_bits != 0b10 {
            warn!(
                "unexpected check-bits value {:#b}, expected 0b10",
                check_bits
            );
            return None;
        }
        let contents = PesParsedContents { buf };
        if (Self::FIXED_HEADER_SIZE + contents.pes_header_data_len()) > buf.len() {
            warn!(
                "reported PES header length {} does not fit within remaining buffer length {}",
                contents.pes_header_data_len(),
                buf.len() - Self::FIXED_HEADER_SIZE,
            );
            return None;
        }
        if contents.pes_crc_end() > (Self::FIXED_HEADER_SIZE + contents.pes_header_data_len()) {
            warn!(
                "calculated PES header data length {} does not fit with in recorded PES_header_length {}",
                contents.pes_crc_end() - Self::FIXED_HEADER_SIZE,
                contents.pes_header_data_len(),
            );
            return None;
        }
        Some(contents)
    }

    /// value 1 indicates higher priority and 0 indicates lower priority
    pub fn pes_priority(&self) -> u8 {
        self.buf[0] >> 3 & 1
    }
    /// if the returned value is `DataAlignment::Aligned`, then video or audio access units within
    /// this PES data are aligned to the start of PES packets.  If the value is
    /// `DataAlignment::NotAligned`, then alignment of access units is not defined.
    pub fn data_alignment_indicator(&self) -> DataAlignment {
        if self.buf[0] & 0b100 != 0 {
            DataAlignment::Aligned
        } else {
            DataAlignment::NotAligned
        }
    }
    /// Indicates copyright status of the material in this PES data.
    pub fn copyright(&self) -> Copyright {
        if self.buf[0] & 0b10 != 0 {
            Copyright::Undefined
        } else {
            Copyright::Protected
        }
    }
    /// Indicates the originality of the data in this PES stream.
    pub fn original_or_copy(&self) -> OriginalOrCopy {
        if self.buf[0] & 0b1 != 0 {
            OriginalOrCopy::Original
        } else {
            OriginalOrCopy::Copy
        }
    }
    fn pts_dts_flags(&self) -> u8 {
        self.buf[1] >> 6
    }
    fn escr_flag(&self) -> bool {
        self.buf[1] >> 5 & 1 != 0
    }
    fn esrate_flag(&self) -> bool {
        self.buf[1] >> 4 & 1 != 0
    }
    fn dsm_trick_mode_flag(&self) -> bool {
        self.buf[1] >> 3 & 1 != 0
    }
    fn additional_copy_info_flag(&self) -> bool {
        self.buf[1] >> 2 & 1 != 0
    }
    fn pes_crc_flag(&self) -> bool {
        self.buf[1] >> 1 & 1 != 0
    }
    fn pes_extension_flag(&self) -> bool {
        self.buf[1] & 1 != 0
    }
    fn pes_header_data_len(&self) -> usize {
        self.buf[2] as usize
    }

    fn header_slice(&self, from: usize, to: usize) -> Result<&'buf [u8], PesError> {
        if to > self.pes_header_data_len() + Self::FIXED_HEADER_SIZE {
            Err(PesError::NotEnoughData {
                requested: to,
                available: self.pes_header_data_len() + Self::FIXED_HEADER_SIZE,
            })
        } else if to > self.buf.len() {
            Err(PesError::NotEnoughData {
                requested: to,
                available: self.buf.len(),
            })
        } else {
            Ok(&self.buf[from..to])
        }
    }

    const FIXED_HEADER_SIZE: usize = 3;
    const TIMESTAMP_SIZE: usize = 5;

    fn pts_dts_end(&self) -> usize {
        match self.pts_dts_flags() {
            0b00 => Self::FIXED_HEADER_SIZE,
            0b01 => Self::FIXED_HEADER_SIZE,
            0b10 => Self::FIXED_HEADER_SIZE + Self::TIMESTAMP_SIZE,
            0b11 => Self::FIXED_HEADER_SIZE + Self::TIMESTAMP_SIZE * 2,
            v => panic!("unexpected value {}", v),
        }
    }
    /// Returns the timestamps present in this PES header.
    ///
    /// If no timestamp fields are present, then `Err(PesError::FieldNotPresent)` is produced.
    ///
    /// If the value of the _pts_dts_flags_ field in the header is invalid then
    /// `Err(PesError::PtsDtsFlagsInvalid)` is produced.
    pub fn pts_dts(&self) -> Result<PtsDts, PesError> {
        match self.pts_dts_flags() {
            0b00 => Err(PesError::FieldNotPresent),
            0b01 => Err(PesError::PtsDtsFlagsInvalid),
            0b10 => self
                .header_slice(Self::FIXED_HEADER_SIZE, self.pts_dts_end())
                .map(|s| PtsDts::PtsOnly(Timestamp::from_bytes(s))),
            0b11 => self
                .header_slice(Self::FIXED_HEADER_SIZE, self.pts_dts_end())
                .map(|s| PtsDts::Both {
                    pts: Timestamp::from_bytes(&s[..Self::TIMESTAMP_SIZE]),
                    dts: Timestamp::from_bytes(&s[Self::TIMESTAMP_SIZE..]),
                }),
            v => panic!("unexpected value {}", v),
        }
    }
    const ESCR_SIZE: usize = 6;

    /// Returns the 'Elementary Stream Clock Reference' value.
    ///
    /// If the field is not present in the header, then `Err(PesError::FieldNotPresent)` is
    /// produced.
    pub fn escr(&self) -> Result<ClockRef, PesError> {
        if self.escr_flag() {
            self.header_slice(self.pts_dts_end(), self.pts_dts_end() + Self::ESCR_SIZE)
                .map(|s| {
                    let base = u64::from(s[0] & 0b0011_1000) << 27
                        | u64::from(s[0] & 0b0000_0011) << 28
                        | u64::from(s[1]) << 20
                        | u64::from(s[2] & 0b1111_1000) << 12
                        | u64::from(s[2] & 0b0000_0011) << 13
                        | u64::from(s[3]) << 5
                        | u64::from(s[4] & 0b1111_1000) >> 3;
                    let extension =
                        u16::from(s[4] & 0b0000_0011) << 7 | u16::from(s[5] & 0b1111_1110) >> 1;
                    ClockRef::from_parts(base, extension)
                })
        } else {
            Err(PesError::FieldNotPresent)
        }
    }
    fn escr_end(&self) -> usize {
        self.pts_dts_end() + if self.escr_flag() { Self::ESCR_SIZE } else { 0 }
    }
    const ES_RATE_SIZE: usize = 3;

    /// Returns the 'Elementary Stream Rate' value.
    ///
    /// If the field is not present in the header, then `Err(PesError::FieldNotPresent)` is
    /// produced.
    pub fn es_rate(&self) -> Result<EsRate, PesError> {
        if self.esrate_flag() {
            self.header_slice(self.escr_end(), self.escr_end() + Self::ES_RATE_SIZE)
                .map(|s| {
                    EsRate::new(
                        u32::from(s[0] & 0b0111_1111) << 15
                            | u32::from(s[1]) << 7
                            | u32::from(s[2] & 0b1111_1110) >> 1,
                    )
                })
        } else {
            Err(PesError::FieldNotPresent)
        }
    }
    fn es_rate_end(&self) -> usize {
        self.escr_end()
            + if self.esrate_flag() {
                Self::ES_RATE_SIZE
            } else {
                0
            }
    }
    const DSM_TRICK_MODE_SIZE: usize = 1;
    /// Returns information about the 'Digital Storage Media trick mode'
    ///
    /// If the field is not present in the header, then `Err(PesError::FieldNotPresent)` is
    /// produced.
    pub fn dsm_trick_mode(&self) -> Result<DsmTrickMode, PesError> {
        if self.dsm_trick_mode_flag() {
            self.header_slice(
                self.es_rate_end(),
                self.es_rate_end() + Self::DSM_TRICK_MODE_SIZE,
            )
            .map(|s| {
                let trick_mode_control = s[0] >> 5;
                let trick_mode_data = s[0] & 0b0001_1111;
                match trick_mode_control {
                    0b000 => DsmTrickMode::FastForward {
                        field_id: trick_mode_data >> 3,
                        intra_slice_refresh: (trick_mode_data & 0b100) != 0,
                        frequency_truncation: FrequencyTruncationCoefficientSelection::from_id(
                            trick_mode_data & 0b11,
                        ),
                    },
                    0b001 => DsmTrickMode::SlowMotion {
                        rep_cntrl: trick_mode_control,
                    },
                    0b010 => DsmTrickMode::FreezeFrame {
                        field_id: trick_mode_data >> 3,
                        reserved: trick_mode_data & 0b111,
                    },
                    0b011 => DsmTrickMode::FastReverse {
                        field_id: trick_mode_data >> 3,
                        intra_slice_refresh: (trick_mode_data & 0b100) != 0,
                        frequency_truncation: FrequencyTruncationCoefficientSelection::from_id(
                            trick_mode_data & 0b11,
                        ),
                    },
                    0b100 => DsmTrickMode::SlowReverse {
                        rep_cntrl: trick_mode_control,
                    },
                    _ => DsmTrickMode::Reserved {
                        reserved: trick_mode_control,
                    },
                }
            })
        } else {
            Err(PesError::FieldNotPresent)
        }
    }
    fn dsm_trick_mode_end(&self) -> usize {
        self.es_rate_end()
            + if self.dsm_trick_mode_flag() {
                Self::DSM_TRICK_MODE_SIZE
            } else {
                0
            }
    }
    const ADDITIONAL_COPY_INFO_SIZE: usize = 1;
    /// Returns a 7-bit value containing private data relating to copyright information
    ///
    /// If the field is not present in the header, then `Err(PesError::FieldNotPresent)` is
    /// produced.
    pub fn additional_copy_info(&self) -> Result<u8, PesError> {
        if self.additional_copy_info_flag() {
            self.header_slice(
                self.dsm_trick_mode_end(),
                self.dsm_trick_mode_end() + Self::ADDITIONAL_COPY_INFO_SIZE,
            )
            .and_then(|s| {
                if s[0] & 0b1000_0000 == 0 {
                    Err(PesError::MarkerBitNotSet)
                } else {
                    Ok(s[0] & 0b0111_1111)
                }
            })
        } else {
            Err(PesError::FieldNotPresent)
        }
    }
    fn additional_copy_info_end(&self) -> usize {
        self.dsm_trick_mode_end()
            + if self.additional_copy_info_flag() {
                Self::ADDITIONAL_COPY_INFO_SIZE
            } else {
                0
            }
    }
    const PREVIOUS_PES_PACKET_CRC_SIZE: usize = 2;
    /// returns a 16-bit _Cyclic Redundancy Check_ value for the PES packet data bytes that
    /// precededed this one
    ///
    /// This crate does not perform any CRC-checks based on this value being present.
    ///
    /// If the field is not present in the header, then `Err(PesError::FieldNotPresent)` is
    /// produced.
    pub fn previous_pes_packet_crc(&self) -> Result<u16, PesError> {
        if self.pes_crc_flag() {
            self.header_slice(
                self.additional_copy_info_end(),
                self.additional_copy_info_end() + Self::PREVIOUS_PES_PACKET_CRC_SIZE,
            )
            .map(|s| u16::from(s[0]) << 8 | u16::from(s[1]))
        } else {
            Err(PesError::FieldNotPresent)
        }
    }
    fn pes_crc_end(&self) -> usize {
        self.additional_copy_info_end()
            + if self.pes_crc_flag() {
                Self::PREVIOUS_PES_PACKET_CRC_SIZE
            } else {
                0
            }
    }
    /// Returns the PES extension structure if present, or `Err(PesError::FieldNotPresent)` if not.
    pub fn pes_extension(&self) -> Result<PesExtension<'buf>, PesError> {
        if self.pes_extension_flag() {
            self.header_slice(
                self.pes_crc_end(),
                self.pes_header_data_len() + Self::FIXED_HEADER_SIZE,
            )
            .map(|s| PesExtension { _buf: s })
        } else {
            Err(PesError::FieldNotPresent)
        }
    }

    // TODO: consider dropping payload() data access from header; uniformly providing payload
    //       data through ElementaryStreamConsumer::continue_packet() callbacks or similar

    /// Provides the portion of the PES payload that was in the same TS packet as the PES header
    ///
    /// **NB** the full PES payload is likely to be split across multiple TS packets, and
    /// implementors of `ElementaryStreamConsumer` must be prepared to accept the PES payload data
    /// split between the header and multiple subsequent calls to
    /// `ElementaryStreamConsumer::continue_packet()`
    pub fn payload(&self) -> &'buf [u8] {
        &self.buf[Self::FIXED_HEADER_SIZE + self.pes_header_data_len()..]
    }
}

impl<'buf> fmt::Debug for PesParsedContents<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let mut s = f.debug_struct("PesParsedContents");
        s.field("pes_priority", &self.pes_priority())
            .field("data_alignment_indicator", &self.data_alignment_indicator())
            .field("copyright", &self.copyright())
            .field("original_or_copy", &self.original_or_copy())
            .field("pts_dts", &self.pts_dts());
        if let Ok(escr) = self.escr() {
            s.field("escr", &escr);
        }
        if let Ok(es_rate) = self.es_rate() {
            s.field("es_rate", &es_rate);
        }
        if let Ok(dsm_trick_mode) = self.dsm_trick_mode() {
            s.field("dsm_trick_mode", &dsm_trick_mode);
        }
        if let Ok(additional_copy_info) = self.additional_copy_info() {
            s.field("additional_copy_info", &additional_copy_info);
        }
        if let Ok(previous_pes_packet_crc) = self.previous_pes_packet_crc() {
            s.field("previous_pes_packet_crc", &previous_pes_packet_crc);
        }
        if let Ok(pes_extension) = self.pes_extension() {
            s.field("pes_extension", &pes_extension);
        }
        s.finish()
    }
}

/// Detail about the formatting problem which prevented a [`Timestamp`](struct.Timestamp.html)
/// value being parsed.
#[derive(PartialEq, Eq, Debug)]
pub enum TimestampError {
    /// Parsing the timestamp failed because the 'prefix-bit' values within the timestamp did not
    /// have the expected values
    IncorrectPrefixBits {
        /// expected prefix-bits for this timestamp
        expected: u8,
        /// the actual, incorrect bits that were present
        actual: u8,
    },
    /// Parsing the timestamp failed because a 'marker-bit' value within the timestamp did not
    /// have the expected value
    MarkerBitNotSet {
        /// the bit-index of the bit which should have been 1, but was found to be 0
        bit_number: u8,
    },
}

/// A 33-bit Elementary Stream timestamp, used to represent PTS and DTS values which may appear in
/// an Elementary Stream header.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub struct Timestamp {
    val: u64,
}
impl Timestamp {
    /// The largest representable timestamp value before the timestamp wraps back around to zero.
    pub const MAX: Timestamp = Timestamp { val: (1 << 33) - 1 };

    /// 90kHz timebase in which PTS and DTS values are measured.
    pub const TIMEBASE: u64 = 90_000;

    /// Parse a Presentation Time Stamp value from the 5 bytes at the start of the given slice
    ///
    /// Panics if fewer than 5 bytes given
    pub fn from_pts_bytes(buf: &[u8]) -> Result<Timestamp, TimestampError> {
        Timestamp::check_prefix(buf, 0b0010)?;
        Timestamp::from_bytes(buf)
    }
    /// Parse a Decode Time Stamp value from the 5 bytes at the start of the given slice
    ///
    /// Panics if fewer than 5 bytes given
    pub fn from_dts_bytes(buf: &[u8]) -> Result<Timestamp, TimestampError> {
        Timestamp::check_prefix(buf, 0b0001)?;
        Timestamp::from_bytes(buf)
    }
    fn check_prefix(buf: &[u8], expected: u8) -> Result<(), TimestampError> {
        assert!(expected <= 0b1111);
        let actual = buf[0] >> 4;
        if actual == expected {
            Ok(())
        } else {
            Err(TimestampError::IncorrectPrefixBits { expected, actual })
        }
    }
    fn check_marker_bit(buf: &[u8], bit_number: u8) -> Result<(), TimestampError> {
        let byte_index = bit_number / 8;
        let bit_index = bit_number % 8;
        let bit_mask = 1 << (7 - bit_index);
        if buf[byte_index as usize] & bit_mask != 0 {
            Ok(())
        } else {
            Err(TimestampError::MarkerBitNotSet { bit_number })
        }
    }
    /// Parse a Time Stamp value from the 5 bytes at the start of the given slice, without checking
    /// the 4-bit prefix for any particular value (the `from_pts_bytes()` and `from_dts_bytes()`
    /// methods, in contrast, do check that the expected prefix bits are present).
    ///
    /// Panics if fewer than 5 bytes given
    pub fn from_bytes(buf: &[u8]) -> Result<Timestamp, TimestampError> {
        Timestamp::check_marker_bit(buf, 7)?;
        Timestamp::check_marker_bit(buf, 23)?;
        Timestamp::check_marker_bit(buf, 39)?;
        Ok(Timestamp {
            val: (u64::from(buf[0] & 0b0000_1110) << 29)
                | u64::from(buf[1]) << 22
                | (u64::from(buf[2] & 0b1111_1110) << 14)
                | u64::from(buf[3]) << 7
                | u64::from(buf[4]) >> 1,
        })
    }
    /// Panics if the given val is greater than 2^33-1
    pub fn from_u64(val: u64) -> Timestamp {
        assert!(val < 1 << 34);
        Timestamp { val }
    }
    /// produces the timestamp's value (only the low 33 bits are used)
    pub fn value(self) -> u64 {
        self.val
    }

    /// returns true if timestamps are likely to have wrapped around since `other`, given a curent
    /// timestamp of `self`, and given the two timestamp values were taken no more than about
    /// _13.3 hours_ apart (i.e. no more than half the 26.5-ish hours it takes for the wrap around
    /// to occur).
    pub fn likely_wrapped_since(self, other: Self) -> bool {
        other.val > self.val && other.val - self.val > Self::MAX.val / 2
    }
}

/// Contains some combination of PTS and DTS timestamps (or maybe neither).
///
/// The timestamps will be wrapped in `Result`, in case an error in the stored timestamp syntax
/// means that it can't be decoded.
#[derive(PartialEq, Eq, Debug)]
pub enum PtsDts {
    /// There are no timestamps present
    None,
    /// Only Presentation Time Stamp is present
    PtsOnly(Result<Timestamp, TimestampError>),
    /// the _pts_dts_flags_ field contained an invalid value
    Invalid,
    /// Both Presentation and Decode Time Stamps are present
    Both {
        /// Presentation Time Stamp
        pts: Result<Timestamp, TimestampError>,
        /// Decode Time Stamp
        dts: Result<Timestamp, TimestampError>,
    },
}

/// Indicates if the start of some 'unit' of Elementary Stream content is immediately at the start of the PES
/// packet payload.
///
/// Returned by
/// [`PesParsedContents.data_alignment_indicator()`](struct.PesParsedContents.html#method.data_alignment_indicator)
#[derive(PartialEq, Eq, Debug)]
pub enum DataAlignment {
    /// Access Units are aligned to the start of the PES packet payload
    Aligned,
    /// Access Units are might not be aligned to the start of the PES packet payload
    NotAligned,
}
/// Indicates the copyright status of the contents of the Elementary Stream packet.
///
/// Returned by [`PesParsedContents.copyright()`](struct.PesParsedContents.html#method.copyright)
#[derive(PartialEq, Eq, Debug)]
pub enum Copyright {
    /// Content of this Elementry Stream is protected by copyright
    Protected,
    /// Copyright protection of the content of this Elementary Stream is not defined
    Undefined,
}
/// Indicates weather the contents of the Elementary Stream packet are original or a copy.
///
/// Returned by
/// [`PesParsedContents.original_or_copy()`](struct.PesParsedContents.html#method.original_or_copy)
#[derive(PartialEq, Eq, Debug)]
pub enum OriginalOrCopy {
    /// The Elementary Stream is original content
    Original,
    /// The Elementary Stream is a copy
    Copy,
}

#[cfg(test)]
mod test {
    use crate::demultiplex;
    use crate::demultiplex::PacketFilter;
    use crate::packet;
    use crate::pes;
    use assert_matches::assert_matches;
    use bitstream_io::{BigEndian, BitWrite};
    use bitstream_io::{BitWriter, BE};
    use hex_literal::*;
    use std::io;

    packet_filter_switch! {
        NullFilterSwitch<NullDemuxContext> {
            Nul: demultiplex::NullPacketFilter<NullDemuxContext>,
        }
    }
    demux_context!(NullDemuxContext, NullFilterSwitch);
    impl NullDemuxContext {
        fn do_construct(&mut self, _req: demultiplex::FilterRequest<'_, '_>) -> NullFilterSwitch {
            NullFilterSwitch::Nul(demultiplex::NullPacketFilter::default())
        }
    }

    fn make_test_data<F>(builder: F) -> Vec<u8>
    where
        F: Fn(&mut BitWriter<Vec<u8>, BE>) -> Result<(), io::Error>,
    {
        let data: Vec<u8> = Vec::new();
        let mut w = BitWriter::endian(data, BigEndian);
        builder(&mut w).unwrap();
        w.into_writer()
    }

    /// `ts` is a 33-bit timestamp value
    fn write_ts(w: &mut BitWriter<Vec<u8>, BE>, ts: u64, prefix: u8) -> Result<(), io::Error> {
        assert!(
            ts < 1u64 << 33,
            "ts value too large {:#x} >= {:#x}",
            ts,
            1u64 << 33
        );
        w.write(4, prefix & 0b1111)?;
        w.write(3, (ts & 0b1_1100_0000_0000_0000_0000_0000_0000_0000) >> 30)?;
        w.write(1, 1)?; // marker_bit
        w.write(15, (ts & 0b0_0011_1111_1111_1111_1000_0000_0000_0000) >> 15)?;
        w.write(1, 1)?; // marker_bit
        w.write(15, ts & 0b0_0000_0000_0000_0000_0111_1111_1111_1111)?;
        w.write(1, 1) // marker_bit
    }

    fn write_escr(
        w: &mut BitWriter<Vec<u8>, BE>,
        base: u64,
        extension: u16,
    ) -> Result<(), io::Error> {
        assert!(
            base < 1u64 << 33,
            "base value too large {:#x} >= {:#x}",
            base,
            1u64 << 33
        );
        assert!(
            extension < 1u16 << 9,
            "extension value too large {:#x} >= {:#x}",
            base,
            1u16 << 9
        );
        w.write(2, 0b11)?; // reserved
        w.write(
            3,
            (base & 0b1_1100_0000_0000_0000_0000_0000_0000_0000) >> 30,
        )?;
        w.write(1, 1)?; // marker_bit
        w.write(
            15,
            (base & 0b0_0011_1111_1111_1111_1000_0000_0000_0000) >> 15,
        )?;
        w.write(1, 1)?; // marker_bit
        w.write(15, base & 0b0_0000_0000_0000_0000_0111_1111_1111_1111)?;
        w.write(1, 1)?; // marker_bit
        w.write(9, extension)?;
        w.write(1, 1) // marker_bit
    }
    fn write_es_rate(w: &mut BitWriter<Vec<u8>, BE>, rate: u32) -> Result<(), io::Error> {
        assert!(
            rate < 1u32 << 22,
            "rate value too large {:#x} >= {:#x}",
            rate,
            1u32 << 22
        );
        w.write(1, 1)?; // marker_bit
        w.write(22, rate)?;
        w.write(1, 1) // marker_bit
    }

    #[test]
    fn parse_header() {
        let data = make_test_data(|w| {
            w.write(24, 1)?; // packet_start_code_prefix
            w.write(8, 7)?; // stream_id
            w.write(16, 7)?; // PES_packet_length

            w.write(2, 0b10)?; // check-bits
            w.write(2, 0)?; // PES_scrambling_control
            w.write(1, 0)?; // pes_priority
            w.write(1, 1)?; // data_alignment_indicator
            w.write(1, 0)?; // copyright
            w.write(1, 0)?; // original_or_copy
            w.write(2, 0b10)?; // PTS_DTS_flags
            w.write(1, 1)?; // ESCR_flag
            w.write(1, 1)?; // ES_rate_flag
            w.write(1, 1)?; // DSM_trick_mode_flag
            w.write(1, 1)?; // additional_copy_info_flag
            w.write(1, 1)?; // PES_CRC_flag
            w.write(1, 0)?; // PES_extension_flag
            let pes_header_length = 5  // PTS
                + 6  // ESCR
                + 3  // es_rate
                + 1  // DSM trick mode
                + 1  // additional_copy_info
                + 2; // previous_PES_packet_CRC
            w.write(8, pes_header_length)?; // PES_data_length (size of fields that follow)
            write_ts(w, 123456789, 0b0010)?; // PTS
            write_escr(w, 0b111111111111111111111111111111111, 234)?;
            write_es_rate(w, 1234567)?;

            // DSM_trick_mode,
            w.write(3, 0b00)?; // trick_mode_control (== fast_forward)
            w.write(2, 2)?; // field_id
            w.write_bit(true)?; // intra_slice_refresh
            w.write(2, 0)?; // frequency_truncation

            // additonal_copy_info,
            w.write(1, 1)?; // marker_bit
            w.write(7, 123)?; // additional_copy_info
            w.write(16, 54321) // previous_PES_packet_CRC
        });
        let header = pes::PesHeader::from_bytes(&data[..]).unwrap();
        assert_eq!(pes::StreamId::Unknown(7), header.stream_id());
        //assert_eq!(8, header.pes_packet_length());

        match header.contents() {
            pes::PesContents::Parsed(parsed_contents) => {
                let p =
                    parsed_contents.expect("expected PesContents::Parsed(Some(_)) but was None");
                assert_eq!(0, p.pes_priority());
                assert_eq!(pes::DataAlignment::Aligned, p.data_alignment_indicator());
                assert_eq!(pes::Copyright::Protected, p.copyright());
                assert_eq!(pes::OriginalOrCopy::Copy, p.original_or_copy());
                match p.pts_dts() {
                    Ok(pes::PtsDts::PtsOnly(Ok(ts))) => {
                        let a = ts.value();
                        let b = 123456789;
                        assert_eq!(
                            a, b,
                            "timestamp values don't match:\n  actual:{:#b}\nexpected:{:#b}",
                            a, b
                        );
                    }
                    _ => panic!("expected PtsDts::PtsOnly, got{}", stringify!(_)),
                }
                assert_eq!(p.payload().len(), 0);
                match p.escr() {
                    Ok(escr) => {
                        assert_eq!(0b111111111111111111111111111111111, escr.base());
                        assert_eq!(234, escr.extension());
                    }
                    e => panic!("expected escr value, got {:?}", e),
                }
                assert_matches!(
                    p.dsm_trick_mode(),
                    Ok(pes::DsmTrickMode::FastForward {
                        field_id: 2,
                        intra_slice_refresh: true,
                        frequency_truncation:
                            pes::FrequencyTruncationCoefficientSelection::DCNonZero,
                    })
                );
                assert_matches!(p.es_rate(), Ok(pes::EsRate(1234567)));
                assert_matches!(p.additional_copy_info(), Ok(123));
                assert_matches!(p.previous_pes_packet_crc(), Ok(54321));
                println!("{:#?}", p);
            }
            pes::PesContents::Payload(_) => {
                panic!("expected PesContents::Parsed, got PesContents::Payload")
            }
        }
    }

    #[test]
    fn pts() {
        let pts_prefix = 0b0010;
        let pts = make_test_data(|w| {
            write_ts(w, 0b1_0101_0101_0101_0101_0101_0101_0101_0101, pts_prefix)
        });
        let a = pes::Timestamp::from_pts_bytes(&pts[..]).unwrap().value();
        let b = 0b1_0101_0101_0101_0101_0101_0101_0101_0101;
        assert_eq!(
            a, b,
            "timestamp values don't match:\n  actual:{:#b}\nexpected:{:#b}",
            a, b
        );
    }

    #[test]
    fn dts() {
        let pts_prefix = 0b0001;
        let pts = make_test_data(|w| {
            write_ts(w, 0b0_1010_1010_1010_1010_1010_1010_1010_1010, pts_prefix)
        });
        let a = pes::Timestamp::from_dts_bytes(&pts[..]).unwrap().value();
        let b = 0b0_1010_1010_1010_1010_1010_1010_1010_1010;
        assert_eq!(
            a, b,
            "timestamp values don't match:\n  actual:{:#b}\nexpected:{:#b}",
            a, b
        );
    }

    #[test]
    fn timestamp_ones() {
        let pts_prefix = 0b0010;
        let pts = make_test_data(|w| {
            write_ts(w, 0b1_1111_1111_1111_1111_1111_1111_1111_1111, pts_prefix)
        });
        let a = pes::Timestamp::from_pts_bytes(&pts[..]).unwrap().value();
        let b = 0b1_1111_1111_1111_1111_1111_1111_1111_1111;
        assert_eq!(
            a, b,
            "timestamp values don't match:\n  actual:{:#b}\nexpected:{:#b}",
            a, b
        );
    }

    #[test]
    fn timestamp_zeros() {
        let pts_prefix = 0b0010;
        let pts = make_test_data(|w| {
            write_ts(w, 0b0_0000_0000_0000_0000_0000_0000_0000_0000, pts_prefix)
        });
        let a = pes::Timestamp::from_pts_bytes(&pts[..]).unwrap().value();
        let b = 0b0_0000_0000_0000_0000_0000_0000_0000_0000;
        assert_eq!(
            a, b,
            "timestamp values don't match:\n  actual:{:#b}\nexpected:{:#b}",
            a, b
        );
    }

    #[test]
    fn timestamp_bad_prefix() {
        let pts_prefix = 0b0010;
        let mut pts = make_test_data(|w| write_ts(w, 1234, pts_prefix));
        // make the prefix bits invalid by flipping a 0 to a 1,
        pts[0] |= 0b10000000;
        assert_matches!(
            pes::Timestamp::from_pts_bytes(&pts[..]),
            Err(pes::TimestampError::IncorrectPrefixBits {
                expected: 0b0010,
                actual: 0b1010
            })
        )
    }

    #[test]
    fn timestamp_bad_marker() {
        let pts_prefix = 0b0010;
        let mut pts = make_test_data(|w| write_ts(w, 1234, pts_prefix));
        // make the first maker_bit (at index 7) invalid, by flipping a 1 to a 0,
        pts[0] &= 0b11111110;
        assert_matches!(
            pes::Timestamp::from_pts_bytes(&pts[..]),
            Err(pes::TimestampError::MarkerBitNotSet { bit_number: 7 })
        )
    }

    struct MockState {
        start_stream_called: bool,
        begin_packet_called: bool,
        continuity_error_called: bool,
    }
    impl MockState {
        fn new() -> MockState {
            MockState {
                start_stream_called: false,
                begin_packet_called: false,
                continuity_error_called: false,
            }
        }
    }
    struct MockElementaryStreamConsumer {
        state: std::rc::Rc<std::cell::RefCell<MockState>>,
    }
    impl MockElementaryStreamConsumer {
        fn new(state: std::rc::Rc<std::cell::RefCell<MockState>>) -> MockElementaryStreamConsumer {
            MockElementaryStreamConsumer { state }
        }
    }
    impl pes::ElementaryStreamConsumer<NullDemuxContext> for MockElementaryStreamConsumer {
        fn start_stream(&mut self, _ctx: &mut NullDemuxContext) {
            self.state.borrow_mut().start_stream_called = true;
        }
        fn begin_packet(&mut self, _ctx: &mut NullDemuxContext, _header: pes::PesHeader<'_>) {
            self.state.borrow_mut().begin_packet_called = true;
        }
        fn continue_packet(&mut self, _ctx: &mut NullDemuxContext, _data: &[u8]) {}
        fn end_packet(&mut self, _ctx: &mut NullDemuxContext) {}
        fn continuity_error(&mut self, _ctx: &mut NullDemuxContext) {
            self.state.borrow_mut().continuity_error_called = true;
        }
    }

    #[test]
    fn pes_packet_consumer() {
        let state = std::rc::Rc::new(std::cell::RefCell::new(MockState::new()));
        let mock = MockElementaryStreamConsumer::new(state.clone());
        let mut pes_filter = pes::PesPacketFilter::new(mock);
        let buf = hex!("4741F510000001E0000084C00A355DDD11B1155DDBF5910000000109100000000167640029AD843FFFC21FFFE10FFFF087FFF843FFFC21FFFE10FFFFFFFFFFFFFFFF087FFFFFFFFFFFFFFF2CC501E0113F780A1010101F00000303E80000C350940000000168FF3CB0000001060001C006018401103A0408D2BA80000050204E95D400000302040AB500314454473141FEFF53040000C815540DF04F77FFFFFFFFFFFFFFFFFFFF80000000016588800005DB001008673FC365F48EAE");
        let pk = packet::Packet::new(&buf[..]);
        let mut ctx = NullDemuxContext::new();
        pes_filter.consume(&mut ctx, &pk);
        {
            let state = state.borrow();
            assert!(state.start_stream_called);
            assert!(state.begin_packet_called);
            assert!(!state.continuity_error_called);
        }
        // processing the same packet again (therefore with the same continuity_counter value),
        // should cause a continuity error to be flagged,
        let pk = packet::Packet::new(&buf[..]);
        pes_filter.consume(&mut ctx, &pk);
        {
            let state = state.borrow();
            assert!(state.continuity_error_called);
        }
    }

    #[test]
    fn header_length_doesnt_fit() {
        let data = make_test_data(|w| {
            w.write(24, 1)?; // packet_start_code_prefix
            w.write(8, 7)?; // stream_id
            w.write(16, 7)?; // PES_packet_length

            w.write(2, 0b10)?; // check-bits
            w.write(2, 0)?; // PES_scrambling_control
            w.write(1, 0)?; // pes_priority
            w.write(1, 1)?; // data_alignment_indicator
            w.write(1, 0)?; // copyright
            w.write(1, 0)?; // original_or_copy
            w.write(2, 0b00)?; // PTS_DTS_flags
            w.write(1, 0)?; // ESCR_flag
            w.write(1, 0)?; // ES_rate_flag
            w.write(1, 0)?; // DSM_trick_mode_flag
            w.write(1, 0)?; // additional_copy_info_flag
            w.write(1, 1)?; // PES_CRC_flag
            w.write(1, 0)?; // PES_extension_flag
            let pes_header_length = 2; // previous_PES_packet_CRC
            w.write(8, pes_header_length)?; // PES_data_length (size of fields that follow)

            // deliberately write 1 byte where 2 bytes are expected
            w.write(8, 1) // INVALID previous_PES_packet_CRC
        });
        // the buffer generated is now one byte too short, so attempting to get the PES contents
        // should fail,
        let header = pes::PesHeader::from_bytes(&data[..]).unwrap();
        assert!(matches!(header.contents(), pes::PesContents::Parsed(None)));
    }

    #[test]
    fn pes_header_data_length_too_short() {
        let data = make_test_data(|w| {
            w.write(24, 1)?; // packet_start_code_prefix
            w.write(8, 7)?; // stream_id
            w.write(16, 7)?; // PES_packet_length

            w.write(2, 0b10)?; // check-bits
            w.write(2, 0)?; // PES_scrambling_control
            w.write(1, 0)?; // pes_priority
            w.write(1, 1)?; // data_alignment_indicator
            w.write(1, 0)?; // copyright
            w.write(1, 0)?; // original_or_copy
            w.write(2, 0b00)?; // PTS_DTS_flags
            w.write(1, 0)?; // ESCR_flag
            w.write(1, 0)?; // ES_rate_flag
            w.write(1, 0)?; // DSM_trick_mode_flag
            w.write(1, 0)?; // additional_copy_info_flag
            w.write(1, 1)?; // PES_CRC_flag
            w.write(1, 0)?; // PES_extension_flag
            let pes_header_length = 1; // invalid; we need two bytes for previous_PES_packet_CRC
            w.write(8, pes_header_length)?; // PES_header_data_length (size of fields that follow)

            // we put the correct number of bytes into the buffer, but the pes_header_length above
            // doesn't reflect what's actually here
            w.write(8, 2) // previous_PES_packet_CRC
        });
        let header = pes::PesHeader::from_bytes(&data[..]).unwrap();
        assert!(matches!(header.contents(), pes::PesContents::Parsed(None)));
    }
}
