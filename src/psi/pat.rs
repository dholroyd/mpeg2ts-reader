//! Types related to the _Program Association Table_

#[derive(Clone,Debug)]
pub enum ProgramDescriptor {
    Network { pid: u16 },
    Program { program_number: u16, pid: u16 },
}

impl ProgramDescriptor {
    /// panics if fewer than 4 bytes are provided
    pub fn from_bytes(data: &[u8]) -> ProgramDescriptor {
        let program_number = (u16::from(data[0]) << 8) | u16::from(data[1]);
        let pid = (u16::from(data[2]) & 0b0001_1111) << 8 | u16::from(data[3]);
        if program_number == 0 {
            ProgramDescriptor::Network { pid }
        } else {
            ProgramDescriptor::Program { program_number, pid }
        }
    }

    pub fn pid(&self) -> u16 {
        match *self {
            ProgramDescriptor::Network { pid } => pid,
            ProgramDescriptor::Program { pid, .. } => pid,
        }
    }
}

/// Sections of the _Program Association Table_ give details of the programs within a transport
/// stream.  There may be only one program, or in the case of a broadcast multiplex, there may
/// be many.
#[derive(Clone,Debug)]
pub struct PatSection<'buf> {
    data: &'buf[u8],
}
impl<'buf> PatSection<'buf> {
    pub fn new(data: &'buf[u8]) -> PatSection<'buf> {
        PatSection {
            data,
        }
    }
    pub fn programs(&self) -> impl Iterator<Item=ProgramDescriptor> + 'buf {
        ProgramIter { buf: &self.data[..] }
    }
}

/// Iterate over the list of programs in a `PatSection`.
struct ProgramIter<'buf> {
    buf: &'buf[u8],
}
impl<'buf> Iterator for ProgramIter<'buf> {
    type Item = ProgramDescriptor;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }
        let (head, tail) = self.buf.split_at(4);
        self.buf = tail;
        Some(ProgramDescriptor::from_bytes(head))
    }
}
