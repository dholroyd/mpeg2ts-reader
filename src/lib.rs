//! Structures for parsing MPEG2 Transport Stream data
//!
//! # Design Principals
//!
//!  * *Avoid copying and allocating* if possible.  Most of the implementation works by borrowing
//!    slices of the underlying byte buffer.  The implementation tries to avoid buffering up
//!    intermediate data were practical.
//!  * *Non-blocking*.  It should be possible to integrate this library into a system non-blocking
//!    event-loop.  The caller has to 'push' data.
//!  * *Extensible*.  The standard calls out a number of 'reserved values' and other points of
//!    extension.  This library should make it possible for other crates to implement such
//!    extensions.
//!  * *Transport Neutral*.  There is currently no code here supporting consuming from files or the
//!    network.  The APIs accept `&[u8]`, and the caller handles providing the data from wherever.
//!
//!
//! # TODO
//!
//! - General API
//!   - lots of places return `Option` but should probably return `Result`
//! - TS packet
//!   - Adaptation field is mostly unimplemented
//! - PSI tables
//!   - PAT and PMT mostly there, but no other tables currently implemented
//! - Elementary Stream syntax
//!   - most of the optional header fields (except PTS/DTS) are unimplemented

extern crate hexdump;
extern crate byteorder;
extern crate data_encoding;
extern crate amphora;
extern crate bitreader;
#[cfg(test)]
#[macro_use]
extern crate matches;
#[cfg(test)]
extern crate bitstream_io;
extern crate fixedbitset;

pub mod unpacketise;
pub mod packet;
pub mod demultiplex;
pub mod psi;
pub mod pes;
mod mpegts_crc;
