# Hayro test suite

This is the test suite for hayro, which mainly aims to ensure there are no regressions
in rendering when applying changes to the code base (though some tests are for exampled based on fuzzed PDFs and only serve the purpose of ensuring there are no crashes).

## Introduction
A number of the PDFs that are used as part of the tests can be found inside of the `pdfs` folder. However, the vast majority are not included in this repository and need to be downloaded separately via the `sync.py` script. The PDFs are hosted on a Cloudflare R2 object storage and will simply be downloaded one after another when running the script.

There are four different categories of tests:
- Custom tests, which include custom-selected PDFs (like for example from the hayro issue tracker).
- pdf.js tests, which have been copied from the pdf.js regression test suite.
- PDFBox tests, which have been copied from the PDFBox issue tracker.
- Corpus tests, which are taken from the [large-scale PDF corpus](https://pdfa.org/new-large-scale-pdf-corpus-now-publicly-available/).

Each test category has its own manifest file, defining the names of the tests as well as some additional metadata. As mentioned, all corpus, PDFBox and pdf.js files need to be downloaded first, while only some of the custom category are not checked into the repository.

## Synchronizing the corpus

1. Install the Python dependency required by the `sync.py` script:
   ```bash
   python3 -m pip install --user rich
   ```
2. Download the corpora and regenerate the manifests and Rust tests:
   ```bash
   python3 sync.py
   ```

Doing so will create a new `downloads` folder containing all PDF files. In case you have added new tests to a manifest file, it will also regenerate the corresponding Rust tests.

## Generating the baseline snapshots

The test suite is based on reference image snapshots, which are not checked into the GitHub repository. Therefore, you first need to generate the "baseline" snapshots from the `main` branch of the repository by running the test suite once:

```bash
cargo test --release
```

This initial run renders every configured page and stores the results under `snapshots/`. Afterwards, if you run the same command again, all tests should pass. Now, if you make changes to the code base, you can simply rerun the command to ensure there are no visual differences in any of the PDFs. In case there are, you can observe the difference in the diff image that is created in the `diffs` folder. In case the changes are not a regression, you can rerun the tests as below to replace all not-matching reference snapshots:

```bash
REPLACE=1 cargo test --release
```

Once the snapshots have been refreshed, run the tests again (without `REPLACE`) to confirm that the suite now passes cleanly.
