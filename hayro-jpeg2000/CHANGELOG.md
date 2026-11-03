# Changelog

## [Unreleased]

## [0.5.0] - 2026-11-02

This release has an [MSRV] of 1.92.

### Added

- Support for progression order changes, regions of interest and PPM markers spanning several
  segments. ([#1398][], [#1399][], [#1401][] by [@LaurenzV][])
- Support for images with more than 4 channels. ([#1408][] by [@LaurenzV][])
- The `image` integration now reports ICC profiles. ([#1516][] by [@LaurenzV][])

### Changed

- The `image` integration no longer converts gray and RGB images with an ICC profile to sRGB.
  ([#1516][] by [@LaurenzV][])
- Performance improvements. ([#1538][], [#1539][], [#1540][], [#1542][], [#1618][], [#1624][],
  [#1627][], [#1628][] by [@LaurenzV][])

### Removed

- Breaking change: the `logging` feature. ([#1648][] by [@LaurenzV][])

### Fixed

- Fix several panics and decoding issues, especially for subsampled components and reduced
  resolutions. ([#1397][], [#1400][], [#1405][], [#1408][], [#1409][], [#1468][] by [@LaurenzV][])
- Improve color space detection for raw codestreams with subsampled chroma. ([#1403][] by
  [@LaurenzV][])
- Improve the handling of truncated and malformed files. ([#1404][], [#1411][], [#1412][] by
  [@LaurenzV][])

[@LaurenzV]: https://github.com/LaurenzV

[#1397]: https://github.com/LaurenzV/hayro/pull/1397
[#1398]: https://github.com/LaurenzV/hayro/pull/1398
[#1399]: https://github.com/LaurenzV/hayro/pull/1399
[#1400]: https://github.com/LaurenzV/hayro/pull/1400
[#1401]: https://github.com/LaurenzV/hayro/pull/1401
[#1403]: https://github.com/LaurenzV/hayro/pull/1403
[#1404]: https://github.com/LaurenzV/hayro/pull/1404
[#1405]: https://github.com/LaurenzV/hayro/pull/1405
[#1408]: https://github.com/LaurenzV/hayro/pull/1408
[#1409]: https://github.com/LaurenzV/hayro/pull/1409
[#1411]: https://github.com/LaurenzV/hayro/pull/1411
[#1412]: https://github.com/LaurenzV/hayro/pull/1412
[#1468]: https://github.com/LaurenzV/hayro/pull/1468
[#1516]: https://github.com/LaurenzV/hayro/pull/1516
[#1538]: https://github.com/LaurenzV/hayro/pull/1538
[#1539]: https://github.com/LaurenzV/hayro/pull/1539
[#1540]: https://github.com/LaurenzV/hayro/pull/1540
[#1542]: https://github.com/LaurenzV/hayro/pull/1542
[#1618]: https://github.com/LaurenzV/hayro/pull/1618
[#1624]: https://github.com/LaurenzV/hayro/pull/1624
[#1627]: https://github.com/LaurenzV/hayro/pull/1627
[#1628]: https://github.com/LaurenzV/hayro/pull/1628
[#1648]: https://github.com/LaurenzV/hayro/pull/1648

[MSRV]: ../README.md#minimum-supported-rust-version-msrv

[Unreleased]: https://github.com/LaurenzV/hayro/compare/hayro-jpeg2000-v0.5.0...HEAD
[0.5.0]: https://github.com/LaurenzV/hayro/compare/hayro-jpeg2000-v0.4.1...hayro-jpeg2000-v0.5.0
