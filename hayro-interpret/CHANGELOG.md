# Changelog

## [Unreleased]

## [0.9.0] - 2026-11-02

This release has an [MSRV] of 1.92.

### Added

- Re-export `kurbo`. ([#1475][] by [@Its-Just-Nans][])
- Expose more information to devices, such as character codes, writing modes, colors and the stream
  of stencil images. ([#1512][], [#1513][] by [@LaurenzV][], [#1603][], [#1604][], [#1605][] by
  [@yhay81][], [#1678][] by [@philkunz][])
- Support for hinted glyph outlines. ([#1705][] by [@LaurenzV][])

### Changed

- Breaking change: fonts were moved into the new `hayro-font` crate and are re-exported under
  `font`. ([#1696][], [#1699][] by [@LaurenzV][])
- Breaking change: `Device::begin_marked_content` now receives the property list. ([#1513][],
  [#1625][] by [@LaurenzV][])
- Breaking change: `InterpreterSettings::render_annotations` was replaced by
  `InterpreterSettings::render_content`. ([#1537][] by [@ipsi][])
- Breaking change: `Function::eval` and `ShadingFunction::eval` now take slices. ([#1653][] by
  [@LaurenzV][])
- Breaking change: devices now need to apply `TilingPattern::opacity`. ([#1571][] by [@Vantrongs][])
- Text with rendering mode 7 is now passed to devices as invisible text. ([#1677][] by
  [@philkunz][])
- Emit a warning when a font can't be loaded. ([#1498][] by [@erkurtharun][])
- Performance improvements. ([#1521][], [#1645][], [#1653][], [#1654][], [#1664][], [#1699][],
  [#1708][], [#1710][], [#1711][] by [@LaurenzV][])

### Removed

- Breaking change: `SoftMask::id`. ([#1553][] by [@LaurenzV][])
- Breaking change: the `logging` feature. ([#1648][] by [@LaurenzV][])

### Fixed

- Improve the Unicode mapping of glyph names. ([#1511][], [#1512][], [#1514][], [#1515][] by
  [@LaurenzV][])
- Fix several issues with soft masks, patterns, shadings, clipping paths and annotations.
  ([#1502][], [#1503][], [#1553][], [#1561][], [#1645][] by [@LaurenzV][], [#1563][], [#1564][],
  [#1565][], [#1571][], [#1580][], [#1581][], [#1583][], [#1592][] by [@Vantrongs][], [#1548][] by
  [@zfsguy][], [#1394][] by [@sensa-code][], [#1351][] by [@KoffeinFlummi][])
- Fix several issues with functions. ([#1654][], [#1655][], [#1656][] by [@LaurenzV][])
- Fix some embedded fonts not rendering. ([#1699][], [#1702][] by [@LaurenzV][])
- Fix panics and excessive memory use on malformed files. ([#1521][], [#1562][], [#1688][] by
  [@LaurenzV][])

[@erkurtharun]: https://github.com/erkurtharun
[@ipsi]: https://github.com/ipsi
[@Its-Just-Nans]: https://github.com/Its-Just-Nans
[@KoffeinFlummi]: https://github.com/KoffeinFlummi
[@LaurenzV]: https://github.com/LaurenzV
[@philkunz]: https://github.com/philkunz
[@sensa-code]: https://github.com/sensa-code
[@Vantrongs]: https://github.com/Vantrongs
[@yhay81]: https://github.com/yhay81
[@zfsguy]: https://github.com/zfsguy

[#1351]: https://github.com/LaurenzV/hayro/pull/1351
[#1394]: https://github.com/LaurenzV/hayro/pull/1394
[#1475]: https://github.com/LaurenzV/hayro/pull/1475
[#1498]: https://github.com/LaurenzV/hayro/pull/1498
[#1502]: https://github.com/LaurenzV/hayro/pull/1502
[#1503]: https://github.com/LaurenzV/hayro/pull/1503
[#1511]: https://github.com/LaurenzV/hayro/pull/1511
[#1512]: https://github.com/LaurenzV/hayro/pull/1512
[#1513]: https://github.com/LaurenzV/hayro/pull/1513
[#1514]: https://github.com/LaurenzV/hayro/pull/1514
[#1515]: https://github.com/LaurenzV/hayro/pull/1515
[#1521]: https://github.com/LaurenzV/hayro/pull/1521
[#1537]: https://github.com/LaurenzV/hayro/pull/1537
[#1548]: https://github.com/LaurenzV/hayro/pull/1548
[#1553]: https://github.com/LaurenzV/hayro/pull/1553
[#1561]: https://github.com/LaurenzV/hayro/pull/1561
[#1562]: https://github.com/LaurenzV/hayro/pull/1562
[#1563]: https://github.com/LaurenzV/hayro/pull/1563
[#1564]: https://github.com/LaurenzV/hayro/pull/1564
[#1565]: https://github.com/LaurenzV/hayro/pull/1565
[#1571]: https://github.com/LaurenzV/hayro/pull/1571
[#1580]: https://github.com/LaurenzV/hayro/pull/1580
[#1581]: https://github.com/LaurenzV/hayro/pull/1581
[#1583]: https://github.com/LaurenzV/hayro/pull/1583
[#1592]: https://github.com/LaurenzV/hayro/pull/1592
[#1603]: https://github.com/LaurenzV/hayro/pull/1603
[#1604]: https://github.com/LaurenzV/hayro/pull/1604
[#1605]: https://github.com/LaurenzV/hayro/pull/1605
[#1625]: https://github.com/LaurenzV/hayro/pull/1625
[#1645]: https://github.com/LaurenzV/hayro/pull/1645
[#1648]: https://github.com/LaurenzV/hayro/pull/1648
[#1653]: https://github.com/LaurenzV/hayro/pull/1653
[#1654]: https://github.com/LaurenzV/hayro/pull/1654
[#1655]: https://github.com/LaurenzV/hayro/pull/1655
[#1656]: https://github.com/LaurenzV/hayro/pull/1656
[#1664]: https://github.com/LaurenzV/hayro/pull/1664
[#1677]: https://github.com/LaurenzV/hayro/pull/1677
[#1678]: https://github.com/LaurenzV/hayro/pull/1678
[#1688]: https://github.com/LaurenzV/hayro/pull/1688
[#1696]: https://github.com/LaurenzV/hayro/pull/1696
[#1699]: https://github.com/LaurenzV/hayro/pull/1699
[#1702]: https://github.com/LaurenzV/hayro/pull/1702
[#1705]: https://github.com/LaurenzV/hayro/pull/1705
[#1708]: https://github.com/LaurenzV/hayro/pull/1708
[#1710]: https://github.com/LaurenzV/hayro/pull/1710
[#1711]: https://github.com/LaurenzV/hayro/pull/1711

[MSRV]: ../README.md#minimum-supported-rust-version-msrv

[Unreleased]: https://github.com/LaurenzV/hayro/compare/hayro-interpret-v0.9.0...HEAD
[0.9.0]: https://github.com/LaurenzV/hayro/compare/hayro-interpret-v0.8.0...hayro-interpret-v0.9.0
