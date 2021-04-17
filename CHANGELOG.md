# Changelog

<!-- next-header -->

## Unreleased - FutureDate

## 0.15.0 - 2021-04-17

### Changed
 - `Iso639LanguageDescriptor::languages()` now produces `Result<Language>` rather than just `Language`
 - Since we don't support decryption, scrambled TS packets (packets with values of `transport_scrambling_control` other
   than `0`) are now dropped, to prevent the application being passed bogus data
 - `AdaptationFieldExtension::new()`  changed to return `Result`

### Fixed
 - Fixed a panic when parsing truncated PMT data
 - Fixed a panic when parsing a truncated descriptor value
 - Fixed a panic when parsing a truncated language code in `iso_639_language_descriptor`
 - Fixed a panic when reported PES header length does not fit within available space
 - Fixed a panic due to a bug parsing PES header `ESCR_base` field
 - Fixes a panic when `PES_header_data_length` is not long enough to accommodate all the headers actually present
 - Fixed so we accept very short but still syntactically valid PSI sections previously rejected in some cases
 - Fixed a panic on TS packet with `adaptation_field_extension_flag` set, but `adaptation_field_extension_length` is `0`

## 0.14.0 - 2021-04-11
### Changed
 - The `pes::ElementaryStreamConsumer` type and its methods are now parameterised to gain access to the context object
   you provided to the demultiplexer.  Thanks @fkaa.
 - `RegistrationDescriptor::format_identifier()` return type changed from u32 to a value of the `FormatIdentifier` enum
   from the [smptera-format-identifiers-rust](https://crates.io/crates/smptera-format-identifiers-rust) crate.  Also
   `RegistrationDescriptor` fields are no longer public.
 - `Pid::new()` is now a `const` function.  Other crates can now define `Pid` constants.
 - Definitions of constants for 'PAT' and 'stuffing' PIDs have been relocated, now that don't have to be in the same
   module as the `Pid` type.

### Added
 - Implementation of `TryFrom<u16>` for `Pid`

## 0.13.0
### Changed
 - The `descriptor_enum!{}` macro no longer provides a default case (which used to produce `Error`), so callers which don't define
   mappings for all possible descriptor tag values (`0`-`255`) will now get a compiler error.
### Fixed
 - Fixed incorrect value of `Timestamp::MAX`.
### Added
 - Added `Timestamp::TIMEBASE` constant.

## 0.12.00
### Added
 - `AVC_video_descriptor()` parsing.
 - `maximum_bitrate_descriptor()` parsing.
 - `Timestamp::likely_wrapped_since()` utility for detecting wraparound, and supporting `Timestamp::MAX` constant.

## 0.11.0
### Added
 - Made `mpegts_crc::sum_32()` public.
 - Added types to `psi` module for handling 'compact syntax' sections (mirroring existing types for handling 'section
   syntax' sections).

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
