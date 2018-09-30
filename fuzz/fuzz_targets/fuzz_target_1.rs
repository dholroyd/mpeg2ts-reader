#![no_main]

#[macro_use]
extern crate libfuzzer_sys;
#[macro_use]
extern crate mpeg2ts_reader;

use mpeg2ts_reader::{demultiplex, pes};

pub struct FuzzElementaryStreamConsumer;
impl pes::ElementaryStreamConsumer for FuzzElementaryStreamConsumer {
    fn start_stream(&mut self) {}
    fn begin_packet(&mut self, header: pes::PesHeader) {}
    fn continue_packet(&mut self, data: &[u8]) {}
    fn end_packet(&mut self) {}
    fn continuity_error(&mut self) {}
}

packet_filter_switch!{
    FuzzFilterSwitch<FuzzDemuxContext> {
        Pat: demultiplex::PatPacketFilter<FuzzDemuxContext>,
        Pmt: demultiplex::PmtPacketFilter<FuzzDemuxContext>,
        Elem: pes::PesPacketFilter<FuzzDemuxContext,FuzzElementaryStreamConsumer>,
        Null: demultiplex::NullPacketFilter<FuzzDemuxContext>,
        Unhandled: demultiplex::UnhandledPid<FuzzDemuxContext>,
    }
}
demux_context!(FuzzDemuxContext, FuzzStreamConstructor);

pub struct FuzzStreamConstructor;
impl demultiplex::StreamConstructor for FuzzStreamConstructor {
    type F = FuzzFilterSwitch;

    fn construct(&mut self, req: demultiplex::FilterRequest) -> Self::F {
        match req {
            demultiplex::FilterRequest::ByPid(0) => FuzzFilterSwitch::Pat(demultiplex::PatPacketFilter::new()),
            demultiplex::FilterRequest::ByPid(_) => FuzzFilterSwitch::Unhandled(demultiplex::UnhandledPid::new()),
            demultiplex::FilterRequest::ByStream(_stype, _pmt_section, _stream_info) => FuzzFilterSwitch::Null(demultiplex::NullPacketFilter::new()),
            demultiplex::FilterRequest::Pmt{pid, program_number} => FuzzFilterSwitch::Pmt(demultiplex::PmtPacketFilter::new(pid, program_number)),
            demultiplex::FilterRequest::Nit{pid} => FuzzFilterSwitch::Null(demultiplex::NullPacketFilter::new()),
        }
    }
}
fuzz_target!(|data: &[u8]| {
    let mut ctx = FuzzDemuxContext::new(FuzzStreamConstructor);
    let mut demux = demultiplex::Demultiplex::new(&mut ctx);
    let res = demux.push(&mut ctx, data);
});
