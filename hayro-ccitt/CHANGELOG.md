# Changelog

## [Unreleased]

## [0.5.0] - 2026-11-02

This release has an [MSRV] of 1.92.

### Added

- A strict mode that rejects damaged data instead of recovering from it. ([#1472][] by
  [@LaurenzV][])

### Changed

- Breaking change: `Decoder` now receives whole rows at once. ([#1536][] by [@LaurenzV][])
- Breaking change: `DecodeSettings` has a new `strict` field. ([#1472][] by [@LaurenzV][])
- Breaking change: `EncodingMode::Group3_2D` no longer has a `k` field. ([#1434][] by [@LaurenzV][])
- Invalid codes are no longer retried with the code table of the other color. ([#1471][] by
  [@LaurenzV][])
- Performance improvements. ([#1426][], [#1427][], [#1428][], [#1429][], [#1536][] by [@LaurenzV][])

### Removed

- Breaking change: `DecodeSettings::invert_black`. ([#1536][] by [@LaurenzV][])
- Breaking change: `DecodeError::LineLengthMismatch`. ([#1434][] by [@LaurenzV][])

### Fixed

- Improve the support for EOLs, byte-aligned rows and the end of Group 3 data. ([#1415][],
  [#1416][], [#1417][], [#1425][] by [@LaurenzV][])
- Recover from damaged lines in Group 3 data. ([#1420][] by [@LaurenzV][])
- Fix an infinite loop with 0 columns, and decode until the end of the data with 0 rows. ([#1419][]
  by [@LaurenzV][])

[@LaurenzV]: https://github.com/LaurenzV

[#1415]: https://github.com/LaurenzV/hayro/pull/1415
[#1416]: https://github.com/LaurenzV/hayro/pull/1416
[#1417]: https://github.com/LaurenzV/hayro/pull/1417
[#1419]: https://github.com/LaurenzV/hayro/pull/1419
[#1420]: https://github.com/LaurenzV/hayro/pull/1420
[#1425]: https://github.com/LaurenzV/hayro/pull/1425
[#1426]: https://github.com/LaurenzV/hayro/pull/1426
[#1427]: https://github.com/LaurenzV/hayro/pull/1427
[#1428]: https://github.com/LaurenzV/hayro/pull/1428
[#1429]: https://github.com/LaurenzV/hayro/pull/1429
[#1434]: https://github.com/LaurenzV/hayro/pull/1434
[#1471]: https://github.com/LaurenzV/hayro/pull/1471
[#1472]: https://github.com/LaurenzV/hayro/pull/1472
[#1536]: https://github.com/LaurenzV/hayro/pull/1536

[MSRV]: ../README.md#minimum-supported-rust-version-msrv

[Unreleased]: https://github.com/LaurenzV/hayro/compare/hayro-ccitt-v0.5.0...HEAD
[0.5.0]: https://github.com/LaurenzV/hayro/compare/hayro-ccitt-v0.4.0...hayro-ccitt-v0.5.0
