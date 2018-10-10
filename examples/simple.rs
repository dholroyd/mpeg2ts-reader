#[macro_use]
extern crate mpeg2ts_reader;
extern crate hex_slice;

use std::env;
use std::fs::File;
use std::io::Read;
use mpeg2ts_reader::demultiplex;
use mpeg2ts_reader::pes;
use mpeg2ts_reader::StreamType;
use hex_slice::AsHex;
use std::cmp;
use mpeg2ts_reader::psi;

// This macro invocation creates an enum called DumpFilterSwitch, encapsulating all possible ways
// that this application may handle transport stream packets.  Each enum variant is just a wrapper
// around an implementation of the PacketFilter trait
packet_filter_switch!{
    DumpFilterSwitch<DumpDemuxContext> {
        // the DumpFilterSwitch::Pes variant will perform the logic actually specific to this
        // application,
        Pes: pes::PesPacketFilter<DumpDemuxContext,PtsDumpElementaryStreamConsumer>,

        // these definitions are boilerplate required by the framework,
        Pat: demultiplex::PatPacketFilter<DumpDemuxContext>,
        Pmt: demultiplex::PmtPacketFilter<DumpDemuxContext>,

        // this variant will be used when we want to ignore data in the transport stream that this
        // application does not care about
        Null: demultiplex::NullPacketFilter<DumpDemuxContext>,
    }
}

// This macro invocation creates a type called DumpDemuxContext, which is our application-specific
// implementation of the DemuxContext trait.
demux_context!(DumpDemuxContext, DumpStreamConstructor);

// When the de-multiplexing process needs to create a PacketFilter instance to handle a particular
// kind of data discovered within the Transport Stream being processed, it will send a
// FilterRequest to our application-specific implementation of the StreamConstructor trait
pub struct DumpStreamConstructor;
impl demultiplex::StreamConstructor for DumpStreamConstructor {
    type F = DumpFilterSwitch;

    fn construct(&mut self, req: demultiplex::FilterRequest) -> Self::F {
        match req {
            // The 'Program Association Table' is is always on PID 0.  We just use the standard
            // handling here, but an application could insert its own logic if required,
            demultiplex::FilterRequest::ByPid(0) =>
                DumpFilterSwitch::Pat(demultiplex::PatPacketFilter::default()),
            // Some Transport Streams will contain data on 'well known' PIDs, which are not
            // announced in PAT / PMT metadata.  This application does not process any of these
            // well known PIDs, so we register NullPacketFiltet such that they will be ignored
            demultiplex::FilterRequest::ByPid(_) =>
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default()),
            // This match-arm installs our application-specific handling for each H264 stream
            // discovered within the transport stream,
            demultiplex::FilterRequest::ByStream(StreamType::H264, pmt_section, stream_info) =>
                PtsDumpElementaryStreamConsumer::construct(pmt_section, stream_info),
            // We need to have a match-arm to specify how to handle any other StreamType values
            // that might be present; we answer with NullPacketFilter so that anything other than
            // H264 (handled above) is ignored,
            demultiplex::FilterRequest::ByStream(_stype, _pmt_section, _stream_info) =>
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default()),
            // The 'Program Map Table' defines the sub-streams for a particular program within the
            // Transport Stream (it is common for Transport Streams to contain only one program).
            // We just use the standard handling here, but an application could insert its own
            // logic if required,
            demultiplex::FilterRequest::Pmt{pid, program_number} =>
                DumpFilterSwitch::Pmt(demultiplex::PmtPacketFilter::new(pid, program_number)),
            // Ignore 'Network Information Table', if present,
            demultiplex::FilterRequest::Nit{..} =>
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default()),
        }
    }
}

// Implement the ElementaryStreamConsumer to just dump and PTS/DTS timestamps to stdout
pub struct PtsDumpElementaryStreamConsumer {
    pid: u16,
    len: Option<usize>,
}
impl PtsDumpElementaryStreamConsumer {
    fn construct(_pmt_sect: &psi::pmt::PmtSection, stream_info: &psi::pmt::StreamInfo)
        -> DumpFilterSwitch
    {
        let filter = pes::PesPacketFilter::new(
            PtsDumpElementaryStreamConsumer {
                pid: stream_info.elementary_pid(),
                len: None
            }
        );
        DumpFilterSwitch::Pes(filter)
    }
}
impl pes::ElementaryStreamConsumer for PtsDumpElementaryStreamConsumer {
    fn start_stream(&mut self) { }
    fn begin_packet(&mut self, header: pes::PesHeader) {
        match header.contents() {
            pes::PesContents::Parsed(Some(parsed)) => {
                match parsed.pts_dts() {
                    Ok(pes::PtsDts::PtsOnly(Ok(pts))) => {
                        print!("PID {}: pts {:#08x}                ",
                               self.pid,
                               pts.value())
                    },
                    Ok(pes::PtsDts::Both{pts:Ok(pts), dts:Ok(dts)}) => {
                        print!("PID {}: pts {:#08x} dts {:#08x} ",
                               self.pid,
                               pts.value(),
                               dts.value())
                    },
                    _ => (),
                }
                let payload = parsed.payload();
                self.len = Some(payload.len());
                println!("{:02x}", payload[..cmp::min(payload.len(),16)].plain_hex(false))
            },
            pes::PesContents::Parsed(None) => (),
            pes::PesContents::Payload(payload) => {
                self.len = Some(payload.len());
                println!("PID {}:                               {:02x}",
                         self.pid,
                         payload[..cmp::min(payload.len(),16)].plain_hex(false))
            },
        }
    }
    fn continue_packet(&mut self, data: &[u8]) {
        println!("PID {}:                     continues {:02x}",
                 self.pid,
                 data[..cmp::min(data.len(),16)].plain_hex(false));
        self.len = self.len.map(|l| l+data.len() );
    }
    fn end_packet(&mut self) {
        println!("PID {}: end of packet length={:?}",
                 self.pid,
                 self.len);
    }
    fn continuity_error(&mut self) { }
}

fn main() {
    // open input file named on command line,
    let name = env::args().nth(1).unwrap();
    let mut f = File::open(&name).expect(&format!("file not found: {}", &name));

    // create the context object that stores the state of the transport stream demultiplexing
    // process
    let mut ctx = DumpDemuxContext::new(DumpStreamConstructor);

    // create the demultiplexer, which will use the ctx to create a filter for pid 0 (PAT)
    let mut demux = demultiplex::Demultiplex::new(&mut ctx);

    // consume the input file,
    let mut buf = [0u8; 188*1024];
    loop {
        match f.read(&mut buf[..]).expect("read failed") {
            0 => break ,
            n => demux.push(&mut ctx, &buf[0..n]),
        }
    }
}