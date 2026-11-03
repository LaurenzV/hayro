# Changelog

## [Unreleased]

## [0.1.1] - 2026-11-02

This release has an [MSRV] of 1.92.

### Added

- Implement `Hash` for `CidFamily` and `CharacterCollection`. ([#1699][] by [@LaurenzV][])

### Changed

- Stricter parsing of numbers and `beginbfrange` arrays. ([#1712][], [#1714][] by [@LaurenzV][])

### Fixed

- Fix parsing of CMaps with procedures, unusual names or tokens that look like numbers. ([#1715][],
  [#1717][], [#1718][] by [@LaurenzV][])
- Fix a stack overflow on malformed CMaps. ([#1713][] by [@LaurenzV][])

[@LaurenzV]: https://github.com/LaurenzV

[#1699]: https://github.com/LaurenzV/hayro/pull/1699
[#1712]: https://github.com/LaurenzV/hayro/pull/1712
[#1713]: https://github.com/LaurenzV/hayro/pull/1713
[#1714]: https://github.com/LaurenzV/hayro/pull/1714
[#1715]: https://github.com/LaurenzV/hayro/pull/1715
[#1717]: https://github.com/LaurenzV/hayro/pull/1717
[#1718]: https://github.com/LaurenzV/hayro/pull/1718

[MSRV]: ../README.md#minimum-supported-rust-version-msrv

[Unreleased]: https://github.com/LaurenzV/hayro/compare/hayro-cmap-v0.1.1...HEAD
[0.1.1]: https://github.com/LaurenzV/hayro/compare/hayro-cmap-v0.1.0...hayro-cmap-v0.1.1
