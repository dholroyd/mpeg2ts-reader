mpeg2ts-reader
==============

Rust reader for MPEG2 Transport Stream data

[![crates.io version](https://img.shields.io/crates/v/mpeg2ts-reader.svg)](https://crates.io/crates/mpeg2ts-reader)
[![Documentation](https://docs.rs/mpeg2ts-reader/badge.svg)](https://docs.rs/mpeg2ts-reader)
[![Coverage Status](https://coveralls.io/repos/github/dholroyd/mpeg2ts-reader/badge.svg)](https://coveralls.io/github/dholroyd/mpeg2ts-reader)
![Unstable API](https://img.shields.io/badge/API_stability-unstable-yellow.svg)

Zero-copy access to payload data within an MPEG Transport Stream.

This crate,
 - implements a low-level state machine that recognises the structural elements of Transport Stream syntax
 - provides traits that you should implement to define your application-specific processing of the contained data.

# Example

Dump H264 payload data as hex.

```rust
#[macro_use]
extern crate mpeg2ts_reader;
extern crate hex_slice;

use hex_slice::AsHex;
use mpeg2ts_reader::demultiplex;
use mpeg2ts_reader::packet;
use mpeg2ts_reader::pes;
use mpeg2ts_reader::psi;
use mpeg2ts_reader::StreamType;
use std::cmp;
use std::env;
use std::fs::File;
use std::io::Read;

// This macro invocation creates an enum called DumpFilterSwitch, encapsulating all possible ways
// that this application may handle transport stream packets.  Each enum variant is just a wrapper
// around an implementation of the PacketFilter trait
packet_filter_switch! {
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
demux_context!(DumpDemuxContext, DumpFilterSwitch);

// When the de-multiplexing process needs to create a PacketFilter instance to handle a particular
// kind of data discovered within the Transport Stream being processed, it will send a
// FilterRequest to our application-specific implementation of the do_construct() method
impl DumpDemuxContext {
    fn do_construct(&mut self, req: demultiplex::FilterRequest<'_, '_>) -> DumpFilterSwitch {
        match req {
            // The 'Program Association Table' is is always on PID 0.  We just use the standard
            // handling here, but an application could insert its own logic if required,
            demultiplex::FilterRequest::ByPid(packet::Pid::PAT) => {
                DumpFilterSwitch::Pat(demultiplex::PatPacketFilter::default())
            }
            // 'Stuffing' data on PID 0x1fff may be used to pad-out parts of the transport stream
            // so that it has constant overall bitrate.  This causes it to be ignored if present.
            demultiplex::FilterRequest::ByPid(packet::Pid::STUFFING) => {
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
            // Some Transport Streams will contain data on 'well known' PIDs, which are not
            // announced in PAT / PMT metadata.  This application does not process any of these
            // well known PIDs, so we register NullPacketFiltet such that they will be ignored
            demultiplex::FilterRequest::ByPid(_) => {
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
            // This match-arm installs our application-specific handling for each H264 stream
            // discovered within the transport stream,
            demultiplex::FilterRequest::ByStream {
                stream_type: StreamType::H264,
                pmt,
                stream_info,
                ..
            } => PtsDumpElementaryStreamConsumer::construct(pmt, stream_info),
            // We need to have a match-arm to specify how to handle any other StreamType values
            // that might be present; we answer with NullPacketFilter so that anything other than
            // H264 (handled above) is ignored,
            demultiplex::FilterRequest::ByStream { .. } => {
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
            // The 'Program Map Table' defines the sub-streams for a particular program within the
            // Transport Stream (it is common for Transport Streams to contain only one program).
            // We just use the standard handling here, but an application could insert its own
            // logic if required,
            demultiplex::FilterRequest::Pmt {
                pid,
                program_number,
            } => DumpFilterSwitch::Pmt(demultiplex::PmtPacketFilter::new(pid, program_number)),
            // Ignore 'Network Information Table', if present,
            demultiplex::FilterRequest::Nit { .. } => {
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
        }
    }
}

// Implement the ElementaryStreamConsumer to just dump and PTS/DTS timestamps to stdout
pub struct PtsDumpElementaryStreamConsumer {
    pid: packet::Pid,
    len: Option<usize>,
}
impl PtsDumpElementaryStreamConsumer {
    fn construct(
        _pmt_sect: &psi::pmt::PmtSection,
        stream_info: &psi::pmt::StreamInfo,
    ) -> DumpFilterSwitch {
        let filter = pes::PesPacketFilter::new(PtsDumpElementaryStreamConsumer {
            pid: stream_info.elementary_pid(),
            len: None,
        });
        DumpFilterSwitch::Pes(filter)
    }
}
impl pes::ElementaryStreamConsumer for PtsDumpElementaryStreamConsumer {
    fn start_stream(&mut self) {}
    fn begin_packet(&mut self, header: pes::PesHeader) {
        match header.contents() {
            pes::PesContents::Parsed(Some(parsed)) => {
                match parsed.pts_dts() {
                    Ok(pes::PtsDts::PtsOnly(Ok(pts))) => {
                        print!("{:?}: pts {:#08x}                ", self.pid, pts.value())
                    }
                    Ok(pes::PtsDts::Both {
                        pts: Ok(pts),
                        dts: Ok(dts),
                    }) => print!(
                        "{:?}: pts {:#08x} dts {:#08x} ",
                        self.pid,
                        pts.value(),
                        dts.value()
                    ),
                    _ => (),
                }
                let payload = parsed.payload();
                self.len = Some(payload.len());
                println!(
                    "{:02x}",
                    payload[..cmp::min(payload.len(), 16)].plain_hex(false)
                )
            }
            pes::PesContents::Parsed(None) => (),
            pes::PesContents::Payload(payload) => {
                self.len = Some(payload.len());
                println!(
                    "{:?}:                               {:02x}",
                    self.pid,
                    payload[..cmp::min(payload.len(), 16)].plain_hex(false)
                )
            }
        }
    }
    fn continue_packet(&mut self, data: &[u8]) {
        println!(
            "{:?}:                     continues {:02x}",
            self.pid,
            data[..cmp::min(data.len(), 16)].plain_hex(false)
        );
        self.len = self.len.map(|l| l + data.len());
    }
    fn end_packet(&mut self) {
        println!("{:?}: end of packet length={:?}", self.pid, self.len);
    }
    fn continuity_error(&mut self) {}
}

fn main() {
    // open input file named on command line,
    let name = env::args().nth(1).unwrap();
    let mut f = File::open(&name).expect(&format!("file not found: {}", &name));

    // create the context object that stores the state of the transport stream demultiplexing
    // process
    let mut ctx = DumpDemuxContext::new();

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
```

# Performance shoot-out

Comparing this crate to a couple of others which you might use to read a Transport Stream --
[mpeg2ts](https://crates.io/crates/mpeg2ts) and [ffmpg-sys](https://crates.io/crates/ffmpeg-sys):

![Performance](https://github.com/dholroyd/mpeg2ts-reader/raw/master/shootout/report.svg?sanitize=true)

The benchmarks producing the above chart data are in the [`shootout`](shootout) folder.  (If the benchmarks are giving
an unfair representation of relative performance, that's a mistake -- please raise a bug!)

The conditions of the test are,
 * the data is already in memory (no network/disk access)
 * test dataset is larger than CPU cache
 * processing is happening on a single core (no multiprocessing of the stream).

# Supported Transport Stream features

Not all Transport Stream features are supported yet.  Here's a summary of what's available,
and what's yet to come:

- Framing
  - [x] _ISO/IEC 13818-1_ 188-byte packets
  - [ ] m2ts 192-byte packets (would be nice if an external crate could support, at least)
  - [ ] recovery after loss of synchronisation
- Transport Stream packet
  - [x] Fixed headers
  - [x] Adaptation field
  - [ ] TS-level scrambling (values of `transport_scrambling_control` other than `0`) not supported
- Program Specific Information tables
  - [x] Section syntax
  - [ ] 'Multi-section' tables
  - [x] PAT - Program Association Table
  - [x] PMT - Program Mapping Table
  - [ ] TSDT - Transport Stream Description Table
- Packetised Elementary Stream syntax
  - [x] PES_packet_data
  - [x] PTS/DTS
  - [x] ESCR
  - [x] ES_rate
  - [x] DSM_trick_mode
  - [x] additional_copy_info
  - [x] PES_CRC
  - [ ] PES_extension
- Descriptors
  - [ ] video_stream_descriptor
  - [ ] audio_stream_descriptor
  - [ ] hierarchy_descriptor
  - [x] registration_descriptor
  - [ ] data_stream_alignment_descriptor
  - [ ] target_background_grid_descriptor
  - [ ] video_window_descriptor
  - [ ] ca_descriptor
  - [x] iso_639_language_descriptor
  - [ ] system_clock_descriptor
  - [ ] multiplex_buffer_utilization_descriptor
  - [ ] copyright_descriptor
  - [x] maximum_bitrate_descriptor
  - [ ] private_data_indicator_descriptor
  - [ ] smoothing_buffer_descriptor
  - [ ] std_descriptor
  - [ ] ibp_descriptor
  - [ ] mpeg4_video_descriptor
  - [ ] mpeg4_audio_descriptor
  - [ ] iod_descriptor
  - [ ] sl_descriptor
  - [ ] fmc_descriptor
  - [ ] external_es_id_descriptor
  - [ ] muxcode_descriptor
  - [ ] fmxbuffersize_descriptor
  - [ ] multiplexbuffer_descriptor
  - [x] AVC_video_descriptor
