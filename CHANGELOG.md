# Changelog

## 0.10.0
### Added
 - `StreamType::is_pes()` util function to identify those StreamType values that the spec expects to carry PES content.

## 0.9.0
### Fixed
 - Made the methods of `descriptor::iso_639_language::Language` public (they were private by mistake)
 - Drop TS packets that have `transport_error_indicator` flag set, rather than passing known-bad data to the
   application.

### Added
 - Some more descriptor-tag values in `CoreDescriptors` (but not the descriptor definitions themselves yet).

### Changed
 - Removed the `StreamConstructor` trait, and merged its previous responsibilities into `DemuxContext`.  This makes it
   much simpler for client code to gain access to any relevant `DemuxContext` state when the demuxer requests a handler
   for a newly discovered stream within the TS.
 - Added a `StreamId` enum to replace the `u8` previously used as the return value for `PesHeader::stream_id()`
 - Removed single usage of `hex-slice` crate; resulting Debug impl is not quite so nice, but now there's one less
   dependency.
 - Avoid calling `start_stream()` on `ElementaryStreamConsumer` after parsing errors.  It was only intended to be
   called when the substream was first encountered in the multiplex.

## 0.8.0
### Fixed
 - Avoid panics due to 0-length `adaptation_field`, larger-than-expected `program_info_length` and too small
   `section_length`.  All found through fuzz testing.

### Changed
 - All public API members now have at least minimal documentation
 - Removed the `demultiplex::UnhandledPid` type to try and simplify the API slightly.
 - Removed `PesPacketConsumer`.  The types `PesPacketFilter` and `PesPacketConsumer` seem in hindsight to be redundant
   (the first just being a thin wrapper for the second).  `PesPacketFilter` now implements the complete functionality.
 - Removed `PmtSection::program_info_length()` and `StreamInfo::es_info_length()` from public API.
 - Changed `PmtSection::pcr_pid()` to return a `Pid` (got missed when others changed from `u16` to `Pid`) in version
   0.7.0.

## 0.7.0
### Fixed
 - Removed some unused dependencies

### Changed
 - Changed the representation of PID values in the API from plain `u16` to a new `Pid` wrapper,
   so that the API can't represent invalid PID values.
 - `FilterRequest::ByStream` changes from tuple to struct variant, and gains `program_id` of the
   program to which the stream belongs.
 - All uses of `println!()` have been replaced with use of `warn!()` from the `log` crate.

## [0.6.0]
### Fixed
 - PES data is no longer truncated at the end of the TS packet with
   `payload_unit_start_indicator`

### Changed
 - Many methods previously taking a `Packet` by value now instead take it by reference.  This breaks with the old API,
   but may deliver a small performance improvement for some workloads.
 - Many objects that previously offered a no-args `new()` method have had this replaced with a `default()` impl.
 - The `PCR` type has been renamed `ClockRef`, since it's used to represent the values of both
  _Prograem Clock Reference_ and _Elementry Stream Clock Reference_ fields.
 - `descriptor::RegistrationDescriptor` became `descriptor::registration::RegistrationDescriptor`.
   New descriptor implementations will be added each in their own source file.
 - Moved PAT/PMT types from `demultiplex` module into `psi::pat` and `psi::pmt` modules.
 - Refactored some methods returning custom `Iterator` types to instead return `impl Iterator`, so that the
   actual iterator types can be hidden from the public API
 - `pes_packet_length` is now represented as `enum PesLength`, rather than
   directly as a `u16`, so that the special status of the length-value 0 can be
   made explicit (it's now mapped to `PesLength::Unbounded`)

### Added

 - Added `iso_639_language_descriptor` support
