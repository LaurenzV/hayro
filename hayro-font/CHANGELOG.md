# Changelog

## [Unreleased]

## [0.5.0] - 2026-11-02

This release has an [MSRV] of 1.92.

`hayro-font` now contains the PDF font handling that used to be part of `hayro-interpret`. It
replaces the previous, unrelated `hayro-font` crate (last released as 0.4.0).

### Added

- Initial release. ([#1696][], [#1699][] by [@LaurenzV][])
- Support for hinted glyph outlines. ([#1705][] by [@LaurenzV][])

[@LaurenzV]: https://github.com/LaurenzV

[#1696]: https://github.com/LaurenzV/hayro/pull/1696
[#1699]: https://github.com/LaurenzV/hayro/pull/1699
[#1705]: https://github.com/LaurenzV/hayro/pull/1705

[MSRV]: ../README.md#minimum-supported-rust-version-msrv

[Unreleased]: https://github.com/LaurenzV/hayro/compare/hayro-font-v0.5.0...HEAD
[0.5.0]: https://github.com/LaurenzV/hayro/tree/hayro-font-v0.5.0
