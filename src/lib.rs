//! Structures for parsing MPEG2 Transport Stream data
//!
//! # Design Principals
//!
//!  * *Avoid copying and allocating* if possible.  Most of the implementation works by borrowing
//!    slices of the underlying byte buffer.  The implementation tries to avoid buffering up
//!    intermidiate data were practical.
//!  * *Non-blocking*.  It should be possible to integrate this library into a system non-blocking
//!    event-loop.  The caller has to 'push' data.
//!  * *Extensible*.  The standard calls out a number of 'reserved values' and other points of
//!    extension.  This library should make it possible for other crates to implement such
//!    extensons.
//!
//!
//! # TODO
//!
//! - Adaptation fields
//! - PSI:
//!   - Programe Mapping Table
//!   - and the rest!
//! - Elemetry stream syntax

extern crate hexdump;
extern crate byteorder;
extern crate data_encoding;
extern crate amphora;
extern crate bitreader;
#[cfg(test)]
#[macro_use]
extern crate matches;

pub mod unpacketise;
pub mod packet;
pub mod demultiplex;
pub mod psi;
mod mpegts_crc;
