#[macro_use]
extern crate mpeg2ts_reader;

use mpeg2ts_reader::demultiplex;
use mpeg2ts_reader::demultiplex::DemuxContext;
use mpeg2ts_reader::demultiplex::PacketFilter;
use mpeg2ts_reader::packet::Packet;
use mpeg2ts_reader::psi;
use std::env;
use std::fs::File;
use std::io::Read;
use std::marker;

packet_filter_switch! {
    PcrDumpFilterSwitch<PcrDumpDemuxContext> {
        Pat: demultiplex::PatPacketFilter<PcrDumpDemuxContext>,
        Pmt: demultiplex::PmtPacketFilter<PcrDumpDemuxContext>,
        Null: demultiplex::NullPacketFilter<PcrDumpDemuxContext>,
        Pcr: PcrPacketFilter<PcrDumpDemuxContext>,
    }
}
demux_context!(PcrDumpDemuxContext, PcrDumpFilterSwitch);
impl PcrDumpDemuxContext {
    fn do_construct(&mut self, req: demultiplex::FilterRequest<'_, '_>) -> PcrDumpFilterSwitch {
        match req {
            demultiplex::FilterRequest::ByPid(psi::pat::PAT_PID) => {
                PcrDumpFilterSwitch::Pat(demultiplex::PatPacketFilter::default())
            }
            demultiplex::FilterRequest::Pmt {
                pid,
                program_number,
            } => PcrDumpFilterSwitch::Pmt(demultiplex::PmtPacketFilter::new(pid, program_number)),

            demultiplex::FilterRequest::ByStream {
                pmt, stream_info, ..
            } => {
                if stream_info.elementary_pid() == pmt.pcr_pid() {
                    PcrDumpFilterSwitch::Pcr(PcrPacketFilter::construct(pmt, stream_info))
                } else {
                    PcrDumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
                }
            }

            demultiplex::FilterRequest::ByPid(_) => {
                PcrDumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
            demultiplex::FilterRequest::Nit { .. } => {
                PcrDumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
        }
    }
}

pub struct PcrPacketFilter<Ctx: DemuxContext> {
    phantom: marker::PhantomData<Ctx>,
}
impl<Ctx: DemuxContext> PcrPacketFilter<Ctx> {
    pub fn construct(
        _pmt: &psi::pmt::PmtSection,
        _stream_info: &psi::pmt::StreamInfo,
    ) -> PcrPacketFilter<Ctx> {
        Self::new()
    }
    pub fn new() -> PcrPacketFilter<Ctx> {
        PcrPacketFilter {
            phantom: marker::PhantomData,
        }
    }
}
impl<Ctx: DemuxContext> PacketFilter for PcrPacketFilter<Ctx> {
    type Ctx = Ctx;
    fn consume(&mut self, _ctx: &mut Self::Ctx, pk: &Packet) {
        if let Some(adaptation_field) = pk.adaptation_field() {
            if let Ok(pcr) = adaptation_field.pcr() {
                println!("{:?} pcr={}", pk.pid(), u64::from(pcr));
            }
        }
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // open input file named on command line,
    let name = env::args().nth(1).unwrap();
    let mut f = File::open(&name).unwrap_or_else(|_| panic!("file not found: {}", &name));

    // create the context object that stores the state of the transport stream demultiplexing
    // process
    let mut ctx = PcrDumpDemuxContext::new();

    // create the demultiplexer, which will use the ctx to create a filter for pid 0 (PAT)
    let mut demux = demultiplex::Demultiplex::new(&mut ctx);

    // consume the input file,
    let mut buf = [0u8; 188 * 1024];
    loop {
        match f.read(&mut buf[..]).expect("read failed") {
            0 => break,
            n => demux.push(&mut ctx, &buf[0..n]),
        }
    }
}
