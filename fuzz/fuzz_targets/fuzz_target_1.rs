#![no_main]

use libfuzzer_sys::fuzz_target;
use mpeg2ts_reader::{demultiplex, pes, packet, packet_filter_switch, demux_context, descriptor};

pub struct FuzzElementaryStreamConsumer;
impl pes::ElementaryStreamConsumer<FuzzDemuxContext> for FuzzElementaryStreamConsumer {
    fn start_stream(&mut self, _ctx: &mut FuzzDemuxContext) {}
    fn begin_packet(&mut self, _ctx: &mut FuzzDemuxContext, header: pes::PesHeader) {
        let _ = header.stream_id();
        let _ = header.pes_packet_length();
        match header.contents() {
            pes::PesContents::Parsed(Some(content)) => {
                let _ = content.pes_priority();
                let _ = content.data_alignment_indicator();
                let _ = content.copyright();
                let _ = content.original_or_copy();
                let _ = content.pts_dts();
                let _ = content.escr();
                let _ = content.es_rate();
                let _ = content.dsm_trick_mode();
                let _ = content.additional_copy_info();
                let _ = content.previous_pes_packet_crc();
                let _ = content.pes_extension();
                let _ = content.payload();
            },
            pes::PesContents::Parsed(None) => {},
            pes::PesContents::Payload(_data) => {},
        }
    }
    fn continue_packet(&mut self, _ctx: &mut FuzzDemuxContext, _data: &[u8]) {}
    fn end_packet(&mut self, _ctx: &mut FuzzDemuxContext) {}
    fn continuity_error(&mut self, _ctx: &mut FuzzDemuxContext) {}
}

pub struct FuzzPacketFilter;
impl demultiplex::PacketFilter for FuzzPacketFilter {
    type Ctx = FuzzDemuxContext;

    fn consume(&mut self, _ctx: &mut Self::Ctx, pk: &packet::Packet<'_>) {
        if let Some(af) = pk.adaptation_field() {
            format!("{:?}", af);
        }
    }
}

packet_filter_switch!{
    FuzzFilterSwitch<FuzzDemuxContext> {
        Pat: demultiplex::PatPacketFilter<FuzzDemuxContext>,
        Pmt: demultiplex::PmtPacketFilter<FuzzDemuxContext>,
        Elem: pes::PesPacketFilter<FuzzDemuxContext,FuzzElementaryStreamConsumer>,
        Packet: FuzzPacketFilter,
        Null: demultiplex::NullPacketFilter<FuzzDemuxContext>,
    }
}
demux_context!(FuzzDemuxContext, FuzzFilterSwitch);
impl FuzzDemuxContext {
    fn do_construct(&mut self, req: demultiplex::FilterRequest) -> FuzzFilterSwitch {
        match req {
            demultiplex::FilterRequest::ByPid(packet::Pid::PAT) => FuzzFilterSwitch::Pat(demultiplex::PatPacketFilter::default()),
            demultiplex::FilterRequest::ByPid(_) => FuzzFilterSwitch::Null(demultiplex::NullPacketFilter::default()),
            demultiplex::FilterRequest::ByStream { stream_type, pmt, stream_info, .. } => {
                // we make redundant calls to pmt.descriptors() for each stream, but this is the
                // simplest place to hook this call into the fuzz test right now,
                for desc in pmt.descriptors::<descriptor::CoreDescriptors>() {
                    format!("{:?}", desc);
                }
                for desc in stream_info.descriptors::<descriptor::CoreDescriptors>() {
                    format!("{:?}", desc);
                }
                if stream_type.is_pes() {
                    FuzzFilterSwitch::Elem(pes::PesPacketFilter::new(FuzzElementaryStreamConsumer))
                } else {
                    FuzzFilterSwitch::Packet(FuzzPacketFilter)
                }
            },
            demultiplex::FilterRequest::Pmt{pid, program_number} => FuzzFilterSwitch::Pmt(demultiplex::PmtPacketFilter::new(pid, program_number)),
            demultiplex::FilterRequest::Nit{pid: _} => FuzzFilterSwitch::Null(demultiplex::NullPacketFilter::default()),
        }
    }
}
fuzz_target!(|data: &[u8]| {
    let mut ctx = FuzzDemuxContext::new();
    let mut demux = demultiplex::Demultiplex::new(&mut ctx);
    demux.push(&mut ctx, data);
});
