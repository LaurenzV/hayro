# Changelog

## [Unreleased]

## [0.4.0] - 2026-11-02

This release has an [MSRV] of 1.92.

### Added

- A strict mode that rejects files violating the specification. ([#1476][], [#1533][] by
  [@LaurenzV][])
- Support for files with several pages. ([#1530][], [#1531][] by [@LaurenzV][])
- `Globals` to decode global segments once and share them between pages. ([#1529][] by
  [@LaurenzV][])
- Support for the extended template with 12 AT pixels. ([#1485][] by [@LaurenzV][])

### Changed

- Breaking change: `Image` was replaced by `File` and `Page`, and all constructors take
  `DecodeSettings`. ([#1476][], [#1529][], [#1530][], [#1531][] by [@LaurenzV][])
- Breaking change: new error variants for strict mode. ([#1476][], [#1533][] by [@LaurenzV][])
- Decode more malformed files instead of failing. ([#1477][], [#1478][], [#1479][], [#1480][],
  [#1482][], [#1483][], [#1490][], [#1493][] by [@LaurenzV][])
- Raise the limits for the number of symbols. ([#1629][], [#1630][], [#1631][] by [@LaurenzV][])
- Performance improvements. ([#1459][], [#1536][], [#1543][] by [@LaurenzV][])

### Removed

- Breaking change: `HuffmanError::PrefixLengthTooLarge`. ([#1492][] by [@LaurenzV][])

### Fixed

- Fix several issues with refinement regions. ([#1440][], [#1444][], [#1448][], [#1449][], [#1450][]
  by [@LaurenzV][])
- Fix several issues with custom Huffman tables. ([#1470][], [#1481][], [#1491][], [#1492][] by
  [@LaurenzV][])
- Fix valid files being rejected. ([#1452][], [#1453][] by [@LaurenzV][])
- Reject regions with the colour extension instead of decoding them incorrectly. ([#1500][] by
  [@LaurenzV][])
- Fix panics and excessive memory use on malformed files. ([#1455][], [#1456][], [#1457][],
  [#1552][] by [@LaurenzV][], [#1671][] by [@bitemyapp][])

[@bitemyapp]: https://github.com/bitemyapp
[@LaurenzV]: https://github.com/LaurenzV

[#1440]: https://github.com/LaurenzV/hayro/pull/1440
[#1444]: https://github.com/LaurenzV/hayro/pull/1444
[#1448]: https://github.com/LaurenzV/hayro/pull/1448
[#1449]: https://github.com/LaurenzV/hayro/pull/1449
[#1450]: https://github.com/LaurenzV/hayro/pull/1450
[#1452]: https://github.com/LaurenzV/hayro/pull/1452
[#1453]: https://github.com/LaurenzV/hayro/pull/1453
[#1455]: https://github.com/LaurenzV/hayro/pull/1455
[#1456]: https://github.com/LaurenzV/hayro/pull/1456
[#1457]: https://github.com/LaurenzV/hayro/pull/1457
[#1459]: https://github.com/LaurenzV/hayro/pull/1459
[#1470]: https://github.com/LaurenzV/hayro/pull/1470
[#1476]: https://github.com/LaurenzV/hayro/pull/1476
[#1477]: https://github.com/LaurenzV/hayro/pull/1477
[#1478]: https://github.com/LaurenzV/hayro/pull/1478
[#1479]: https://github.com/LaurenzV/hayro/pull/1479
[#1480]: https://github.com/LaurenzV/hayro/pull/1480
[#1481]: https://github.com/LaurenzV/hayro/pull/1481
[#1482]: https://github.com/LaurenzV/hayro/pull/1482
[#1483]: https://github.com/LaurenzV/hayro/pull/1483
[#1485]: https://github.com/LaurenzV/hayro/pull/1485
[#1490]: https://github.com/LaurenzV/hayro/pull/1490
[#1491]: https://github.com/LaurenzV/hayro/pull/1491
[#1492]: https://github.com/LaurenzV/hayro/pull/1492
[#1493]: https://github.com/LaurenzV/hayro/pull/1493
[#1500]: https://github.com/LaurenzV/hayro/pull/1500
[#1529]: https://github.com/LaurenzV/hayro/pull/1529
[#1530]: https://github.com/LaurenzV/hayro/pull/1530
[#1531]: https://github.com/LaurenzV/hayro/pull/1531
[#1533]: https://github.com/LaurenzV/hayro/pull/1533
[#1536]: https://github.com/LaurenzV/hayro/pull/1536
[#1543]: https://github.com/LaurenzV/hayro/pull/1543
[#1552]: https://github.com/LaurenzV/hayro/pull/1552
[#1629]: https://github.com/LaurenzV/hayro/pull/1629
[#1630]: https://github.com/LaurenzV/hayro/pull/1630
[#1631]: https://github.com/LaurenzV/hayro/pull/1631
[#1671]: https://github.com/LaurenzV/hayro/pull/1671

[MSRV]: ../README.md#minimum-supported-rust-version-msrv

[Unreleased]: https://github.com/LaurenzV/hayro/compare/hayro-jbig2-v0.4.0...HEAD
[0.4.0]: https://github.com/LaurenzV/hayro/compare/hayro-jbig2-v0.3.1...hayro-jbig2-v0.4.0
