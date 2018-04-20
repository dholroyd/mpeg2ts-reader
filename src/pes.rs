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

use packet;
use demultiplex;
use std::marker;

/// Trait for types that will receive call-backs as pieces of a specific elementary stream are
/// encounted within a transport stream.
///
/// Instances of this type are registered with a
/// [`PesPacketConsumer`](struct.PesPacketConsumer.html), which itself is responsible for
/// extracting elementary stream data from transport stream packets.
pub trait ElementaryStreamConsumer {
    fn start_stream(&mut self);
    fn begin_packet(&mut self, header: PesHeader);
    fn continue_packet(&mut self, data: &[u8]);
    fn end_packet(&mut self);
    fn continuity_error(&mut self);
}

#[derive(Debug,PartialEq)]
enum PesState {
    Begin,
    Started,
    IgnoreRest,
}

/// Extracts elementary stream data from a series of transport stream packets which are passed
/// one-by-one.
///
/// A `PesPacketConsumer` is registered with a
/// [`Demultiplex`](../demultiplex/struct.Demultiplex.html) instance
pub struct PesPacketConsumer<C>
where
    C: ElementaryStreamConsumer
{
    stream_consumer: C,
    ccounter: Option<packet::ContinuityCounter>,
    state: PesState,
}
impl<C> PesPacketConsumer<C>
where
    C: ElementaryStreamConsumer
{
    pub fn new(stream_consumer: C) -> PesPacketConsumer<C> {
        PesPacketConsumer {
            stream_consumer,
            ccounter: None,
            state: PesState::Begin,
        }
    }

    pub fn is_continuous(&self, packet: &packet::Packet) -> bool {
        if let Some(cc) = self.ccounter {
            // counter only increases if the packet has a payload,
            let result = if packet.adaptation_control().has_payload() {
                packet.continuity_counter().follows(cc)
            } else {
                packet.continuity_counter().count() == cc.count()
            };
            if !result {
                println!("discontinuity at packet with PID={} last={} this={} ({:?})", packet.pid(), cc.count(), packet.continuity_counter().count(), packet.adaptation_control());
            }
            result
        } else {
            true
        }
    }

    #[inline(always)]
    pub fn consume(&mut self, packet: packet::Packet) {
        if !self.is_continuous(&packet) {
            self.stream_consumer.continuity_error();
            self.state = PesState::IgnoreRest;
        }
        self.ccounter = Some(packet.continuity_counter());
        if packet.payload_unit_start_indicator() {
            if self.state == PesState::Started {
                self.stream_consumer.end_packet();
            } else {
                self.state = PesState::Started;
            }
            if let Some(payload) = packet.payload() {
                if let Some(header) = PesHeader::from_bytes(payload) {
                    self.stream_consumer.begin_packet(header);
                }
            }
        } else {
            match self.state {
                PesState::Started => {
                    if let Some(payload) = packet.payload() {
                        if payload.len() > 0 {
                            self.stream_consumer.continue_packet(payload);
                        }
                    }
                },
                PesState::Begin => {
                    println!("pid={}: Ignoring elementary stream content without a payload_start_indicator", packet.pid());
                    self.state = PesState::IgnoreRest;
                },
                PesState::IgnoreRest => ()
            }
        }
    }
}

pub struct PesPacketFilter<Ctx,E>
where
    Ctx: demultiplex::DemuxContext,
    E: ElementaryStreamConsumer
{
    consumer: PesPacketConsumer<E>,
    phantom: marker::PhantomData<Ctx>,
}
impl<Ctx,E> PesPacketFilter<Ctx,E>
    where
        Ctx: demultiplex::DemuxContext,
        E: ElementaryStreamConsumer
{
    pub fn new(consumer: E) -> PesPacketFilter<Ctx,E> {
        PesPacketFilter {
            consumer: PesPacketConsumer::new(consumer),
            phantom: marker::PhantomData,
        }
    }
}
impl<Ctx,E> demultiplex::PacketFilter for PesPacketFilter<Ctx,E>
where
    Ctx: demultiplex::DemuxContext,
    E: ElementaryStreamConsumer
{
    type Ctx = Ctx;

    #[inline(always)]
    fn consume(&mut self, _ctx: &mut Self::Ctx, pk: packet::Packet) {
        self.consumer.consume(pk);
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
    buf: &'buf[u8],
}
impl<'buf> PesHeader<'buf> {
    pub fn from_bytes(buf: &'buf[u8]) -> Option<PesHeader> {
        if buf.len() < 6 {
            println!("Buffer size {} too small to hold PES header", buf.len());
            return None;
        }
        let packet_start_code_prefix = u32::from(buf[0]) << 16 | u32::from(buf[1]) << 8 | u32::from(buf[2]);
        if packet_start_code_prefix != 1 {
            println!("invalid packet_start_code_prefix {:#x}, expected 0x000001", packet_start_code_prefix);
            return None
        }
        Some(PesHeader {
            buf,
        })
    }

    pub fn stream_id(&self) -> u8 {
        self.buf[3]
    }

    pub fn pes_packet_length(&self) -> u16 {
        u16::from(self.buf[4]) << 8 | u16::from(self.buf[5])
    }

    // maaaaaybe just have parsed_contents(&self) + payload(&self)
    pub fn contents(&self) -> PesContents<'buf> {
        let header_len = 6;
        let rest = &self.buf[header_len..];
        if is_parsed(self.stream_id()) {
            PesContents::Parsed(PesParsedContents::from_bytes(rest))
        } else {
            PesContents::Payload(rest)
        }
    }
}

fn is_parsed(stream_id: u8) -> bool {
    match stream_id {
        0b1011_1100 |
        0b1011_1111 |
        0b1111_0000 |
        0b1111_0001 |
        0b1111_1111 |
        0b1111_0010 |
        0b1111_1000 => false,
        _ => true,
    }
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
    Parsed(Option<PesParsedContents<'buf>>),
    Payload(&'buf[u8]),
}

/// Extra data which may optionally be present in the `PesHeader`, potentially including
/// Presentation Timestamp (PTS) and Decode Timestamp (DTS) values.
pub struct PesParsedContents<'buf> {
    buf: &'buf[u8],
}
impl<'buf> PesParsedContents<'buf> {
    pub fn from_bytes(buf: &'buf[u8]) -> Option<PesParsedContents<'buf>> {
        if buf.len() < 3 {
            println!("buf not large enough to hold PES parsed header: {} bytes", buf.len());
            return None;
        }
        let check_bits = buf[0] >> 6;
        if check_bits != 0b10 {
            println!("unexpected check-bits value {:#b}, expected 0b10", check_bits);
            return None;
        }
        Some(PesParsedContents{
            buf
        })
    }

    /// value 1 indicates higher priority and 0 indicates lower priority
    pub fn pes_priority(&self) -> u8 {
        self.buf[0] >> 3 & 1
    }
    pub fn data_alignment_indicator(&self) -> DataAlignment {
        if self.buf[0] & 0b100 != 0 {
            DataAlignment::Aligned
        } else {
            DataAlignment::NotAligned
        }
    }
    pub fn copyright(&self) -> Copyright {
        if self.buf[0] & 0b10 != 0 {
            Copyright::Undefined
        } else {
            Copyright::Protected
        }
    }
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
    /*
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
    */
    fn pes_header_data_len(&self) -> usize {
        self.buf[2] as usize
    }
    pub fn pts_dts(&self) -> PtsDts {
        let header_size = 3;
        let timestamp_size = 5;
        match self.pts_dts_flags() {
            0b00 => PtsDts::None,
            0b01 => PtsDts::Invalid,
            0b10 => {
                if self.buf.len() < header_size+timestamp_size {
                    println!("PES packet buffer not long enough to hold of PTS, {}", self.buf.len());
                    return PtsDts::None;
                }
                PtsDts::PtsOnly(
                    Timestamp::from_bytes(&self.buf[header_size..header_size+timestamp_size])
                )
            },
            0b11 => {
                if self.buf.len() < header_size+timestamp_size*2 {
                    println!("PES packet buffer not long enough to hold of PTS+DTS, {}", self.buf.len());
                    return PtsDts::None;
                }
                PtsDts::Both {
                    pts: Timestamp::from_bytes(&self.buf[header_size..header_size+timestamp_size]),
                    dts: Timestamp::from_bytes(&self.buf[header_size+timestamp_size..header_size+timestamp_size*2]),
                }
            },
            v => panic!("unexpected value {}", v),
        }
    }
    pub fn payload(&self) -> &'buf[u8] {
        let fixed_header_len = 3;
        &self.buf[fixed_header_len+self.pes_header_data_len()..]
    }
}


/// Detail about the formatting problem which prevented a [`Timestamp`](struct.Timestamp.html)
/// value being parsed.
#[derive(PartialEq,Debug)]
pub enum TimestampError {
    IncorrectPrefixBits {
        expected: u8,
        actual: u8,
    },
    MarkerBitNotSet {
        bit_number: u8,
    }
}

/// A 33-bit Elementary Stream timestamp, used to represent PTS and DTS values which may appear in
/// an Elementy Stream header.
#[derive(PartialEq,Debug,Clone,Copy)]
pub struct Timestamp {
    val: u64,
}
impl Timestamp {
    pub fn from_pts_bytes(buf: &[u8]) -> Result<Timestamp,TimestampError> {
        Timestamp::check_prefix(buf, 0b0010)?;
        Timestamp::from_bytes(buf)
    }
    pub fn from_dts_bytes(buf: &[u8]) -> Result<Timestamp,TimestampError> {
        Timestamp::check_prefix(buf, 0b0001)?;
        Timestamp::from_bytes(buf)
    }
    fn check_prefix(buf: &[u8], expected: u8) -> Result<(),TimestampError> {
        assert!(expected <= 0b1111);
        let actual = buf[0] >> 4;
        if actual == expected {
            Ok(())
        } else {
            Err(TimestampError::IncorrectPrefixBits{ expected, actual })
        }
    }
    fn check_marker_bit(buf: &[u8], bit_number: u8) -> Result<(),TimestampError> {
        let byte_index = bit_number / 8;
        let bit_index = bit_number % 8;
        let bit_mask = 1 << (7 - bit_index);
        if buf[byte_index as usize] & bit_mask != 0 {
            Ok(())
        } else {
            Err(TimestampError::MarkerBitNotSet { bit_number })
        }
    }
    fn from_bytes(buf: &[u8]) -> Result<Timestamp,TimestampError> {
        Timestamp::check_marker_bit(buf, 7)?;
        Timestamp::check_marker_bit(buf, 23)?;
        Timestamp::check_marker_bit(buf, 39)?;
        Ok(Timestamp {
            val: (u64::from(buf[0] & 0b00001110) << 29) |
                  u64::from(buf[1]) << 22 |
                 (u64::from(buf[2] & 0b11111110) << 14) |
                  u64::from(buf[3]) << 7 |
                  u64::from(buf[4]) >> 1
        })
    }
    pub fn value(&self) -> u64 {
        self.val
    }
}


#[derive(PartialEq,Debug)]
pub enum PtsDts {
    None,
    PtsOnly (Result<Timestamp,TimestampError>),
    Invalid,
    Both {
        pts: Result<Timestamp,TimestampError>,
        dts: Result<Timestamp,TimestampError>,
    },
}

/// Indicates if the start of some 'unit' of Elementary Stream content is immediately at the start of the PES
/// packet payload.
///
/// Returned by
/// [`PesParsedContents.data_alignment_indicator()`](struct.PesParsedContents.html#method.data_alignment_indicator)
#[derive(PartialEq,Debug)]
pub enum DataAlignment {
    Aligned,
    NotAligned,
}
/// Indicates the copyright status of the contents of the Elementary Stream packet.
///
/// Returned by [`PesParsedContents.copyright()`](struct.PesParsedContents.html#method.copyright)
#[derive(PartialEq,Debug)]
pub enum Copyright {
    Protected,
    Undefined,
}
/// Indicates weather the contents of the Elementary Stream packet are original or a copy.
///
/// Returned by
/// [`PesParsedContents.original_or_copy()`](struct.PesParsedContents.html#method.original_or_copy)
#[derive(PartialEq,Debug)]
pub enum OriginalOrCopy {
    Original,
    Copy,
}

#[cfg(test)]
mod test {
    use std::io;
    use std;
    use bitstream_io::{BE, BitWriter};
    use data_encoding::base16;
    use pes;
    use packet;

    fn make_test_data<F>(builder: F) -> Vec<u8>
    where
        F: Fn(BitWriter<BE>)->Result<(), io::Error>
    {
        let mut data: Vec<u8> = Vec::new();
        builder(BitWriter::<BE>::new(&mut data)).unwrap();
        data
    }

    /// `ts` is a 33-bit timestamp value
    fn write_ts(w: &mut BitWriter<BE>, ts: u64, prefix: u8) -> Result<(), io::Error> {
        assert!(ts < 1u64<<33, "ts value too large {:#x} >= {:#x}", ts, 1u64<<33);
        w.write(4, prefix & 0b1111)?;
        w.write(3,  (ts & 0b1_1100_0000_0000_0000_0000_0000_0000_0000) >> 30)?;
        w.write(1, 1)?;     // marker_bit
        w.write(15, (ts & 0b0_0011_1111_1111_1111_1000_0000_0000_0000) >> 15)?;
        w.write(1, 1)?;     // marker_bit
        w.write(15, ts & 0b0_0000_0000_0000_0000_0111_1111_1111_1111)?;
        w.write(1, 1)       // marker_bit
    }

    #[test]
    fn parse_header() {
        let data = make_test_data(|mut w| {
            w.write(24, 1)?; // packet_start_code_prefix
            w.write(8, 7)?;  // stream_id
            w.write(16, 7)?; // PES_packet_length

            w.write(2, 0b10)?;  // check-bits
            w.write(2, 0)?;     // PES_scrambling_control
            w.write(1, 0)?;     // pes_priority
            w.write(1, 1)?;     // data_alignment_indicator
            w.write(1, 0)?;     // copyright
            w.write(1, 0)?;     // original_or_copy
            w.write(2, 0b10)?;  // PTS_DTS_flags
            w.write(1, 0)?;     // ESCR_flag
            w.write(1, 0)?;     // ES_rate_flag
            w.write(1, 0)?;     // DSM_trick_mode_flag
            w.write(1, 0)?;     // additonal_copy_info_flag
            w.write(1, 0)?;     // PES_CRC_flag
            w.write(1, 0)?;     // PES_extension_flag
            w.write(8, 5)?;     // PES_data_length (size of following PTS)
            write_ts(&mut w, 123456789, 0b0010)  // PTS
        });
        let header = pes::PesHeader::from_bytes(&data[..]).unwrap();
        assert_eq!(7, header.stream_id());
        //assert_eq!(8, header.pes_packet_length());

        match header.contents() {
            pes::PesContents::Parsed(parsed_contents) => {
                let p = parsed_contents.expect("expected PesContents::Parsed(Some(_)) but was None");
                assert_eq!(0, p.pes_priority());
                assert_eq!(pes::DataAlignment::Aligned, p.data_alignment_indicator());
                assert_eq!(pes::Copyright::Protected, p.copyright());
                assert_eq!(pes::OriginalOrCopy::Copy, p.original_or_copy());
                match p.pts_dts() {
                    pes::PtsDts::PtsOnly(Ok(ts)) => {
                        let a = ts.value();
                        let b = 123456789;
                        assert_eq!(a, b, "timestamp values don't match:\n  actual:{:#b}\nexpected:{:#b}", a, b);
                    },
                    _ => panic!("expected PtsDts::PtsOnly, got{}", stringify!(_)),
                }
                assert_eq!(p.payload().len(), 0);
            },
            pes::PesContents::Payload(_) => panic!("expected PesContents::Parsed, got PesContents::Payload"),
        }
    }

    #[test]
    fn pts() {
        let pts_prefix = 0b0010;
        let pts = make_test_data(|mut w| {
            write_ts(&mut w, 0b1_0101_0101_0101_0101_0101_0101_0101_0101, pts_prefix)
        });
        let a = pes::Timestamp::from_pts_bytes(&pts[..]).unwrap().value();
        let b = 0b1_0101_0101_0101_0101_0101_0101_0101_0101;
        assert_eq!(a, b, "timestamp values don't match:\n  actual:{:#b}\nexpected:{:#b}", a, b);
    }

    #[test]
    fn dts() {
        let pts_prefix = 0b0001;
        let pts = make_test_data(|mut w| {
            write_ts(&mut w, 0b0_1010_1010_1010_1010_1010_1010_1010_1010, pts_prefix)
        });
        let a = pes::Timestamp::from_dts_bytes(&pts[..]).unwrap().value();
        let b = 0b0_1010_1010_1010_1010_1010_1010_1010_1010;
        assert_eq!(a, b, "timestamp values don't match:\n  actual:{:#b}\nexpected:{:#b}", a, b);
    }

    #[test]
    fn timestamp_ones() {
        let pts_prefix = 0b0010;
        let pts = make_test_data(|mut w| {
            write_ts(&mut w, 0b1_1111_1111_1111_1111_1111_1111_1111_1111, pts_prefix)
        });
        let a = pes::Timestamp::from_pts_bytes(&pts[..]).unwrap().value();
        let b = 0b1_1111_1111_1111_1111_1111_1111_1111_1111;
        assert_eq!(a, b, "timestamp values don't match:\n  actual:{:#b}\nexpected:{:#b}", a, b);
    }

    #[test]
    fn timestamp_zeros() {
        let pts_prefix = 0b0010;
        let pts = make_test_data(|mut w| {
            write_ts(&mut w, 0b0_0000_0000_0000_0000_0000_0000_0000_0000, pts_prefix)
        });
        let a = pes::Timestamp::from_pts_bytes(&pts[..]).unwrap().value();
        let b = 0b0_0000_0000_0000_0000_0000_0000_0000_0000;
        assert_eq!(a, b, "timestamp values don't match:\n  actual:{:#b}\nexpected:{:#b}", a, b);
    }

    #[test]
    fn timestamp_bad_prefix() {
        let pts_prefix = 0b0010;
        let mut pts = make_test_data(|mut w| {
            write_ts(&mut w, 1234, pts_prefix)
        });
        // make the prefix bits invalid by flipping a 0 to a 1,
        pts[0] |= 0b10000000;
        assert_matches!(pes::Timestamp::from_pts_bytes(&pts[..]), Err(pes::TimestampError::IncorrectPrefixBits{ expected: 0b0010, actual: 0b1010 }))
    }

    #[test]
    fn timestamp_bad_marker() {
        let pts_prefix = 0b0010;
        let mut pts = make_test_data(|mut w| {
            write_ts(&mut w, 1234, pts_prefix)
        });
        // make the first maker_bit (at index 7) invalid, by flipping a 1 to a 0,
        pts[0] &= 0b11111110;
        assert_matches!(pes::Timestamp::from_pts_bytes(&pts[..]), Err(pes::TimestampError::MarkerBitNotSet{ bit_number: 7 }))
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
        state: std::rc::Rc<std::cell::RefCell<MockState>>
    }
    impl MockElementaryStreamConsumer {
        fn new(state: std::rc::Rc<std::cell::RefCell<MockState>>) -> MockElementaryStreamConsumer {
            MockElementaryStreamConsumer {
                state
            }
        }
    }
    impl pes::ElementaryStreamConsumer for MockElementaryStreamConsumer {
        fn start_stream(&mut self) {
            self.state.borrow_mut().start_stream_called = true;
        }
        fn begin_packet(&mut self, _header: pes::PesHeader) {
            self.state.borrow_mut().begin_packet_called = true;
        }
        fn continue_packet(&mut self, _data: &[u8]) {
        }
        fn end_packet(&mut self) {
        }
        fn continuity_error(&mut self) {
            self.state.borrow_mut().continuity_error_called = true;
        }
    }

    #[test]
    fn pes_packet_consumer() {
        let state = std::rc::Rc::new(std::cell::RefCell::new(MockState::new()));
        let mock = MockElementaryStreamConsumer::new(state.clone());
        let mut pes_consumer = pes::PesPacketConsumer::new(mock);
        let buf = base16::decode(b"4741F510000001E0000084C00A355DDD11B1155DDBF5910000000109100000000167640029AD843FFFC21FFFE10FFFF087FFF843FFFC21FFFE10FFFFFFFFFFFFFFFF087FFFFFFFFFFFFFFF2CC501E0113F780A1010101F00000303E80000C350940000000168FF3CB0000001060001C006018401103A0408D2BA80000050204E95D400000302040AB500314454473141FEFF53040000C815540DF04F77FFFFFFFFFFFFFFFFFFFF80000000016588800005DB001008673FC365F48EAE").unwrap();
        let pk = packet::Packet::new(&buf[..]);
        pes_consumer.consume(pk);
        {
            let state = state.borrow();
            assert!(state.begin_packet_called);
            assert!(!state.continuity_error_called);
        }
        // processing the same packet again (therefore with the same continuity_counter value),
        // should cause a continuity error to be flagged,
        let pk = packet::Packet::new(&buf[..]);
        pes_consumer.consume(pk);
        {
            let state = state.borrow();
            assert!(state.continuity_error_called);
        }
    }
}
