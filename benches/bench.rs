#[macro_use]
extern crate criterion;
#[macro_use]
extern crate mpeg2ts_reader;

use criterion::{Criterion,Benchmark,Throughput};
use std::fs::File;
use std::io::Read;
use mpeg2ts_reader::demultiplex;
use mpeg2ts_reader::pes;
use mpeg2ts_reader::StreamType;

packet_filter_switch!{
    NullFilterSwitch<NullDemuxContext> {
        Pat: demultiplex::PatPacketFilter<NullDemuxContext>,
        Pmt: demultiplex::PmtPacketFilter<NullDemuxContext>,
        Null: demultiplex::NullPacketFilter<NullDemuxContext>,
        NullPes: pes::PesPacketFilter<NullDemuxContext,NullElementaryStreamConsumer>,
    }
}
demux_context!(NullDemuxContext, NullStreamConstructor);

pub struct NullStreamConstructor;
impl demultiplex::StreamConstructor for NullStreamConstructor {
    type F = NullFilterSwitch;

    fn construct(&mut self, req: demultiplex::FilterRequest) -> Self::F {
        match req {
            demultiplex::FilterRequest::ByPid(0) => NullFilterSwitch::Pat(demultiplex::PatPacketFilter::new()),
            demultiplex::FilterRequest::ByPid(_) => NullFilterSwitch::Null(demultiplex::NullPacketFilter::new()),
            demultiplex::FilterRequest::ByStream(StreamType::Private(0x86), pmt_section, stream_info) => NullElementaryStreamConsumer::construct(pmt_section, stream_info),
            demultiplex::FilterRequest::ByStream(_stype, _pmt_section, _stream_info) => NullFilterSwitch::Null(demultiplex::NullPacketFilter::new()),
            demultiplex::FilterRequest::Pmt{pid, program_number} => NullFilterSwitch::Pmt(demultiplex::PmtPacketFilter::new(pid, program_number)),
        }
    }
}

pub struct NullElementaryStreamConsumer { }
impl NullElementaryStreamConsumer {
    fn construct(_pmt_sect: &demultiplex::PmtSection,stream_info: &demultiplex::StreamInfo) -> NullFilterSwitch {
        println!("stream info: {:?}", stream_info);
        let filter = pes::PesPacketFilter::new(NullElementaryStreamConsumer { });
        NullFilterSwitch::NullPes(filter)
    }
}
impl pes::ElementaryStreamConsumer for NullElementaryStreamConsumer {
    fn start_stream(&mut self) { println!("start_steam()"); }
    fn begin_packet(&mut self, _header: pes::PesHeader) { }
    fn continue_packet(&mut self, _data: &[u8]) { }
    fn end_packet(&mut self) { }
    fn continuity_error(&mut self) { }
}

fn mpeg2ts_reader(c: &mut Criterion) {
    let mut f = File::open("big_buck_bunny_1080p_24fps_h264.ts").expect("file not found");
    let l = f.metadata().unwrap().len() as usize;
    let size = l.min(188*200_000);
    let mut buf = vec![0; size];
    f.read(&mut buf[..]).unwrap();
    let mut ctx = NullDemuxContext::new(NullStreamConstructor);
    let mut demux = demultiplex::Demultiplex::new(&mut ctx);
    c.bench("parse", Benchmark::new("parse", move |b| {
        b.iter(|| {
            demux.push(&mut ctx, &buf[..]);
        } );
    }).throughput(Throughput::Bytes(size as u32)));
}


criterion_group!(benches, mpeg2ts_reader);
criterion_main!(benches);
