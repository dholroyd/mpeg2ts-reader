#Changelog

## [Unreleased]
### Changed
 - Many methods previously taking a `Packet` by value now instead take it by reference.  This breaks with the old API,
   but may deliver a small performance improvement for some workloads.