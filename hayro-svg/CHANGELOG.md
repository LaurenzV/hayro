# Changelog

## [Unreleased]

## [0.9.0] - 2026-11-02

This release has an [MSRV] of 1.92.

### Changed

- Breaking change: updated the re-exported `hayro-interpret` to 0.9. See its changelog for details.
- Breaking change: `RenderCache` no longer implements `Clone`. ([#1704][] by [@LaurenzV][])

### Removed

- Breaking change: the `logging` feature. ([#1648][] by [@LaurenzV][])

### Fixed

- Fix the opacity of patterns. ([#1571][] by [@Vantrongs][])

[@LaurenzV]: https://github.com/LaurenzV
[@Vantrongs]: https://github.com/Vantrongs

[#1571]: https://github.com/LaurenzV/hayro/pull/1571
[#1648]: https://github.com/LaurenzV/hayro/pull/1648
[#1704]: https://github.com/LaurenzV/hayro/pull/1704

[MSRV]: ../README.md#minimum-supported-rust-version-msrv

[Unreleased]: https://github.com/LaurenzV/hayro/compare/hayro-svg-v0.9.0...HEAD
[0.9.0]: https://github.com/LaurenzV/hayro/compare/hayro-svg-v0.8.0...hayro-svg-v0.9.0
