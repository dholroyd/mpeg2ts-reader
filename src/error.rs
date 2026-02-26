//! Error types for reporting problems found during transport stream parsing.
//!
//! The [`ErrorSink`] trait provides a callback mechanism for receiving [`DemuxError`]
//! notifications.  Types implementing [`DemuxContext`](crate::demultiplex::DemuxContext) must
//! also implement `ErrorSink`.

use crate::packet;
use crate::psi::pat;
use crate::psi::pmt::PmtError;
use std::fmt;

// Re-import for use in warning variants
use crate::packet::PacketError;

/// A problem detected while parsing transport stream data.
///
/// These errors are delivered to [`ErrorSink::error()`] when the application opts in
/// by overriding that method.  Each variant carries enough context (PID, table ID, etc.) to
/// identify the source of the problem without heap allocation.
#[derive(Debug, PartialEq, Eq)]
pub enum DemuxError {
    /// The transport_error_indicator was set in a packet header.
    TransportError {
        /// The PID of the packet with the error indicator set.
        pid: packet::Pid,
    },
    /// A scrambled packet was encountered and dropped (descrambling is not supported).
    ScrambledPacket {
        /// The PID of the scrambled packet.
        pid: packet::Pid,
    },
    /// A PSI section had a table_id that doesn't match the expected value.
    InvalidTableId {
        /// The PID on which the section was received.
        pid: packet::Pid,
        /// The table_id value that was expected.
        expected: u8,
        /// The table_id value that was found.
        actual: u8,
    },
    /// A PSI section's section_length exceeds the allowed limit for its table type.
    SectionTooLarge {
        /// The PID on which the section was received.
        pid: packet::Pid,
        /// The table_id of the section.
        table_id: u8,
        /// The section_length value found.
        length: usize,
        /// The maximum allowed section_length.
        limit: usize,
    },
    /// An error occurred while parsing a PMT section's stream descriptors.
    PmtParseError {
        /// The PID on which the PMT was received.
        pid: packet::Pid,
        /// The parsing error.
        error: PmtError,
    },
    /// A PES packet was received without a preceding payload_start_indicator.
    MissingPayloadStartIndicator {
        /// The PID of the elementary stream.
        pid: packet::Pid,
    },
    /// An error occurred while parsing a PES packet header.
    PesHeaderParseError {
        /// The PID of the elementary stream.
        pid: packet::Pid,
    },
    /// A PSI section's CRC32 check failed.
    CrcCheckFailed {
        /// The PID on which the section was received.
        pid: packet::Pid,
        /// The table_id of the section.
        table_id: u8,
    },
    /// A PSI section is too small to contain the CRC field.
    SectionTooSmallForCrc {
        /// The PID on which the section was received.
        pid: packet::Pid,
        /// The table_id of the section.
        table_id: u8,
        /// The actual size of the section data in bytes.
        actual: usize,
    },
    /// The section_syntax_indicator had an unexpected value.
    UnexpectedSectionSyntaxIndicator {
        /// The PID on which the section was received.
        pid: packet::Pid,
        /// The table_id of the section.
        table_id: u8,
    },
    /// Section data is shorter than the minimum required.
    SectionDataTooShort {
        /// The PID on which the section was received.
        pid: packet::Pid,
        /// The table_id of the section.
        table_id: u8,
        /// The actual size of the section data.
        actual: usize,
        /// The minimum required size.
        minimum: usize,
    },
    /// A PSI section's section_length exceeds the generic PSI limit (4093).
    PsiSectionTooLarge {
        /// The PID on which the section was received.
        pid: packet::Pid,
        /// The table_id of the section.
        table_id: u8,
        /// The section_length value found.
        length: usize,
        /// The maximum allowed section_length.
        limit: usize,
    },
    /// The pointer field in a PSI packet points beyond the available data.
    PsiPointerOutOfBounds {
        /// The PID of the PSI packet.
        pid: packet::Pid,
    },
    /// A PSI section header is too short to parse.
    SectionHeaderTooShort {
        /// The PID of the PSI packet.
        pid: packet::Pid,
    },
    /// A PSI packet has no payload.
    NoPayloadInPsiPacket {
        /// The PID of the PSI packet.
        pid: packet::Pid,
    },
    /// Continuation data arrived after a section was already complete.
    ExtraDataAfterSectionComplete {
        /// The PID on which the data was received.
        pid: packet::Pid,
    },
    /// An error occurred while parsing a PAT entry.
    PatEntryParseError {
        /// The parsing error.
        error: pat::PatError,
    },
    /// A packet's adaptation field has an invalid length.
    MalformedAdaptationField {
        /// The PID of the packet.
        pid: packet::Pid,
        /// The parsing error.
        error: PacketError,
    },
    /// A packet's payload offset is out of bounds.
    MalformedPayload {
        /// The PID of the packet.
        pid: packet::Pid,
        /// The parsing error.
        error: PacketError,
    },
}
impl fmt::Display for DemuxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DemuxError::TransportError { pid } => {
                write!(f, "{:?}: transport_error_indicator", pid)
            }
            DemuxError::ScrambledPacket { pid } => {
                write!(f, "{:?}: dropping scrambled packet", pid)
            }
            DemuxError::InvalidTableId {
                pid,
                expected,
                actual,
            } => write!(
                f,
                "{:?}: expected table_id {:#x}, got {:#x}",
                pid, expected, actual
            ),
            DemuxError::SectionTooLarge {
                pid,
                table_id,
                length,
                limit,
            } => write!(
                f,
                "{:?}: table_id {:#x} section_length={} exceeds limit {}",
                pid, table_id, length, limit
            ),
            DemuxError::PmtParseError { pid, error } => {
                write!(f, "{:?}: PMT parse error: {:?}", pid, error)
            }
            DemuxError::MissingPayloadStartIndicator { pid } => write!(
                f,
                "{:?}: ignoring elementary stream content without payload_start_indicator",
                pid
            ),
            DemuxError::PesHeaderParseError { pid } => {
                write!(f, "{:?}: PES header parse error", pid)
            }
            DemuxError::CrcCheckFailed { pid, table_id } => {
                write!(f, "{:?}: table_id {:#x}: CRC check failed", pid, table_id)
            }
            DemuxError::SectionTooSmallForCrc {
                pid,
                table_id,
                actual,
            } => write!(
                f,
                "{:?}: table_id {:#x}: section too small for CRC ({} bytes)",
                pid, table_id, actual
            ),
            DemuxError::UnexpectedSectionSyntaxIndicator { pid, table_id } => write!(
                f,
                "{:?}: table_id {:#x}: unexpected section_syntax_indicator value",
                pid, table_id
            ),
            DemuxError::SectionDataTooShort {
                pid,
                table_id,
                actual,
                minimum,
            } => write!(
                f,
                "{:?}: table_id {:#x}: section data too short ({} bytes, need {})",
                pid, table_id, actual, minimum
            ),
            DemuxError::PsiSectionTooLarge {
                pid,
                table_id,
                length,
                limit,
            } => write!(
                f,
                "{:?}: table_id {:#x}: section_length={} exceeds PSI limit {}",
                pid, table_id, length, limit
            ),
            DemuxError::PsiPointerOutOfBounds { pid } => {
                write!(f, "{:?}: PSI pointer field out of bounds", pid)
            }
            DemuxError::SectionHeaderTooShort { pid } => {
                write!(f, "{:?}: section header too short", pid)
            }
            DemuxError::NoPayloadInPsiPacket { pid } => {
                write!(f, "{:?}: no payload in PSI packet", pid)
            }
            DemuxError::ExtraDataAfterSectionComplete { pid } => {
                write!(f, "{:?}: extra data after section complete", pid)
            }
            DemuxError::PatEntryParseError { error } => {
                write!(f, "PAT entry parse error: {}", error)
            }
            DemuxError::MalformedAdaptationField { pid, error } => {
                write!(f, "{:?}: malformed adaptation field: {}", pid, error)
            }
            DemuxError::MalformedPayload { pid, error } => {
                write!(f, "{:?}: malformed payload: {}", pid, error)
            }
        }
    }
}

/// Trait for types that receive error reports about transport stream problems.
///
/// Implement this trait on your context type to receive [`DemuxError`] notifications.
/// The default implementation does nothing, and monomorphization eliminates the call
/// entirely in release builds when the default is not overridden.
pub trait ErrorSink {
    /// Called when a syntax problem is found in the transport stream.
    ///
    /// The default implementation does nothing.
    #[inline(always)]
    fn error(&mut self, _error: DemuxError) {}
}

/// No-op implementation of `ErrorSink` for unit type, useful in tests.
impl ErrorSink for () {}
