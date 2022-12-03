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

/// The types of Elementary Stream specified in _ISO/IEC 13818-1_.
///
/// As returned by
/// [`StreamInfo::stream_type()`](psi/pmt/struct.StreamInfo.html#method.stream_type).
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum StreamType {
    // 0x00 reserved
    /// ISO/IEC 11172 Video
    Iso11172Video,
    /// ITU-T Rec. H.262 | ISO/IEC 13818-2 Video or ISO/IEC 11172-2 constrained parameter video stream
    H262,
    /// ISO/IEC 11172 Audio
    Iso11172Audio,
    /// ISO/IEC 13818-3 Audio
    Iso138183Audio,
    /// ITU-T Rec. H.222.0 | ISO/IEC 13818-1 private_sections
    H2220PrivateSections,
    /// ITU-T Rec. H.222.0 | ISO/IEC 13818-1 PES packets containing private data
    H2220PesPrivateData,
    /// ISO/IEC 13522 MHEG
    Mheg,
    /// ITU-T Rec. H.222.0 | ISO/IEC 13818-1 Annex A DSM-CC
    H2220DsmCc,
    /// ITU-T Rec. H.222.1
    H2221,
    /// ISO/IEC 13818-6 DSM CC multiprotocol encapsulation
    Iso138186MultiprotocolEncapsulation,
    /// ISO/IEC 13818-6 DSM CC U-N messages
    DsmccUnMessages,
    /// ISO/IEC 13818-6 DSM CC stream descriptors
    DsmccStreamDescriptors,
    /// ISO/IEC 13818-6 DSM CC tabled data
    DsmccSections,
    /// ITU-T Rec. H.222.0 | ISO/IEC 13818-1 auxiliary
    H2220Auxiliary,
    /// ISO/IEC 13818-7 Audio with ADTS transport syntax
    Adts,
    /// ISO/IEC 14496-2 Visual
    Iso144962Visual,
    /// ISO/IEC 14496-3 Audio with the LATM transport syntax as defined in ISO/IEC 14496-3 / AMD 1
    Latm,
    /// ISO/IEC 14496-1 SL-packetized stream or FlexMux stream carried in PES packets
    FlexMuxPes,
    /// ISO/IEC 14496-1 SL-packetized stream or FlexMux stream carried in ISO/IEC14496_sections.
    FlexMuxIso14496Sections,
    /// ISO/IEC 13818-6 Synchronized Download Protocol
    SynchronizedDownloadProtocol,
    /// Metadata carried in PES packets
    MetadataInPes,
    /// Metadata carried in metadata_sections
    MetadataInMetadataSections,
    /// Metadata carried in ISO/IEC 13818-6 Data Carousel
    DsmccDataCarouselMetadata,
    /// Metadata carried in ISO/IEC 13818-6 Object Carousel
    DsmccObjectCarouselMetadata,
    /// Metadata carried in ISO/IEC 13818-6 Synchronized Download Protocol
    SynchronizedDownloadProtocolMetadata,
    /// IPMP stream (defined in ISO/IEC 13818-11, MPEG-2 IPMP)
    Ipmp,
    /// AVC video stream as defined in ITU-T Rec. H.264 | ISO/IEC 14496-10 Video
    H264,
    /// ISO/IEC 14496-3 Audio, without using any additional transport syntax, such as DST, ALS and SLS
    AudioWithoutTransportSyntax,
    /// ISO/IEC 14496-17 Text
    Iso1449617text,
    // 0x1e-0x23 reserved
    /// ITU-T Rec. H.265 and ISO/IEC 23008-2
    H265,
    // 0x26-0x41 reserved
    /// Chinese Video Standard
    ChineseVideoStandard,
    // 0x43-0x7f reserved
    // 0x80 privately defined
    /// Dolby Digital (AC-3) audio for ATSC
    AtscDolbyDigitalAudio,
    // 0x82-0x94 privately defined
    /// ATSC Data Service Table, Network Resources Table
    AtscDsmccNetworkResourcesTable,
    // 0x95-0xc1 privately defined
    /// PES packets containing ATSC streaming synchronous data
    AtscDsmccSynchronousData,
    /// 0xc3-0xff privately defined,
    Private(u8),
    /// Reserved for use in future standards
    Reserved(u8),
}
impl StreamType {
    /// `true` if packets of a stream with this `stream_type` will carry data in Packetized
    /// Elementary Stream format.
    pub fn is_pes(self) -> bool {
        match self {
            StreamType::Iso11172Video
            | StreamType::H262
            | StreamType::Iso11172Audio
            | StreamType::Iso138183Audio
            | StreamType::H2220PesPrivateData
            | StreamType::Mheg
            | StreamType::H2220DsmCc
            | StreamType::H2221
            | StreamType::Iso138186MultiprotocolEncapsulation
            | StreamType::DsmccUnMessages
            | StreamType::DsmccStreamDescriptors
            | StreamType::DsmccSections
            | StreamType::H2220Auxiliary
            | StreamType::Adts
            | StreamType::Iso144962Visual
            | StreamType::Latm
            | StreamType::FlexMuxPes
            | StreamType::FlexMuxIso14496Sections
            | StreamType::SynchronizedDownloadProtocol
            | StreamType::MetadataInPes
            | StreamType::MetadataInMetadataSections
            | StreamType::DsmccDataCarouselMetadata
            | StreamType::DsmccObjectCarouselMetadata
            | StreamType::SynchronizedDownloadProtocolMetadata
            | StreamType::Ipmp
            | StreamType::H264
            | StreamType::AudioWithoutTransportSyntax
            | StreamType::Iso1449617text
            | StreamType::H265
            | StreamType::ChineseVideoStandard
            | StreamType::AtscDolbyDigitalAudio
            | StreamType::AtscDsmccNetworkResourcesTable
            | StreamType::AtscDsmccSynchronousData => true,

            StreamType::H2220PrivateSections | StreamType::Reserved(_) | StreamType::Private(_) => {
                false
            }
        }
    }
}
impl From<u8> for StreamType {
    fn from(val: u8) -> Self {
        match val {
            0x01 => StreamType::Iso11172Video,
            0x02 => StreamType::H262,
            0x03 => StreamType::Iso11172Audio,
            0x04 => StreamType::Iso138183Audio,
            0x05 => StreamType::H2220PrivateSections,
            0x06 => StreamType::H2220PesPrivateData,
            0x07 => StreamType::Mheg,
            0x08 => StreamType::H2220DsmCc,
            0x09 => StreamType::H2221,
            0x0A => StreamType::Iso138186MultiprotocolEncapsulation,
            0x0B => StreamType::DsmccUnMessages,
            0x0C => StreamType::DsmccStreamDescriptors,
            0x0D => StreamType::DsmccSections,
            0x0E => StreamType::H2220Auxiliary,
            0x0F => StreamType::Adts,
            0x10 => StreamType::Iso144962Visual,
            0x11 => StreamType::Latm,
            0x12 => StreamType::FlexMuxPes,
            0x13 => StreamType::FlexMuxIso14496Sections,
            0x14 => StreamType::SynchronizedDownloadProtocol,
            0x15 => StreamType::MetadataInPes,
            0x16 => StreamType::MetadataInMetadataSections,
            0x17 => StreamType::DsmccDataCarouselMetadata,
            0x18 => StreamType::DsmccObjectCarouselMetadata,
            0x19 => StreamType::SynchronizedDownloadProtocolMetadata,
            0x1a => StreamType::Ipmp,
            0x1b => StreamType::H264,
            0x1c => StreamType::AudioWithoutTransportSyntax,
            0x1d => StreamType::Iso1449617text,
            0x24 => StreamType::H265,
            0x42 => StreamType::ChineseVideoStandard,
            0x81 => StreamType::AtscDolbyDigitalAudio,
            0x95 => StreamType::AtscDsmccNetworkResourcesTable,
            0xc2 => StreamType::AtscDsmccSynchronousData,
            _ => {
                if val >= 0x80 {
                    StreamType::Private(val)
                } else {
                    StreamType::Reserved(val)
                }
            }
        }
    }
}

impl From<StreamType> for u8 {
    fn from(val: StreamType) -> Self {
        match val {
            StreamType::Iso11172Video => 0x01,
            StreamType::H262 => 0x02,
            StreamType::Iso11172Audio => 0x03,
            StreamType::Iso138183Audio => 0x04,
            StreamType::H2220PrivateSections => 0x05,
            StreamType::H2220PesPrivateData => 0x06,
            StreamType::Mheg => 0x07,
            StreamType::H2220DsmCc => 0x08,
            StreamType::H2221 => 0x09,
            StreamType::Iso138186MultiprotocolEncapsulation => 0x0A,
            StreamType::DsmccUnMessages => 0x0B,
            StreamType::DsmccStreamDescriptors => 0x0C,
            StreamType::DsmccSections => 0x0D,
            StreamType::H2220Auxiliary => 0x0E,
            StreamType::Adts => 0x0F,
            StreamType::Iso144962Visual => 0x10,
            StreamType::Latm => 0x11,
            StreamType::FlexMuxPes => 0x12,
            StreamType::FlexMuxIso14496Sections => 0x13,
            StreamType::SynchronizedDownloadProtocol => 0x14,
            StreamType::MetadataInPes => 0x15,
            StreamType::MetadataInMetadataSections => 0x16,
            StreamType::DsmccDataCarouselMetadata => 0x17,
            StreamType::DsmccObjectCarouselMetadata => 0x18,
            StreamType::SynchronizedDownloadProtocolMetadata => 0x19,
            StreamType::Ipmp => 0x1a,
            StreamType::H264 => 0x1b,
            StreamType::AudioWithoutTransportSyntax => 0x1c,
            StreamType::Iso1449617text => 0x1d,
            StreamType::H265 => 0x24,
            StreamType::ChineseVideoStandard => 0x42,
            StreamType::AtscDolbyDigitalAudio => 0x81,
            StreamType::AtscDsmccNetworkResourcesTable => 0x95,
            StreamType::AtscDsmccSynchronousData => 0xc2,
            StreamType::Reserved(val) => val,
            StreamType::Private(val) => val,
        }
    }
}

/// The identifier of TS Packets containing 'stuffing' data, with value `0x1fff`
pub const STUFFING_PID: packet::Pid = packet::Pid::new(0x1fff);

#[cfg(test)]
mod test {
    use super::StreamType;

    #[test]
    fn mappings() {
        for st in 0..=255 {
            assert_eq!(st, StreamType::from(st).into())
        }
    }

    #[test]
    fn pes() {
        for st in 0..=255 {
            let ty = StreamType::from(st);
            match ty {
                StreamType::H2220PrivateSections
                | StreamType::Reserved(_)
                | StreamType::Private(_) => assert!(!ty.is_pes(), "{:?}", ty),
                _ => assert!(ty.is_pes(), "{:?}", ty),
            }
        }
    }
}
