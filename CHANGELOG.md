#Changelog

## [Unreleased]
### Changed
 - Many methods previously taking a `Packet` by value now instead take it by reference.  This breaks with the old API,
   but may deliver a small performance improvement for some workloads.
 - Many objects that previously offered a no-args `new()` method have had this replaced with a `default()` impl.