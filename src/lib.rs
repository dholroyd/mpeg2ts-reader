//! Structures for parsing MPEG2 Transport Stream data, per the _ISO/IEC 13818-1_ standard.
//!
//! # Design principals
//!
//!  * *Avoid copying and allocating* if possible.  Most of the implementation works by borrowing
//!    slices of the underlying byte buffer.  The implementation tries to avoid buffering up
//!    intermediate data where practical.
//!  * *Non-blocking*.  It should be possible to integrate this library into a system non-blocking
//!    event-loop.  The caller has to 'push' data.
//!  * *Extensible*.  The standard calls out a number of 'reserved values' and other points of
//!    extension.  This library should make it possible for other crates to implement such
//!    extensions.
//!  * *Minimal*.  Lots of commonly used Transport Stream functionality is specified in standards
//!    from by ISDB / DVB / ATSC / SCTE / etc. and not in 13818-1 itself.  I hope support for these
//!    features can be added via external crates.  (None implemented any yet!)
//!  * *Transport Neutral*.  There is currently no code here supporting consuming from files or the
//!    network.  The APIs accept `&[u8]`, and the caller handles providing the data from wherever.
//!
//!
//! # Planned API changes
//!
//! - Add 'context' objects
//!   - Real usage will likely need to thread context objects through the API in order to
//!     track application-specific details.
//!   - Currently mutable state is stored in the instance for each type of syntax parser, and
//!     it would be nice to explore extracting this out into parser-specific context types
//! - Event generation / remove `warn!()`
//!   - currently problems are reported to the `log` crate
//!   - I would much prefer a way to emit 'events' for interesting data that can't just be an error
//!     return value, and to not have any logging code mixed with parsing logic
//! - General
//!   - lots of places return `Option` but should return `Result` and a descriptive error

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms, future_incompatible, missing_docs)]

pub mod packet;
#[macro_use]
pub mod demultiplex;
pub mod descriptor;
pub mod mpegts_crc;
pub mod pes;
pub mod psi;

use std::fmt;
use std::fmt::Formatter;
// expose access to FormatIdentifier, which is part of the public API of
// descriptor::registration::RegistrationDescriptor
pub use smptera_format_identifiers_rust as smptera;

/// The types of Elementary Stream specified in _ISO/IEC 13818-1_.
///
/// As returned by
/// [`StreamInfo::stream_type()`](psi/pmt/struct.StreamInfo.html#method.stream_type).
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub struct StreamType(pub u8);

impl StreamType {
    // 0x00 reserved
    /// ISO/IEC 11172 Video
    pub const ISO_11172_VIDEO: StreamType = StreamType(0x01);
    /// ITU-T Rec. H.262 | ISO/IEC 13818-2 Video or ISO/IEC 11172-2 constrained parameter video stream
    pub const H262: StreamType = StreamType(0x02);
    /// ISO/IEC 11172 Audio
    pub const ISO_11172_AUDIO: StreamType = StreamType(0x03);
    /// ISO/IEC 13818-3 Audio
    pub const ISO_138183_AUDIO: StreamType = StreamType(0x04);
    /// ITU-T Rec. H.222.0 | ISO/IEC 13818-1 private_sections
    pub const H222_0_PRIVATE_SECTIONS: StreamType = StreamType(0x05);
    /// ITU-T Rec. H.222.0 | ISO/IEC 13818-1 PES packets containing private data
    pub const H222_0_PES_PRIVATE_DATA: StreamType = StreamType(0x06);
    /// ISO/IEC 13522 MHEG
    pub const MHEG: StreamType = StreamType(0x07);
    /// ITU-T Rec. H.222.0 | ISO/IEC 13818-1 Annex A DSM-CC
    pub const H222_0_DSM_CC: StreamType = StreamType(0x08);
    /// ITU-T Rec. H.222.1
    pub const H2221: StreamType = StreamType(0x09);
    /// ISO/IEC 13818-6 DSM CC multiprotocol encapsulation
    pub const ISO_13818_6_MULTIPROTOCOL_ENCAPSULATION: StreamType = StreamType(0x0A);
    /// ISO/IEC 13818-6 DSM CC U-N messages
    pub const DSMCC_UN_MESSAGES: StreamType = StreamType(0x0B);
    /// ISO/IEC 13818-6 DSM CC stream descriptors
    pub const DSMCC_STREAM_DESCRIPTORS: StreamType = StreamType(0x0C);
    /// ISO/IEC 13818-6 DSM CC tabled data
    pub const DSMCC_SECTIONS: StreamType = StreamType(0x0D);
    /// ITU-T Rec. H.222.0 | ISO/IEC 13818-1 auxiliary
    pub const H222_0_AUXILIARY: StreamType = StreamType(0x0E);
    /// ISO/IEC 13818-7 Audio with ADTS transport syntax
    pub const ADTS: StreamType = StreamType(0x0F);
    /// ISO/IEC 14496-2 Visual
    pub const ISO_14496_2_VISUAL: StreamType = StreamType(0x10);
    /// ISO/IEC 14496-3 Audio with the LATM transport syntax as defined in ISO/IEC 14496-3 / AMD 1
    pub const LATM: StreamType = StreamType(0x11);
    /// ISO/IEC 14496-1 SL-packetized stream or FlexMux stream carried in PES packets
    pub const FLEX_MUX_PES: StreamType = StreamType(0x12);
    /// ISO/IEC 14496-1 SL-packetized stream or FlexMux stream carried in ISO/IEC14496_sections.
    pub const FLEX_MUX_ISO_14496_SECTIONS: StreamType = StreamType(0x13);
    /// ISO/IEC 13818-6 Synchronized Download Protocol
    pub const SYNCHRONIZED_DOWNLOAD_PROTOCOL: StreamType = StreamType(0x14);
    /// Metadata carried in PES packets
    pub const METADATA_IN_PES: StreamType = StreamType(0x15);
    /// Metadata carried in metadata_sections
    pub const METADATA_IN_METADATA_SECTIONS: StreamType = StreamType(0x16);
    /// Metadata carried in ISO/IEC 13818-6 Data Carousel
    pub const DSMCC_DATA_CAROUSEL_METADATA: StreamType = StreamType(0x17);
    /// Metadata carried in ISO/IEC 13818-6 Object Carousel
    pub const DSMCC_OBJECT_CAROUSEL_METADATA: StreamType = StreamType(0x18);
    /// Metadata carried in ISO/IEC 13818-6 Synchronized Download Protocol
    pub const SYNCHRONIZED_DOWNLOAD_PROTOCOL_METADATA: StreamType = StreamType(0x19);
    /// IPMP stream (defined in ISO/IEC 13818-11, MPEG-2 IPMP)
    pub const IPMP: StreamType = StreamType(0x1a);
    /// AVC video stream as defined in ITU-T Rec. H.264 | ISO/IEC 14496-10 Video
    pub const H264: StreamType = StreamType(0x1b);
    /// ISO/IEC 14496-3 Audio, without using any additional transport syntax, such as DST, ALS and SLS
    pub const AUDIO_WITHOUT_TRANSPORT_SYNTAX: StreamType = StreamType(0x1c);
    /// ISO/IEC 14496-17 Text
    pub const ISO_14496_17_TEXT: StreamType = StreamType(0x1d);
    // 0x1e-0x23 reserved
    /// ITU-T Rec. H.265 and ISO/IEC 23008-2
    pub const H265: StreamType = StreamType(0x24);
    // 0x26-0x41 reserved
    /// Chinese Video Standard
    pub const CHINESE_VIDEO_STANDARD: StreamType = StreamType(0x42);
    // 0x43-0x7f reserved
    // 0x80 privately defined
    /// Dolby Digital (AC-3) audio for ATSC
    pub const ATSC_DOLBY_DIGITAL_AUDIO: StreamType = StreamType(0x81);
    // 0x82-0x94 privately defined
    /// ATSC Data Service Table, Network Resources Table
    pub const ATSC_DSMCC_NETWORK_RESOURCES_TABLE: StreamType = StreamType(0x95);
    // 0x95-0xc1 privately defined
    /// PES packets containing ATSC streaming synchronous data
    pub const ATSC_DSMCC_SYNCHRONOUS_DATA: StreamType = StreamType(0xc2);
    // 0xc3-0xff privately defined,

    /// `true` if packets of a stream with this `stream_type` will carry data in Packetized
    /// Elementary Stream format.
    pub fn is_pes(self) -> bool {
        matches!(
            self,
            StreamType::ISO_11172_VIDEO
                | StreamType::H262
                | StreamType::ISO_11172_AUDIO
                | StreamType::ISO_138183_AUDIO
                | StreamType::H222_0_PES_PRIVATE_DATA
                | StreamType::MHEG
                | StreamType::H222_0_DSM_CC
                | StreamType::H2221
                | StreamType::ISO_13818_6_MULTIPROTOCOL_ENCAPSULATION
                | StreamType::DSMCC_UN_MESSAGES
                | StreamType::DSMCC_STREAM_DESCRIPTORS
                | StreamType::DSMCC_SECTIONS
                | StreamType::H222_0_AUXILIARY
                | StreamType::ADTS
                | StreamType::ISO_14496_2_VISUAL
                | StreamType::LATM
                | StreamType::FLEX_MUX_PES
                | StreamType::FLEX_MUX_ISO_14496_SECTIONS
                | StreamType::SYNCHRONIZED_DOWNLOAD_PROTOCOL
                | StreamType::METADATA_IN_PES
                | StreamType::METADATA_IN_METADATA_SECTIONS
                | StreamType::DSMCC_DATA_CAROUSEL_METADATA
                | StreamType::DSMCC_OBJECT_CAROUSEL_METADATA
                | StreamType::SYNCHRONIZED_DOWNLOAD_PROTOCOL_METADATA
                | StreamType::IPMP
                | StreamType::H264
                | StreamType::AUDIO_WITHOUT_TRANSPORT_SYNTAX
                | StreamType::ISO_14496_17_TEXT
                | StreamType::H265
                | StreamType::CHINESE_VIDEO_STANDARD
                | StreamType::ATSC_DOLBY_DIGITAL_AUDIO
                | StreamType::ATSC_DSMCC_NETWORK_RESOURCES_TABLE
                | StreamType::ATSC_DSMCC_SYNCHRONOUS_DATA
        )
    }
}
impl fmt::Debug for StreamType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            StreamType::ISO_11172_VIDEO => f.write_str("ISO_11172_VIDEO"),
            StreamType::H262 => f.write_str("H262"),
            StreamType::ISO_11172_AUDIO => f.write_str("ISO_11172_AUDIO"),
            StreamType::ISO_138183_AUDIO => f.write_str("ISO_138183_AUDIO"),
            StreamType::H222_0_PRIVATE_SECTIONS => f.write_str("H222_0_PRIVATE_SECTIONS"),
            StreamType::H222_0_PES_PRIVATE_DATA => f.write_str("H222_0_PES_PRIVATE_DATA"),
            StreamType::MHEG => f.write_str("MHEG"),
            StreamType::H222_0_DSM_CC => f.write_str("H222_0_DSM_CC"),
            StreamType::H2221 => f.write_str("H2221"),
            StreamType::ISO_13818_6_MULTIPROTOCOL_ENCAPSULATION => {
                f.write_str("ISO_13818_6_MULTIPROTOCOL_ENCAPSULATION")
            }
            StreamType::DSMCC_UN_MESSAGES => f.write_str("DSMCC_UN_MESSAGES"),
            StreamType::DSMCC_STREAM_DESCRIPTORS => f.write_str("DSMCC_STREAM_DESCRIPTORS"),
            StreamType::DSMCC_SECTIONS => f.write_str("DSMCC_SECTIONS"),
            StreamType::H222_0_AUXILIARY => f.write_str("H222_0_AUXILIARY"),
            StreamType::ADTS => f.write_str("ADTS"),
            StreamType::ISO_14496_2_VISUAL => f.write_str("ISO_14496_2_VISUAL"),
            StreamType::LATM => f.write_str("LATM"),
            StreamType::FLEX_MUX_PES => f.write_str("FLEX_MUX_PES"),
            StreamType::FLEX_MUX_ISO_14496_SECTIONS => f.write_str("FLEX_MUX_ISO_14496_SECTIONS"),
            StreamType::SYNCHRONIZED_DOWNLOAD_PROTOCOL => {
                f.write_str("SYNCHRONIZED_DOWNLOAD_PROTOCOL")
            }
            StreamType::METADATA_IN_PES => f.write_str("METADATA_IN_PES"),
            StreamType::METADATA_IN_METADATA_SECTIONS => {
                f.write_str("METADATA_IN_METADATA_SECTIONS")
            }
            StreamType::DSMCC_DATA_CAROUSEL_METADATA => f.write_str("DSMCC_DATA_CAROUSEL_METADATA"),
            StreamType::DSMCC_OBJECT_CAROUSEL_METADATA => {
                f.write_str("DSMCC_OBJECT_CAROUSEL_METADATA")
            }
            StreamType::SYNCHRONIZED_DOWNLOAD_PROTOCOL_METADATA => {
                f.write_str("SYNCHRONIZED_DOWNLOAD_PROTOCOL_METADATA")
            }
            StreamType::IPMP => f.write_str("IPMP"),
            StreamType::H264 => f.write_str("H264"),
            StreamType::AUDIO_WITHOUT_TRANSPORT_SYNTAX => {
                f.write_str("AUDIO_WITHOUT_TRANSPORT_SYNTAX")
            }
            StreamType::ISO_14496_17_TEXT => f.write_str("ISO_14496_17_TEXT"),
            StreamType::H265 => f.write_str("H265"),
            StreamType::CHINESE_VIDEO_STANDARD => f.write_str("CHINESE_VIDEO_STANDARD"),
            StreamType::ATSC_DOLBY_DIGITAL_AUDIO => f.write_str("ATSC_DOLBY_DIGITAL_AUDIO"),
            StreamType::ATSC_DSMCC_NETWORK_RESOURCES_TABLE => {
                f.write_str("ATSC_DSMCC_NETWORK_RESOURCES_TABLE")
            }
            StreamType::ATSC_DSMCC_SYNCHRONOUS_DATA => f.write_str("ATSC_DSMCC_SYNCHRONOUS_DATA"),
            _ => {
                if self.0 >= 0x80 {
                    f.write_fmt(format_args!("PRIVATE({})", self.0))
                } else {
                    f.write_fmt(format_args!("RESERVED({})", self.0))
                }
            }
        }
    }
}

impl From<StreamType> for u8 {
    fn from(val: StreamType) -> Self {
        val.0
    }
}
impl From<u8> for StreamType {
    fn from(val: u8) -> Self {
        StreamType(val)
    }
}

/// The identifier of TS Packets containing 'stuffing' data, with value `0x1fff`
pub const STUFFING_PID: packet::Pid = packet::Pid::new(0x1fff);

#[cfg(test)]
mod test {
    use super::StreamType;

    fn is_reserved(st: &StreamType) -> bool {
        match st.0 {
            0x00 | 0x1e..=0x23 | 0x25..=0x41 | 0x43..=0x7f => true,
            _ => false,
        }
    }

    fn is_private(st: &StreamType) -> bool {
        match st.0 {
            // TODO: 0x95 now public, or is ATSC_DSMCC_NETWORK_RESOURCES_TABLE incorrect?
            0x80 | 0x82..=0x94 | 0x96..=0xc1 | 0xc3..=0xff => true,
            _ => false,
        }
    }

    #[test]
    fn pes() {
        for st in 0..=255 {
            let ty = StreamType(st);
            if ty == StreamType::H222_0_PRIVATE_SECTIONS || is_reserved(&ty) || is_private(&ty) {
                assert!(!ty.is_pes(), "{:?}", ty);
            } else {
                assert!(ty.is_pes(), "{:?}", ty);
            }
            assert_eq!(ty, StreamType::from(u8::from(ty)));
            let _ = format!("{:?}", ty);
        }
    }
}
