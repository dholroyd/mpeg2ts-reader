extern crate hexdump;
extern crate mpeg2ts_reader;

use std::env;
use std::fs::File;
use std::io;
use mpeg2ts_reader::unpacketise;
use mpeg2ts_reader::demultiplex;


fn run<R>(mut r: R) -> io::Result<()>
    where R: io::Read, R: Sized
{
    let mut buf = [0u8; 188*1024];
    let reading = true;
    let demultiplex = demultiplex::Demultiplex::new();
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
