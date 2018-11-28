#Changelog

## Unreleased
### Changed
 - Changed the representation of PID values in the API from plain `u16` to a new `Pid` wrapper,
   so that the API can't represent invalid PID values.
 - `FilterRequest::ByStream` changes from tuple to struct variant, and gains `program_id` of the
   program to which the stream belongs.

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
