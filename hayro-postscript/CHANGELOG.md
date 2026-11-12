# Changelog

## [Unreleased]

## [0.2.0] - 2026-11-02

This release has an [MSRV] of 1.92.

### Added

- Support for procedures. ([#1715][] by [@LaurenzV][])
- Support for immediately evaluated names. ([#1719][] by [@LaurenzV][])
- `Scanner::offset` and `Scanner::set_offset`. ([#1716][], [#1722][] by [@LaurenzV][])

### Changed

- Breaking change: `Object` was renamed to `Token`. ([#1720][] by [@LaurenzV][])
- Breaking change: numbers are now `i64` and `f64`. ([#1712][] by [@LaurenzV][])
- Breaking change: `[`, `]`, `<<` and `>>` are now returned as names, and arrays are no longer
  parsed. ([#1713][], [#1714][] by [@LaurenzV][])
- Breaking change: escape sequences in names are no longer decoded, and `Name::decode` was replaced
  by `Name::as_bytes`. ([#1718][] by [@LaurenzV][])
- Tokens that look like numbers but aren't are now returned as names. ([#1717][] by [@LaurenzV][])

### Removed

- Breaking change: `Number::as_i32`, `Number::as_f32` and `Error::UnsupportedType`. ([#1712][],
  [#1715][] by [@LaurenzV][])

### Fixed

- Fix a stack overflow on malformed input. ([#1713][] by [@LaurenzV][])

[@LaurenzV]: https://github.com/LaurenzV

[#1712]: https://github.com/LaurenzV/hayro/pull/1712
[#1713]: https://github.com/LaurenzV/hayro/pull/1713
[#1714]: https://github.com/LaurenzV/hayro/pull/1714
[#1715]: https://github.com/LaurenzV/hayro/pull/1715
[#1716]: https://github.com/LaurenzV/hayro/pull/1716
[#1717]: https://github.com/LaurenzV/hayro/pull/1717
[#1718]: https://github.com/LaurenzV/hayro/pull/1718
[#1719]: https://github.com/LaurenzV/hayro/pull/1719
[#1720]: https://github.com/LaurenzV/hayro/pull/1720
[#1722]: https://github.com/LaurenzV/hayro/pull/1722

[MSRV]: ../README.md#minimum-supported-rust-version-msrv

[Unreleased]: https://github.com/LaurenzV/hayro/compare/hayro-postscript-v0.2.0...HEAD
[0.2.0]: https://github.com/LaurenzV/hayro/compare/hayro-postscript-v0.1.0...hayro-postscript-v0.2.0
