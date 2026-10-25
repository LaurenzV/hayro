# Hayro test suite

This is the test suite for hayro, which mainly aims to ensure there are no regressions
in rendering when applying changes to the code base (though some tests are for exampled based on fuzzed PDFs and only serve the purpose of ensuring there are no crashes).

## Introduction
A number of the PDFs that are used as part of the tests can be found inside of the `pdfs` folder. However, the vast majority are not included in this repository. They are hosted on a Cloudflare R2 object storage and the test runner downloads the ones it needs into the `downloads` folder the first time they are required, together with the CJK fallback fonts in the `assets` folder.

There are four different categories of tests:
- Custom tests, which include custom-selected PDFs (like for example from the hayro issue tracker).
- pdf.js tests, which have been copied from the pdf.js regression test suite.
- PDFBox tests, which have been copied from the PDFBox issue tracker.
- Corpus tests, which are taken from the [large-scale PDF corpus](https://pdfa.org/new-large-scale-pdf-corpus-now-publicly-available/).

Each test category has its own manifest file, defining the names of the tests as well as some additional metadata. The raster tests are created from these manifests when the test runner starts, so adding a new test only requires adding an entry to the corresponding manifest.

## Running the tests

```bash
cargo test -p hayro-tests --release
```

The test suite uses its own test runner, which runs all tests in parallel and shows the tests that are currently running as well as all failures as they happen. Arguments after `--` select the tests to run and change how they are run:

```bash
# Run all tests whose name contains `pdfjs_` or `load::`.
cargo test -p hayro-tests --release -- pdfjs_ load::
# Run a single test.
cargo test -p hayro-tests --release -- --exact raster::pdfjs_alphatrans
# List the selected tests without running them.
cargo test -p hayro-tests --release -- --list svg::
# Don't download anything, skip tests whose files are missing instead.
cargo test -p hayro-tests --release -- --offline
```

Run `cargo test -p hayro-tests --release -- --help` to see all options.

## Generating the baseline snapshots

The test suite is based on reference image snapshots, which are not checked into the GitHub repository. Therefore, you first need to generate the "baseline" snapshots from the `main` branch of the repository by running the test suite once with `--update`:

```bash
cargo test -p hayro-tests --release -- --update
```

Without `--update`, a test whose reference image is missing fails, and its rendered page is included in `report.html` so that you can check it before accepting it.

Afterwards, if you run the tests again without `--update`, all tests should pass. Now, if you make changes to the code base, you can simply rerun the command to ensure there are no visual differences in any of the PDFs. In case there are, the test runner writes a report to `report.html`, which lists every page that differs and lets you compare it with its reference side by side, as a diff overlay or by flipping between the two. In case the changes are not a regression, you can rerun the failing tests with `--update` to replace their reference snapshots:

```bash
cargo test -p hayro-tests --release -- --update
```

To also keep the rendered output of every test (the PNGs, SVGs and PDFs) in the `store` folder, pass `--store`.

## Test types
There are currently four different categories of tests:
- Load tests: They ensure that a file can be loaded/rendered without crashing. Those tests should ideally be run in debug mode.
- Raster tests: This category makes up the bulk and ensures that PDFs are rasterized correctly.
- SVG tests: Those tests are for testing `hayro-svg` by rendering the resulting SVGs with `resvg`. A manifest entry is turned into an SVG test by adding `"svg": true` to it.

Some tests render slightly differently depending on the platform. For those, `"max_diff_pixels": <n>` in the manifest entry allows up to `n` pixels per page to differ from the reference image.
- Write tests: Those are for the `hayro-write` crate, which is considered internal. You can ignore those.

## Other
The eventual goal is to have test PDFs from all major issue trackers (pdf.js, PDFBox, mupdf/ghostscript and Chromium) in our test
suite. The state for each is as follows:
- PDFBox: I manually went through all issues and included the ones I deemed necessary. 
- pdf.js: A large number of tests have been ported, but not all yet. I also still need to go through the pdf.js issue
  tracker to find other issues that haven't been included in the test suite or are still open.
- mupdf/ghostscript: Not ported yet.
- Chromium: PDFs have been taken from the [PDFium issue tracker](https://issues.chromium.org/issues?q=componentid:1586257%20-title:fuzz).
  The PDFIum repo also has some rendering tests, but since those are rather basic it is not planned to run those in
  hayro.

## Last synced test
The following shows the last synced test for each issue tracker, so that we know where to continue from:
- PDFBox: PDFBOX-6024
- Chromium: last_modified <= 2025-03-14.