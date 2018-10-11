#Changelog

## [Unreleased]
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

### Added

 - Added `iso_639_language_descriptor` support
