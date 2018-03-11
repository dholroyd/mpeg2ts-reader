mpeg2ts-reader
==============

Rust reader for MPEG2 Transport Stream data

[![Build Status](https://travis-ci.org/dholroyd/mpeg2ts-reader.svg?branch=master)](https://travis-ci.org/dholroyd/mpeg2ts-reader)
[![crates.io version](https://img.shields.io/crates/v/mpeg2ts-reader.svg)](https://crates.io/crates/mpeg2ts-reader)

# Supported Transport Stream features

Not all Transport Stream features are supported yet.  Here's a summary of what's available,
and what's yet to come:

- Framing
  - [x] _ISO/IEC 13818-1_ 188-byte packets
  - [ ] m2ts 192-byte packets (would be nice if an external crate could support, at least)
  - [ ] recovery after loss of synchronisation
- Transport Stream packet
  - [x] Fixed headers
  - [ ] Adaptation field (Adaptation field size is accounted for in finding packet payload, adaptation field details are not yet exposed)
- Program Specific Information tables
  - [x] Section syntax
  - [ ] 'Multi-section' tables
  - [x] PAT - Program Association Table
  - [x] PMT - Program Mapping Table
  - [ ] TSDT - Transport Stream Description Table
- Packetised Elementary Stream syntax
  - [x] PES_packet_data
  - [x] PTS/DTS
  - [ ] ESCR
  - [ ] ES_rate
  - [ ] DSM_trick_mode
  - [ ] additional_copy_info
  - [ ] PES_CRC
  - [ ] PES_extension
- Descriptors
  - [ ] video_stream_descriptor
  - [ ] audio_stream_descriptor
  - [ ] hierarchy_descriptor
  - [ ] registration_descriptor
  - [ ] data_stream_alignment_descriptor
  - [ ] target_background_grid_descriptor
  - [ ] video_window_descriptor
  - [ ] ca_descriptor
  - [ ] iso_639_language_descriptor
  - [ ] system_clock_descriptor
  - [ ] multiplex_buffer_utilization_descriptor
  - [ ] copyright_descriptor
  - [ ] maximum_bitrate_descriptor
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
