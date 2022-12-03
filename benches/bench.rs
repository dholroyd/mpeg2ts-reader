#[macro_use]
extern crate criterion;
#[macro_use]
extern crate mpeg2ts_reader;

use criterion::{Criterion, Throughput};
use mpeg2ts_reader::demultiplex;
use mpeg2ts_reader::pes;
use mpeg2ts_reader::psi;
use std::fs::File;
use std::io::Read;

packet_filter_switch! {
    NullFilterSwitch<NullDemuxContext> {
        Pat: demultiplex::PatPacketFilter<NullDemuxContext>,
        Pmt: demultiplex::PmtPacketFilter<NullDemuxContext>,
        Null: demultiplex::NullPacketFilter<NullDemuxContext>,
        NullPes: pes::PesPacketFilter<NullDemuxContext,NullElementaryStreamConsumer>,
    }
}
demux_context!(NullDemuxContext, NullFilterSwitch);
impl NullDemuxContext {
    fn do_construct(&mut self, req: demultiplex::FilterRequest<'_, '_>) -> NullFilterSwitch {
        match req {
            demultiplex::FilterRequest::ByPid(psi::pat::PAT_PID) => {
                NullFilterSwitch::Pat(demultiplex::PatPacketFilter::default())
            }
            demultiplex::FilterRequest::ByPid(_) => {
                NullFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
            demultiplex::FilterRequest::ByStream {
                pmt, stream_info, ..
            } => NullElementaryStreamConsumer::construct(pmt, stream_info),
            demultiplex::FilterRequest::Pmt {
                pid,
                program_number,
            } => NullFilterSwitch::Pmt(demultiplex::PmtPacketFilter::new(pid, program_number)),
            demultiplex::FilterRequest::Nit { .. } => {
                NullFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
        }
    }
}

pub struct NullElementaryStreamConsumer {}
impl NullElementaryStreamConsumer {
    fn construct(
        _pmt_sect: &psi::pmt::PmtSection,
        stream_info: &psi::pmt::StreamInfo,
    ) -> NullFilterSwitch {
        println!("stream info: {:?}", stream_info);
        let filter = pes::PesPacketFilter::new(NullElementaryStreamConsumer {});
        NullFilterSwitch::NullPes(filter)
    }
}
impl<Ctx> pes::ElementaryStreamConsumer<Ctx> for NullElementaryStreamConsumer {
    fn start_stream(&mut self, _ctx: &mut Ctx) {}
    fn begin_packet(&mut self, _ctx: &mut Ctx, header: pes::PesHeader) {
        if let pes::PesContents::Parsed(Some(content)) = header.contents() {
            match content.pts_dts() {
                Ok(pes::PtsDts::PtsOnly(Ok(ts))) => {
                    criterion::black_box(ts);
                }
                Ok(pes::PtsDts::Both { pts: Ok(ts), .. }) => {
                    criterion::black_box(ts);
                }
                _ => (),
            };
        }
    }
    fn continue_packet(&mut self, _ctx: &mut Ctx, _data: &[u8]) {}
    fn end_packet(&mut self, _ctx: &mut Ctx) {}
    fn continuity_error(&mut self, _ctx: &mut Ctx) {}
}

fn mpeg2ts_reader(c: &mut Criterion) {
    let buf = if std::env::var("GITHUB_ACTIONS").is_ok() || std::env::var("GIT_AUTHOR_DATE").is_ok()
    {
        // HACK
        // we don't run benchmarking on github, or in pre-commit hook, but we do test that the
        // benchmark code executes without errors, so we need an input buffer of some kind for the
        // code to consume
        vec![0; 188]
    } else {
        let mut f = File::open("resources/testsrc.ts")
            .expect("Test file missing.  To create, run: mkdir -p resources && ffmpeg -f lavfi -i testsrc=duration=20:size=640x360:rate=30,noise=alls=20:allf=t+u -f lavfi -i sine=duration=20:frequency=1:beep_factor=480:sample_rate=48000 -c:v libx264 -b:v 20M -map 0:v -c:a aac -b:a 128k -map 1:a -vf format=yuv420p -f mpegts resources/testsrc.ts");
        let l = f.metadata().unwrap().len() as usize;
        let size = l.min(188 * 200_000);
        let mut buf = vec![0; size];
        f.read(&mut buf[..]).unwrap();
        buf
    };
    let mut ctx = NullDemuxContext::new();
    let mut demux = demultiplex::Demultiplex::new(&mut ctx);
    let mut group = c.benchmark_group("parse");
    group.throughput(Throughput::Bytes(buf.len() as _));
    group.bench_function("parse", move |b| {
        b.iter(|| demux.push(&mut ctx, &buf[..]));
    });
    group.finish();
}

criterion_group!(benches, mpeg2ts_reader);
criterion_main!(benches);
