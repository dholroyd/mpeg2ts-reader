extern crate hexdump;
extern crate mpeg2ts_reader;
extern crate amphora;

use std::env;
use std::fs::File;
use std::io;
use std::collections::HashMap;
use mpeg2ts_reader::unpacketise;
use mpeg2ts_reader::demultiplex;
use mpeg2ts_reader::pes;
use mpeg2ts_reader::StreamType;


struct NullElementaryStreamConsumer { }
impl NullElementaryStreamConsumer {
    fn construct(_descriptors: &[Box<amphora::descriptor::Descriptor>]) -> Box<std::cell::RefCell<demultiplex::PacketFilter>> {
        let consumer = pes::PesPacketConsumer::new(NullElementaryStreamConsumer { });
        Box::new(std::cell::RefCell::new(consumer))
    }
}
impl pes::ElementaryStreamConsumer for NullElementaryStreamConsumer {
    fn start_stream(&mut self) { println!("start_steam()"); }
    fn begin_packet(&mut self, header: pes::PesHeader) {
        if let pes::PesContents::Parsed(Some(parsed)) = header.contents() {
            match parsed.pts_dts() {
                pes::PtsDts::PtsOnly(Ok(pts)) => println!("pts {}", pts.value()),
                pes::PtsDts::Both{pts:Ok(pts), dts:Ok(dts)} => println!("pts {}, dts {}", pts.value(), dts.value()),
                _ => (),
            }
        }
    }
    fn continue_packet(&mut self, _data: &[u8]) { }
    fn end_packet(&mut self) { }
    fn continuity_error(&mut self) { }
}

fn run<R>(mut r: R) -> io::Result<()>
    where R: io::Read, R: Sized
{
    let mut buf = [0u8; 188*1024];
    let reading = true;
    let mut table: HashMap<StreamType, fn(&[Box<amphora::descriptor::Descriptor>])->Box<std::cell::RefCell<demultiplex::PacketFilter>>>
        = HashMap::new();
    table.insert(StreamType::H264, NullElementaryStreamConsumer::construct);
    let stream_constructor = demultiplex::StreamConstructor::new(table);
    let demultiplex = demultiplex::Demultiplex::new(stream_constructor);
    let mut parser = unpacketise::Unpacketise::new(demultiplex);
    while reading {
        match r.read(&mut buf[..])? {
            0 => break,
            // TODO: if not all bytes are consumed, track buf remainder
            n => parser.push(&buf[0..n]),
        }
    }
    Ok(())
}

fn main() {
    let mut args = env::args();
    args.next();
    let name = args.next().unwrap();
    let f = File::open(&name).expect(&format!("file not found: {}", &name));
    run(f).expect(&format!("error reading {}", &name));
}
