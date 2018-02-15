#[macro_use]
extern crate bencher;
extern crate mpeg2ts_reader;
use std::fs::File;
use std::io::Read;
use std::cell;
use std::collections::HashMap;
use bencher::Bencher;
use mpeg2ts_reader::unpacketise;
use mpeg2ts_reader::demultiplex;
use mpeg2ts_reader::pes;
use mpeg2ts_reader::StreamType;

struct NullElementaryStreamConsumer { }
impl NullElementaryStreamConsumer {
    fn construct(stream_info: &demultiplex::StreamInfo) -> Box<std::cell::RefCell<demultiplex::PacketFilter>> {
        println!("stream info: {:?}", stream_info);
        let consumer = pes::PesPacketConsumer::new(NullElementaryStreamConsumer { });
        Box::new(std::cell::RefCell::new(consumer))
    }
}
impl pes::ElementaryStreamConsumer for NullElementaryStreamConsumer {
    fn start_stream(&mut self) { println!("start_steam()"); }
    fn begin_packet(&mut self, header: pes::PesHeader) { }
    fn continue_packet(&mut self, _data: &[u8]) { }
    fn end_packet(&mut self) { }
    fn continuity_error(&mut self) { }
}

fn create_demux() -> demultiplex::Demultiplex {
    let mut table: HashMap<StreamType, fn(&demultiplex::StreamInfo)->Box<cell::RefCell<demultiplex::PacketFilter>>>
    = HashMap::new();

    table.insert(StreamType::Private(0x86), NullElementaryStreamConsumer::construct);
    let ctor = demultiplex::StreamConstructor::new(NullElementaryStreamConsumer::construct, table);
    demultiplex::Demultiplex::new(ctor)
}

fn mpeg2ts_reader(bench: &mut Bencher) {
    let mut f = File::open("big_buck_bunny_1080p_24fps_h264.ts").expect("file not found");
    let l = f.metadata().unwrap().len() as usize;
    let mut buf = vec![0; l];
    f.read(&mut buf[..]).unwrap();
    let mut demux = create_demux();
    let mut parser = unpacketise::Unpacketise::new(demux);
    bench.iter(move || {
        parser.push(&buf[..]);
    });
}

benchmark_group!(benches, mpeg2ts_reader);
benchmark_main!(benches);