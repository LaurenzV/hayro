# Changelog

## [Unreleased]

## [0.9.0] - 2026-11-02

This release has an [MSRV] of 1.92.

### Added

- Support for the `Crypt` filter. ([#1684][] by [@Vantrongs][])
- `Stream::decoded_data` to decode streams that aren't images. ([#1688][] by [@LaurenzV][])

### Changed

- Breaking change: object numbers are now `u32` instead of `i32`. ([#1650][] by [@yhay81][])
- Breaking change: the fields of `Metadata` are now `Option<Box<[u8]>>`. ([#1489][] by
  [@Its-Just-Nans][])
- Breaking change: color operators store their components in a smaller `SmallVec`. ([#1524][] by
  [@LaurenzV][])
- JPEG 2000 images without an indexed color space are now always decoded to 8 bits. ([#1626][] by
  [@LaurenzV][])
- Raise the limits for the number of operands and the nesting depth of objects. ([#1599][] by
  [@yhay81][], [#1556][] by [@ndbroadbent][])
- Performance improvements. ([#1431][], [#1522][], [#1523][], [#1524][], [#1536][], [#1538][],
  [#1626][] by [@LaurenzV][], [#1545][] by [@yhay81][], [#1547][] by [@breferrari][], [#1570][] by
  [@Vantrongs][])

### Removed

- Breaking change: the `logging` feature. ([#1648][] by [@LaurenzV][])

### Fixed

- Fix line breaks in literal strings. ([#1607][] by [@peterudo][])
- Fix reading streams that refer to an external file. ([#1681][] by [@yhay81][])
- Improve the handling of broken and malformed files. ([#1423][] by [@LaurenzV][], [#1555][],
  [#1600][] by [@yhay81][], [#1567][], [#1568][], [#1570][], [#1572][], [#1579][], [#1595][] by
  [@Vantrongs][])
- Fix several panics, stack overflows and excessive allocations on malformed files. ([#1432][],
  [#1510][], [#1688][] by [@LaurenzV][], [#1501][] by [@sean0407][], [#1685][], [#1686][] by
  [@maxence-lefebvre][])

[@breferrari]: https://github.com/breferrari
[@Its-Just-Nans]: https://github.com/Its-Just-Nans
[@LaurenzV]: https://github.com/LaurenzV
[@maxence-lefebvre]: https://github.com/maxence-lefebvre
[@ndbroadbent]: https://github.com/ndbroadbent
[@peterudo]: https://github.com/peterudo
[@sean0407]: https://github.com/sean0407
[@Vantrongs]: https://github.com/Vantrongs
[@yhay81]: https://github.com/yhay81

[#1423]: https://github.com/LaurenzV/hayro/pull/1423
[#1431]: https://github.com/LaurenzV/hayro/pull/1431
[#1432]: https://github.com/LaurenzV/hayro/pull/1432
[#1489]: https://github.com/LaurenzV/hayro/pull/1489
[#1501]: https://github.com/LaurenzV/hayro/pull/1501
[#1510]: https://github.com/LaurenzV/hayro/pull/1510
[#1522]: https://github.com/LaurenzV/hayro/pull/1522
[#1523]: https://github.com/LaurenzV/hayro/pull/1523
[#1524]: https://github.com/LaurenzV/hayro/pull/1524
[#1536]: https://github.com/LaurenzV/hayro/pull/1536
[#1538]: https://github.com/LaurenzV/hayro/pull/1538
[#1545]: https://github.com/LaurenzV/hayro/pull/1545
[#1547]: https://github.com/LaurenzV/hayro/pull/1547
[#1555]: https://github.com/LaurenzV/hayro/pull/1555
[#1556]: https://github.com/LaurenzV/hayro/pull/1556
[#1567]: https://github.com/LaurenzV/hayro/pull/1567
[#1568]: https://github.com/LaurenzV/hayro/pull/1568
[#1570]: https://github.com/LaurenzV/hayro/pull/1570
[#1572]: https://github.com/LaurenzV/hayro/pull/1572
[#1579]: https://github.com/LaurenzV/hayro/pull/1579
[#1595]: https://github.com/LaurenzV/hayro/pull/1595
[#1599]: https://github.com/LaurenzV/hayro/pull/1599
[#1600]: https://github.com/LaurenzV/hayro/pull/1600
[#1607]: https://github.com/LaurenzV/hayro/pull/1607
[#1626]: https://github.com/LaurenzV/hayro/pull/1626
[#1648]: https://github.com/LaurenzV/hayro/pull/1648
[#1650]: https://github.com/LaurenzV/hayro/pull/1650
[#1681]: https://github.com/LaurenzV/hayro/pull/1681
[#1684]: https://github.com/LaurenzV/hayro/pull/1684
[#1685]: https://github.com/LaurenzV/hayro/pull/1685
[#1686]: https://github.com/LaurenzV/hayro/pull/1686
[#1688]: https://github.com/LaurenzV/hayro/pull/1688

[MSRV]: ../README.md#minimum-supported-rust-version-msrv

[Unreleased]: https://github.com/LaurenzV/hayro/compare/hayro-syntax-v0.9.0...HEAD
[0.9.0]: https://github.com/LaurenzV/hayro/compare/hayro-syntax-v0.8.0...hayro-syntax-v0.9.0
