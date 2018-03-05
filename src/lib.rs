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
//! - Event generation / remove `println!()`
//!   - currently all errors are sent to stdout, which it no way to do things
//!   - need a way to emit 'events' for interesting data that can't just be a return-value
//! - General
//!   - lots of places return `Option` but should return `Result` and a descriptive error
//!
//! # Rust nightly
//!
//! Currently uses _nightly_ in order to make use of `impl Trait`.  Will probably switch to
//! _stable_ once `impl Trait` stabilises.

#![feature(universal_impl_trait)]

extern crate hexdump;
extern crate byteorder;
extern crate data_encoding;
extern crate bitreader;
#[cfg(test)]
#[macro_use]
extern crate matches;
#[cfg(test)]
extern crate bitstream_io;
extern crate fixedbitset;

pub mod packet;
pub mod demultiplex;
pub mod psi;
pub mod pes;
mod mpegts_crc;

#[derive(Debug,PartialEq,Eq,Hash,Clone,Copy)]
pub enum StreamType {
	// 0x00 reserved
	Iso11172Video,
	H262,
	Iso11172Audio,
	Iso138183Audio,
	H2220PrivateSections,
	H2220PesPrivateData,
	Mheg,
	H2220DsmCc,
	H2221,
	Iso138186MultiprotocolEncapsulation,
	DsmccUnMessages,
	DsmccStreamDescriptors,
	DsmccSections,
	H2220Auxiliary,
	Adts,
	Iso144962Visual,
	Latm,
	FlexMuxPes,
	FlexMuxIso14496Sections,
	SynchronizedDownloadProtocol,
	MetadataInPes,
	MetadataInMetadataSections,
	DsmccDataCarouselMetadata,
	DsmccObjectCarouselMetadata,
	SynchronizedDownloadProtocolMetadata,
	Ipmp,
	H264,
	// 0x1c-0x23 reserved
	H265,
	// 0x26-0x41 reserved
	ChineseVideoStandard,
	// 0x43-0x7f reserved
	// 0x80 privately defined
	AtscDolbyDigitalAudio,
	// 0x82-0x94 privately defined
	AtscDsmccNetworkResourcesTable,
	// 0x95-0xc1 privately defined
	AtscDsmccSynchronousData,
	// 0xc3-0xff privately defined,
    Private(u8),
    Reserved(u8),
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
