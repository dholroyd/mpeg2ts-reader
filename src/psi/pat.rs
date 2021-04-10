//! Types related to the _Program Association Table_

use crate::packet;
use log::warn;

/// The identifier of TS Packets containing Program Association Table sections, with value `0`.
pub const PAT_PID: packet::Pid = packet::Pid::new(0);

/// Identifiers related to a specific program within the Transport Stream
#[derive(Clone, Debug)]
pub enum ProgramDescriptor {
    // TODO: consider renaming 'ProgramAssociationEntry', so as not to cause confusion with types
    //       actually implementing the Descriptor trait (which this type should not do)
    /// this PAT section entry describes where the Network Information Table will be found
    Network {
        /// The PID of NIT section packets
        pid: packet::Pid,
    },
    /// this PAT section entry gives the PID and Program Number of a program within this Transport
    /// Stream
    Program {
        /// The program number might represent the 'channel' of the program
        program_number: u16,
        /// The PID where PMT sections for this program can be found
        pid: packet::Pid,
    },
}

impl ProgramDescriptor {
    /// panics if fewer than 4 bytes are provided
    pub fn from_bytes(data: &[u8]) -> ProgramDescriptor {
        let program_number = (u16::from(data[0]) << 8) | u16::from(data[1]);
        let pid = packet::Pid::new((u16::from(data[2]) & 0b0001_1111) << 8 | u16::from(data[3]));
        if program_number == 0 {
            ProgramDescriptor::Network { pid }
        } else {
            ProgramDescriptor::Program {
                program_number,
                pid,
            }
        }
    }

    /// produces the Pid of either the NIT or PMT, depending on which type of `ProgramDescriptor`
    /// this is
    pub fn pid(&self) -> packet::Pid {
        match *self {
            ProgramDescriptor::Network { pid } => pid,
            ProgramDescriptor::Program { pid, .. } => pid,
        }
    }
}

/// Sections of the _Program Association Table_ give details of the programs within a transport
/// stream.  There may be only one program, or in the case of a broadcast multiplex, there may
/// be many.
#[derive(Clone, Debug)]
pub struct PatSection<'buf> {
    data: &'buf [u8],
}
impl<'buf> PatSection<'buf> {
    /// Create a `PatSection`, wrapping the given slice, whose methods can parse the section's
    /// fields
    pub fn new(data: &'buf [u8]) -> PatSection<'buf> {
        PatSection { data }
    }
    /// Returns an iterator over the entries in this program association table section.
    pub fn programs(&self) -> impl Iterator<Item = ProgramDescriptor> + 'buf {
        ProgramIter { buf: self.data }
    }
}

/// Iterate over the list of programs in a `PatSection`.
struct ProgramIter<'buf> {
    buf: &'buf [u8],
}
impl<'buf> Iterator for ProgramIter<'buf> {
    type Item = ProgramDescriptor;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }
        if self.buf.len() < 4 {
            warn!(
                "too few bytes remaining for PAT descriptor: {}",
                self.buf.len()
            );
            return None;
        }
        let (head, tail) = self.buf.split_at(4);
        self.buf = tail;
        Some(ProgramDescriptor::from_bytes(head))
    }
}
