# Changelog

## [Unreleased]

## [0.9.0] - 2026-11-02

This release has an [MSRV] of 1.92.

### Added

- Initial support for text hinting. ([#1705][] by [@LaurenzV][])
- Initial support for glyph caching. ([#1706][] by [@LaurenzV][])
- `RenderSettings::aliasing_threshold` to control anti-aliasing. ([#1611][] by [@yhay81][])

### Changed

- Breaking change: updated the re-exported `hayro-interpret` to 0.9. See its changelog for details.
- Breaking change: `RenderSettings` has new fields. ([#1611][] by [@yhay81][], [#1705][], [#1706][]
  by [@LaurenzV][])
- Breaking change: `RenderCache` no longer implements `Clone`. ([#1704][] by [@LaurenzV][])
- Performance improvements. ([#1526][], [#1527][], [#1618][], [#1709][] by [@LaurenzV][], [#1609][]
  by [@alexwine36][], [#1612][] by [@yhay81][])

### Removed

- Breaking change: the `simd` feature. SIMD is now always used. ([#1638][] by [@LaurenzV][])
- Breaking change: the `logging` feature. ([#1648][] by [@LaurenzV][])

### Fixed

- Fix color bleeding when downscaling images with transparency. ([#1507][] by [@LaurenzV][])
- Fix several issues with tiling patterns. ([#1668][] by [@LaurenzV][], [#1335][] by [@zfsguy][])
- Fix the opacity of patterns. ([#1571][] by [@Vantrongs][])

[@alexwine36]: https://github.com/alexwine36
[@LaurenzV]: https://github.com/LaurenzV
[@Vantrongs]: https://github.com/Vantrongs
[@yhay81]: https://github.com/yhay81
[@zfsguy]: https://github.com/zfsguy

[#1335]: https://github.com/LaurenzV/hayro/pull/1335
[#1507]: https://github.com/LaurenzV/hayro/pull/1507
[#1526]: https://github.com/LaurenzV/hayro/pull/1526
[#1527]: https://github.com/LaurenzV/hayro/pull/1527
[#1571]: https://github.com/LaurenzV/hayro/pull/1571
[#1609]: https://github.com/LaurenzV/hayro/pull/1609
[#1611]: https://github.com/LaurenzV/hayro/pull/1611
[#1612]: https://github.com/LaurenzV/hayro/pull/1612
[#1618]: https://github.com/LaurenzV/hayro/pull/1618
[#1638]: https://github.com/LaurenzV/hayro/pull/1638
[#1648]: https://github.com/LaurenzV/hayro/pull/1648
[#1668]: https://github.com/LaurenzV/hayro/pull/1668
[#1704]: https://github.com/LaurenzV/hayro/pull/1704
[#1705]: https://github.com/LaurenzV/hayro/pull/1705
[#1706]: https://github.com/LaurenzV/hayro/pull/1706
[#1709]: https://github.com/LaurenzV/hayro/pull/1709

[MSRV]: ../README.md#minimum-supported-rust-version-msrv

[Unreleased]: https://github.com/LaurenzV/hayro/compare/hayro-v0.9.0...HEAD
[0.9.0]: https://github.com/LaurenzV/hayro/compare/hayro-v0.8.0...hayro-v0.9.0
