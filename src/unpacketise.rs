
use packet;

pub struct Unpacketise<C>
    where C: packet::PacketConsumer<()>
{
    consumer: C,
    // TODO: store any remainder from end of last buffer
}

const PACKET_SIZE: usize = 188;

impl <C> Unpacketise<C>
    where C: packet::PacketConsumer<()>
{
    pub fn new(consumer: C) -> Unpacketise<C> {
        Unpacketise { consumer }
    }


    // TODO: 'packet cursor' / iterator or something
    pub fn push(&mut self, buf: &[u8]) {
        for pk in buf.chunks(PACKET_SIZE) {
            // TODO: move test out of loop?
            if pk.len() < PACKET_SIZE {
                println!("need to implement handling of remainder ({} bytes)", pk.len());
                break;
            }
            if packet::Packet::is_sync_byte(pk[0]) {
                self.consumer.consume(packet::Packet::new(pk));
            } else {
                // TODO: attempt to resynchronise
                println!("not ts :( {:#x} {}", pk[0], buf.len());
            }
        }
    }
}


#[cfg(test)]
mod test {
    use unpacketise;
    use packet;
    use std::rc::Rc;
    use std::cell::RefCell;

    struct MockPacketConsumer {
        pub pids: Rc<RefCell<Vec<u16>>>
    }
    impl packet::PacketConsumer<()> for MockPacketConsumer {
        fn consume(&mut self, pk: packet::Packet) -> Option<()> {
            self.pids.borrow_mut().push(pk.pid());
            None
        }
    }

    #[test]
    fn unpacketise() {
        let pids = Rc::new(RefCell::new(vec!()));
        let mock = MockPacketConsumer { pids: pids.clone() };
        let mut buf = [0u8; 188*2];
        buf[0] = 0x47;  // 1st packet sync-byte
        buf[2] = 0x07;  // 1st packet pid
        buf[188] = 0x47;  // 2nd packet sync-byte
        buf[190] = 0x09;  // 2st packet pid
        let mut unpack = unpacketise::Unpacketise::new(mock);
        unpack.push(&buf[..]);
        assert_eq!(*pids.borrow_mut(), vec!(0x07u16, 0x09u16));
    }

    struct NullPacketConsumer {
    }
    impl packet::PacketConsumer<()> for NullPacketConsumer {
        fn consume(&mut self, _: packet::Packet) -> Option<()> {
            None
        }
    }

    #[test]
    fn empty() {
        let null = NullPacketConsumer { };
        let mut unpack = unpacketise::Unpacketise::new(null);
        let buf = [0u8; 0];
        unpack.push(&buf[..]);
    }

    #[test]
    fn byte() {
        let null = NullPacketConsumer { };
        let mut unpack = unpacketise::Unpacketise::new(null);
        let buf = [0x0Au8; 1];
        unpack.push(&buf[..]);
    }
}
