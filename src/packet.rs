//! A [`Packet`](./struct.Packet.html) struct and associated infrastructure to read an MPEG Transport Stream packet


use std::fmt;
use pes;

/// the different values indicating whether a `Packet`'s `adaptation_field()` and `payload()`
/// methods will return `Some` or `None`.
#[derive(Eq, PartialEq, Debug)]
pub enum AdaptationControl {
    /// This value is used if the transport stream packet `adaptation_control` field uses the value
    /// `0b00`, which is not defined by the spec.
    Reserved,
    /// indicates that this packet contains a payload, but not an adaptation field
    PayloadOnly,
    /// indicates that this packet contains an adaptation field, but not a payload
    AdaptationFieldOnly,
    /// indicates that this packet contains both an adaptation field and a payload
    AdaptationFieldAndPayload,
}

impl AdaptationControl {
    #[inline(always)]
    fn from(val: u8) -> AdaptationControl {
        match val {
            0 => AdaptationControl::Reserved,
            1 => AdaptationControl::PayloadOnly,
            2 => AdaptationControl::AdaptationFieldOnly,
            3 => AdaptationControl::AdaptationFieldAndPayload,
            _ => panic!("invalid value {}", val),
        }
    }

    #[inline(always)]
    pub fn has_payload(self) -> bool {
        match self {
            AdaptationControl::Reserved | AdaptationControl::AdaptationFieldOnly => false,
            AdaptationControl::PayloadOnly | AdaptationControl::AdaptationFieldAndPayload => true,
        }
    }
}

#[derive(Eq, PartialEq, Debug)]
pub enum TransportScramblingControl {
    NotScrambled,
    Undefined(u8),
}

impl TransportScramblingControl {
    fn from(val: u8) -> TransportScramblingControl {
        match val {
            0 => TransportScramblingControl::NotScrambled,
            1...3 => TransportScramblingControl::Undefined(val),
            _ => panic!("invalid value {}", val),
        }
    }
}

/// Program Clock Reference
pub struct PCR {
    base: u64,
    extension: u16,
}

impl PartialEq for PCR {
    fn eq(&self, other: &PCR) -> bool {
        self.base == other.base && self.extension == other.extension
    }
}

impl From<PCR> for u64 {
    fn from(pcr: PCR) -> u64 {
        pcr.base * 300 + pcr.extension as u64
    }
}

impl fmt::Debug for PCR {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.debug_struct("PCR")
            .field("base", &self.base)
            .field("extension", &self.extension)
            .finish()
    }
}
impl PCR {
    /// Panics if `data` is shorter than 5 bytes
    pub fn from_slice(data: &[u8]) -> PCR {
        PCR {
            base: u64::from(data[0]) << 25 | u64::from(data[1]) << 17 | u64::from(data[2]) << 9 | u64::from(data[3]) << 1 | u64::from(data[4]) >> 7,
            //reserved: (data[4] >> 1) & 0b00111111,
            extension: (u16::from(data[4]) & 0b1) << 8 | u16::from(data[5]),
        }
    }
    /// Panics if the `base` is greater than 2^33-1 or the `extension` is greater than 2^9-1
    pub fn from_parts(base: u64, extension: u16) -> PCR {
        assert!(base < (1<<33));
        assert!(extension < (1<<9));
        PCR {
            base,
            extension,
        }
    }
}

#[derive(Debug,PartialEq)]
pub enum AdaptationFieldError {
    FieldNotPresent,
    NotEnoughData,
    SpliceTimestampError(pes::TimestampError)
}

/// A collection of fields that may optionally appear within the header of a transport stream
/// `Packet`.
pub struct AdaptationField<'buf> {
    buf: &'buf [u8],
}

impl<'buf> AdaptationField<'buf> {
    // TODO: just eager-load all this stuff in new()?  would be simpler!

    pub fn new(buf: &'buf [u8]) -> AdaptationField {
        AdaptationField { buf }
    }

    pub fn discontinuity_indicator(&self) -> bool {
        self.buf[0] & 0b1000_0000 != 0
    }
    pub fn random_access_indicator(&self) -> bool {
        self.buf[0] & 0b0100_0000 != 0
    }
    pub fn elementary_stream_priority_indicator(&self) -> u8 {
        (self.buf[0] & 0b10_0000) >> 5
    }
    fn pcr_flag(&self) -> bool {
        self.buf[0] & 0b1_0000 != 0
    }
    fn opcr_flag(&self) -> bool {
        self.buf[0] & 0b1000 != 0
    }
    fn splicing_point_flag(&self) -> bool {
        self.buf[0] & 0b100 != 0
    }
    fn transport_private_data_flag(&self) -> bool {
        self.buf[0] & 0b10 != 0
    }
    fn adaptation_field_extension_flag(&self) -> bool {
        self.buf[0] & 0b1 != 0
    }
    fn slice(&self, from: usize, to: usize) -> Result<&'buf[u8],AdaptationFieldError> {
        if to > self.buf.len() {
            Err(AdaptationFieldError::NotEnoughData)
        } else {
            Ok(&self.buf[from..to])
        }
    }
    const PCR_SIZE: usize = 6;
    pub fn pcr(&self) -> Result<PCR, AdaptationFieldError> {
        if self.pcr_flag() {
            Ok(PCR::from_slice(self.slice(1, 1+Self::PCR_SIZE)?))
        } else {
            Err(AdaptationFieldError::FieldNotPresent)
        }
    }
    fn opcr_offset(&self) -> usize {
        if self.pcr_flag() {
            1 + Self::PCR_SIZE
        } else {
            1
        }
    }
    /// Returns the 'Original Program Clock Reference' value, is present.
    pub fn opcr(&self) -> Result<PCR, AdaptationFieldError> {
        if self.opcr_flag() {
            let off = self.opcr_offset();
            Ok(PCR::from_slice(self.slice(off, off+Self::PCR_SIZE)?))
        } else {
            Err(AdaptationFieldError::FieldNotPresent)
        }
    }
    fn splice_countdown_offset(&self) -> usize {
        self.opcr_offset() + if self.opcr_flag() {
            Self::PCR_SIZE
        } else {
            0
        }
    }
    pub fn splice_countdown(&self) -> Result<u8, AdaptationFieldError> {
        if self.splicing_point_flag() {
            let off = self.splice_countdown_offset();
            Ok(self.slice(off, off + 1)?[0])
        } else {
            Err(AdaptationFieldError::FieldNotPresent)
        }
    }
    fn transport_private_data_offset(&self) -> usize {
        self.splice_countdown_offset() + if self.splicing_point_flag() {
            1
        } else {
            0
        }
    }
    pub fn transport_private_data(&self) -> Result<&[u8], AdaptationFieldError> {
        if self.transport_private_data_flag() {
            let off = self.transport_private_data_offset();
            let len = self.slice(off, off + 1)?[0] as usize;
            Ok(self.slice(off + 1, off + 1 + len)?)
        } else {
            Err(AdaptationFieldError::FieldNotPresent)
        }
    }
    fn adaptation_field_extension_offset(&self) -> Result<usize, AdaptationFieldError> {
        let off = self.transport_private_data_offset();
        Ok(off + if self.transport_private_data_flag() {
            let len = self.slice(off, off + 1)?[0] as usize;
            len + 1
        } else {
            0
        })
    }
    pub fn adaptation_field_extension(&self) -> Result<AdaptationFieldExtension<'buf>, AdaptationFieldError> {
        if self.adaptation_field_extension_flag() {
            let off = self.adaptation_field_extension_offset()?;
            let len = self.slice(off, off + 1)?[0] as usize;
            Ok(AdaptationFieldExtension::new(self.slice(off + 1, off + 1 + len)?))
        } else {
            Err(AdaptationFieldError::FieldNotPresent)
        }

    }
}

/// Optional extensions within an [`AdaptationField`](struct.AdaptationField.html).
pub struct AdaptationFieldExtension<'buf> {
    buf: &'buf [u8],
}
impl<'buf> AdaptationFieldExtension<'buf> {
    pub fn new(buf: &'buf [u8]) -> AdaptationFieldExtension<'buf> {
        AdaptationFieldExtension {
            buf,
        }
    }

    fn slice(&self, from: usize, to: usize) -> Result<&'buf[u8],AdaptationFieldError> {
        if to > self.buf.len() {
            Err(AdaptationFieldError::NotEnoughData)
        } else {
            Ok(&self.buf[from..to])
        }
    }

    fn ltw_flag(&self) -> bool {
        self.buf[0] & 0b1000_0000 != 0
    }
    fn piecewise_rate_flag(&self) -> bool {
        self.buf[0] & 0b0100_0000 != 0
    }
    fn seamless_splice_flag(&self) -> bool {
        self.buf[0] & 0b0010_0000 != 0
    }
    /// Returns the 'Legal time window offset', if any.
    pub fn ltw_offset(&self) -> Result<Option<u16>, AdaptationFieldError> {
        if self.ltw_flag() {
            let dat = self.slice(1, 3)?;
            let ltw_valid_flag = dat[0] & 0b1000_0000 != 0;
            Ok(if ltw_valid_flag {
                Some(u16::from(dat[0] & 0b0111_1111)<<8
                    |u16::from(dat[1]))
            } else {
                None
            })
        } else {
            Err(AdaptationFieldError::FieldNotPresent)
        }
    }
    fn piecewise_rate_offset(&self) -> usize {
        1 + if self.ltw_flag() {
            2
        } else {
            0
        }
    }
    pub fn piecewise_rate(&self) -> Result<u32, AdaptationFieldError> {
        if self.piecewise_rate_flag() {
            let off = self.piecewise_rate_offset();
            let dat = self.slice(off, off+3)?;
            Ok(u32::from(dat[0] & 0b0011_1111)<<16
                |u32::from(dat[1])<<8
                |u32::from(dat[2]))
        } else {
            Err(AdaptationFieldError::FieldNotPresent)
        }
    }
    fn seamless_splice_offset(&self) -> usize {
        self.piecewise_rate_offset() + if self.piecewise_rate_flag() {
            3
        } else {
            0
        }
    }
    pub fn seamless_splice(&self) -> Result<SeamlessSplice, AdaptationFieldError> {
        if self.seamless_splice_flag() {
            let off = self.seamless_splice_offset();
            let dat = self.slice(off, off+5)?;
            Ok(SeamlessSplice {
                splice_type: dat[0] >> 4,
                dts_next_au: pes::Timestamp::from_bytes(dat).map_err( AdaptationFieldError::SpliceTimestampError ) ?
            })
        } else {
            Err(AdaptationFieldError::FieldNotPresent)
        }

    }
}

#[derive(Debug,PartialEq)]
pub struct SeamlessSplice {
    pub splice_type: u8,
    pub dts_next_au: pes::Timestamp,
}

/// A counter value used within a transport stream to detect discontinuities in a sequence of packets.
/// The continuity counter should increase by one for each packet with a given PID for which
/// `adaptation_control` indicates that a payload should be present.
///
/// See [`Packet.continuity_counter()`](struct.Packet.html#method.continuity_counter)
#[derive(PartialEq,Debug,Clone,Copy)]
pub struct ContinuityCounter {
    val: u8,
}

impl From<u8> for ContinuityCounter {
    #[inline]
    fn from(count: u8) -> ContinuityCounter {
        ContinuityCounter::new(count)
    }
}

impl ContinuityCounter {
    /// Panics if the given value is greater than 15.
    #[inline]
    pub fn new(count: u8) -> ContinuityCounter {
        assert!(count < 0b10000);
        ContinuityCounter { val: count }
    }

    /// Returns this counter's value, which will be between 0 and 15 inclusive.
    #[inline]
    pub fn count(&self) -> u8 {
        self.val
    }

    /// true iff the given `ContinuityCounter` value follows this one.  Note that the maximum counter
    /// value is 15, and the counter 'wraps around':
    ///
    /// ```rust
    /// # use mpeg2ts_reader::packet::ContinuityCounter;
    /// let a = ContinuityCounter::new(0);
    /// let b = ContinuityCounter::new(15);
    /// assert!(a.follows(b));  // after 15, counter wraps around to 0
    /// ```
    #[inline]
    pub fn follows(&self, other: ContinuityCounter) -> bool {
        (other.val + 1) & 0b1111 == self.val
    }
}

/// A transport stream `Packet` is a wrapper around a byte slice which allows the bytes to be
/// interpreted as a packet structure per _ISO/IEC 13818-1, Section 2.4.3.3_.
pub struct Packet<'buf> {
    buf: &'buf [u8],
}

/// The value `0x47`, which must appear in the first byte of every transport stream packet.
pub const SYNC_BYTE: u8 = 0x47;

/// The fixed 188 byte size of a transport stream packet.
pub const PACKET_SIZE: usize = 188;

const FIXED_HEADER_SIZE: usize = 4;
// when AF present, a 1-byte 'length' field precedes the content,
const ADAPTATION_FIELD_OFFSET: usize = FIXED_HEADER_SIZE + 1;

impl<'buf> Packet<'buf> {
    /// returns `true` if the given value is a valid synchronisation byte, the value `0x42`, which
    /// must appear at the start of every transport stream packet.
    #[inline(always)]
    pub fn is_sync_byte(b: u8) -> bool {
        b == SYNC_BYTE
    }

    /// Panics if the given buffer is less than 188 bytes, or if the initial sync-byte does not
    /// have the correct value (`0x47`).  Calling code is expected to have already checked those
    /// conditions.
    #[inline(always)]
    pub fn new(buf: &'buf [u8]) -> Packet {
        assert_eq!(buf.len(),  PACKET_SIZE);
        assert!(Packet::is_sync_byte(buf[0]));
        Packet { buf }
    }

    pub fn transport_error_indicator(&self) -> bool {
        self.buf[1] & 0b10000000 != 0
    }

    /// a structure larger than a single packet payload needs to be split across multiple packets,
    /// `payload_unit_start()` indicates if this packet payload contains the start of the
    /// structure.  If `false`, this packets payload is a continuation of a structure which began
    /// in an earlier packet within the transport stream.
    #[inline]
    pub fn payload_unit_start_indicator(&self) -> bool {
        self.buf[1] & 0b01000000 != 0
    }

    pub fn transport_priority(&self) -> bool {
        self.buf[1] & 0b00100000 != 0
    }

    /// The sub-stream to which a particular packet belongs is indicated by this Packet Identifier
    /// value.
    #[inline]
    pub fn pid(&self) -> u16 {
        u16::from(self.buf[1] & 0b00011111) << 8 | u16::from(self.buf[2])
    }

    pub fn transport_scrambling_control(&self) -> TransportScramblingControl {
        TransportScramblingControl::from(self.buf[3] >> 6 & 0b11)
    }

    /// The returned enum value indicates if `adaptation_field()`, `payload()` or both will return
    /// something.
    #[inline]
    pub fn adaptation_control(&self) -> AdaptationControl {
        AdaptationControl::from(self.buf[3] >> 4 & 0b11)
    }

    /// Each packet with a given `pid()` value within a transport stream should have a continuity
    /// counter value which increases by 1 from the last counter value seen.  Unexpected continuity
    /// counter values allow the receiver of the transport stream to detect discontinuities in the
    /// stream (e.g. due to data loss during transmission).
    #[inline]
    pub fn continuity_counter(&self) -> ContinuityCounter {
        ContinuityCounter::new(self.buf[3] & 0b00001111)
    }

    fn adaptation_field_length(&self) -> usize {
        self.buf[4] as usize
    }

    /// An `AdaptationField` contains additional packet headers that may be present in the packet.
    pub fn adaptation_field(&self) -> Option<AdaptationField> {
        match self.adaptation_control() {
            AdaptationControl::Reserved | AdaptationControl::PayloadOnly => None,
            AdaptationControl::AdaptationFieldOnly => {
                let len = self.adaptation_field_length();
                if len != (PACKET_SIZE - ADAPTATION_FIELD_OFFSET) {
                    println!(
                        "invalid adaptation_field_length for AdaptationFieldOnly: {}",
                        len
                    );
                    // TODO: Option<Result<AdaptationField>> instead?
                    return None;
                }
                Some(self.mk_af(len))
            }
            AdaptationControl::AdaptationFieldAndPayload => {
                let len = self.adaptation_field_length();
                if len > 182 {
                    println!(
                        "invalid adaptation_field_length for AdaptationFieldAndPayload: {}",
                        len
                    );
                    // TODO: Option<Result<AdaptationField>> instead?
                    return None;
                }
                Some(self.mk_af(len))
            }
        }
    }

    fn mk_af(&self, len: usize) -> AdaptationField {
        AdaptationField::new(
            &self.buf[ADAPTATION_FIELD_OFFSET..ADAPTATION_FIELD_OFFSET + len],
        )
    }

    /// The data contained within the packet, not including the packet headers.
    /// Not all packets have a payload, and `None` is returned if `adaptation_control()` indicates
    /// that no payload is present.  None may also be returned if the packet is malformed.
    /// If `Some` payload is returned, it is guaranteed not to be an empty slice.
    #[inline(always)]
    pub fn payload(&self) -> Option<&'buf [u8]> {
        match self.adaptation_control() {
            AdaptationControl::Reserved | AdaptationControl::AdaptationFieldOnly => None,
            AdaptationControl::PayloadOnly | AdaptationControl::AdaptationFieldAndPayload => self.mk_payload(),
        }
    }

    #[inline]
    fn mk_payload(&self) -> Option<&'buf [u8]> {
        let offset = self.content_offset();
        if offset == self.buf.len() {
            println!("no payload data present");
            None
        } else if offset > self.buf.len() {
            println!("adaptation_field_length {} too large", self.adaptation_field_length());
            None
        } else {
            Some(&self.buf[offset..])
        }
    }

    // borrow a reference to the underlying buffer of this packet
    pub fn buffer(&self) -> &'buf[u8] {
        self.buf
    }

    fn content_offset(&self) -> usize {
        match self.adaptation_control() {
            AdaptationControl::Reserved |
            AdaptationControl::PayloadOnly => FIXED_HEADER_SIZE,
            AdaptationControl::AdaptationFieldOnly |
            AdaptationControl::AdaptationFieldAndPayload => {
                ADAPTATION_FIELD_OFFSET + self.adaptation_field_length()
            }
        }
    }
}

/// trait for objects which process transport stream packets
pub trait PacketConsumer<Ret> {
    fn consume(&mut self, pk: Packet) -> Option<Ret>;
}

#[cfg(test)]
mod test {
    use packet::*;
    use pes;

    #[test]
    #[should_panic]
    fn zero_len() {
        let buf = [0u8; 0];
        Packet::new(&buf[..]);
    }

    #[test]
    fn test_xmas_tree() {
        let mut buf = [0xffu8; self::PACKET_SIZE];
        buf[0] = self::SYNC_BYTE;
        buf[4] = 28; // adaptation_field_length
        buf[19] = 1; // transport_private_data_length
        buf[21] = 11; // adaptation_field_extension_length
        let pk = Packet::new(&buf[..]);
        assert_eq!(pk.pid(), 0b1111111111111);
        assert!(pk.transport_error_indicator());
        assert!(pk.payload_unit_start_indicator());
        assert!(pk.transport_priority());
        assert_eq!(
            pk.transport_scrambling_control(),
            TransportScramblingControl::Undefined(3)
        );
        assert_eq!(
            pk.adaptation_control(),
            AdaptationControl::AdaptationFieldAndPayload
        );
        assert_eq!(pk.continuity_counter().count(), 0b1111);
        assert!(pk.adaptation_field().is_some());
        let ad = pk.adaptation_field().unwrap();
        assert!(ad.discontinuity_indicator());
        assert_eq!(ad.pcr(), Ok(PCR::from_parts(0b1_1111_1111_1111_1111_1111_1111_1111_1111, 0b1_1111_1111)));
        assert_eq!(1234 * 300 + 56, u64::from(PCR::from_parts(1234, 56)));
        assert_eq!(ad.opcr(), Ok(PCR::from_parts(0b1_1111_1111_1111_1111_1111_1111_1111_1111, 0b1_1111_1111)));
        assert_eq!(ad.splice_countdown(), Ok(0b11111111));
        let expected_data = [0xff];
        assert_eq!(ad.transport_private_data(), Ok(&expected_data[..]));
        let ext = ad.adaptation_field_extension().unwrap();
        assert_eq!(ext.ltw_offset(), Ok(Some(0b0111_1111_1111_1111)));
        assert_eq!(ext.piecewise_rate(), Ok(0b0011_1111_1111_1111_1111_1111));
        assert_eq!(ext.seamless_splice(), Ok(SeamlessSplice{ splice_type: 0b1111, dts_next_au: pes::Timestamp::from_u64(0b1_1111_1111_1111_1111_1111_1111_1111_1111)}));
    }
}
