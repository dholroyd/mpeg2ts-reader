extern crate hexdump;
#[macro_use]
extern crate mpeg2ts_reader;

use std::env;
use std::fs::File;
use std::io;
use mpeg2ts_reader::demultiplex;
use mpeg2ts_reader::pes;
use mpeg2ts_reader::StreamType;


packet_filter_switch!{
    NullFilterSwitch<NullDemuxContext> {
        Pat: demultiplex::PatPacketFilter<NullDemuxContext>,
        Pmt: demultiplex::PmtPacketFilter<NullDemuxContext>,
        Null: demultiplex::NullPacketFilter<NullDemuxContext>,
        Unhandled: demultiplex::UnhandledPid<NullDemuxContext>,
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
            demultiplex::FilterRequest::ByStream(StreamType::H264, pmt_section, stream_info) => NullElementaryStreamConsumer::construct(pmt_section, stream_info),
            demultiplex::FilterRequest::ByStream(StreamType::Iso138183Audio, pmt_section, stream_info) => NullElementaryStreamConsumer::construct(pmt_section, stream_info),
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
    let mut ctx = NullDemuxContext::new(NullStreamConstructor);
    let mut demultiplex = demultiplex::Demultiplex::new(&mut ctx);
    while reading {
        match r.read(&mut buf[..])? {
            0 => break,
            // TODO: if not all bytes are consumed, track buf remainder
            n => demultiplex.push(&mut ctx, &buf[0..n]),
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
