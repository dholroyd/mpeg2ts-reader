#![no_main]
#[macro_use] extern crate libfuzzer_sys;
extern crate mpeg2ts_reader;

use mpeg2ts_reader::demultiplex;
use std::collections::HashMap;


fuzz_target!(|data: &[u8]| {
    let table: HashMap<mpeg2ts_reader::StreamType, fn(&demultiplex::PmtSection,&demultiplex::StreamInfo)->Box<std::cell::RefCell<demultiplex::PacketFilter>>>
        = HashMap::new();
    let stream_constructor = demultiplex::StreamConstructor::new(demultiplex::NullPacketFilter::construct, table);
    let mut demux = demultiplex::Demultiplex::new(stream_constructor);
    let res = demux.push(data);
});
