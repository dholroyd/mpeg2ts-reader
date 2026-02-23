use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use mpeg2ts_reader::demultiplex;
use mpeg2ts_reader::demux_context;
use mpeg2ts_reader::packet_filter_switch;
use mpeg2ts_reader::pes;
use mpeg2ts_reader::psi;
use std::fs::File;
use std::io::Read;

pub struct NullTsdtConsumer;
impl<Ctx> demultiplex::TsdtConsumer<Ctx> for NullTsdtConsumer {
    fn tsdt(&mut self, _ctx: &mut Ctx, _header: &psi::TableSyntaxHeader<'_>, _section: &psi::tsdt::TsdtSection<'_>) {}
}

packet_filter_switch! {
    NullFilterSwitch<NullDemuxContext> {
        Pat: demultiplex::PatPacketFilter<NullDemuxContext>,
        Pmt: demultiplex::PmtPacketFilter<NullDemuxContext>,
        Tsdt: demultiplex::TsdtPacketFilter<NullDemuxContext, NullTsdtConsumer>,
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
            demultiplex::FilterRequest::ByPid(psi::tsdt::TSDT_PID) => {
                NullFilterSwitch::Tsdt(demultiplex::TsdtPacketFilter::new(NullTsdtConsumer))
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

#[library_benchmark]
fn reader() {
    let mut f = File::open("testsrc.ts")
        .expect("Test file missing.  To create, run: mkdir -p resources && ffmpeg -f lavfi -i testsrc=duration=20:size=640x360:rate=30,noise=alls=20:allf=t+u -f lavfi -i sine=duration=20:frequency=1:beep_factor=480:sample_rate=48000 -c:v libx264 -b:v 20M -map 0:v -c:a aac -b:a 128k -map 1:a -vf format=yuv420p -f mpegts testsrc.ts");
    let l = f.metadata().unwrap().len() as usize;
    let size = l.min(188 * 200_000);
    let mut buf = vec![0; size];
    f.read(&mut buf[..]).unwrap();

    let mut ctx = NullDemuxContext::new();
    let mut demux = demultiplex::Demultiplex::new(&mut ctx);
    demux.push(&mut ctx, &buf[..]);
}

library_benchmark_group!(
    name = ci;
    benchmarks = reader
);

main!(library_benchmark_groups = ci);
