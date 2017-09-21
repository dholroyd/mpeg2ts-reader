use std::collections::HashMap;
use std::collections::HashSet;
use std::cell::RefCell;
use packet;
use packet::PacketConsumer;
use psi;

// TODO: Pid = u16;

pub struct PatProcessor {
}

impl PatProcessor {
    pub fn new() -> PatProcessor {
        PatProcessor { }
    }
}

impl psi::TableProcessor<PatSection> for PatProcessor {
    fn process(&mut self, table: psi::Table<PatSection>) {
        for i in 0..table.len() {
            println!("  entry {:?}", table.index(i));
        }
        panic!("TODO");
    }
}

#[derive(Clone,Debug)]
struct ProgramDescriptor {
    pub program_number: u16,
    pub reserved: u8,
    pub pid: u16,
}

impl ProgramDescriptor {
    /// panics if fewer than 3 bytes are provided
    pub fn from_bytes(data: &[u8]) -> ProgramDescriptor {
        ProgramDescriptor {
            program_number: ((data[0] as u16) << 8) | (data[1] as u16),
            reserved: data[2] >> 5,
            pid: (data[2] as u16 & 0b00011111) << 8 | (data[3] as u16),
        }
    }
}

#[derive(Clone,Debug)]
pub struct PatSection {
    programes: Vec<ProgramDescriptor>
}

impl psi::TableSection<PatSection> for PatSection {
    fn from_bytes(data: &[u8]) -> Option<PatSection> {
        if data.len() % 4 != 0 {
            println!("section length invalid, must be multiple of 4: {} bytes", data.len());
            return None;
        }
        Some(PatSection {
            programes: data.chunks(4).map(ProgramDescriptor::from_bytes).collect()
        })
    }

}

/// PAT / PMT processing
pub struct Demultiplex {
    processor_by_pid: HashMap<u16, Box<RefCell<packet::PacketConsumer>>>,
    default_processor: Box<RefCell<packet::PacketConsumer>>,
    pat_buffer: psi::SectionPacketConsumer<psi::TableSectionConsumer<PatProcessor, PatSection>>,
}

struct UnhandledPid {
    pids_seen: HashSet<u16>,
}
impl UnhandledPid {
    fn new() -> UnhandledPid {
        UnhandledPid { pids_seen: HashSet::new() }
    }
    fn seen(&mut self, pid: u16) {
        if self.pids_seen.insert(pid) {
            println!("unhandled pid {}", pid);
        }
    }
}
impl packet::PacketConsumer for UnhandledPid {
    fn consume(&mut self, pk: packet::Packet) {
        self.seen(pk.pid())
    }
}

impl Demultiplex {
    pub fn new() -> Demultiplex {
        Demultiplex {
            processor_by_pid: HashMap::new(),
            default_processor: Box::new(RefCell::new(UnhandledPid::new())),
            pat_buffer: psi::SectionPacketConsumer::new(psi::TableSectionConsumer::new(PatProcessor::new())),
        }
    }

    fn pat_packet(&mut self, pk: packet::Packet) {
        self.pat_buffer.consume(pk)
    }

    fn other_packet(&mut self, pk: packet::Packet) {
        match self.processor_by_pid.get(&pk.pid()) {
            Some(processor) => processor.borrow_mut().consume(pk),
            None => self.default_processor.borrow_mut().consume(pk),
        }
    }
}

impl packet::PacketConsumer for Demultiplex {
    fn consume(&mut self, pk: packet::Packet) {
        if pk.pid() == 0 {
            self.pat_packet(pk);
        } else {
            self.other_packet(pk)
        }
    }
}

#[cfg(test)]
mod test {
    use data_encoding::base16;
    use demultiplex::Demultiplex;
    use packet::Packet;
    use packet::PacketConsumer;

    #[test]
    fn pat() {
        let buf = base16::decode(b"474000150000B00D0001C100000001E1E02D507804FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").unwrap();
        let pk = Packet::new(&buf[..]);
        let mut deplex = Demultiplex::new();
        deplex.consume(pk);
    }
}
