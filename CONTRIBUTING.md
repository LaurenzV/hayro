# Contributing to hayro

Thanks for your interest in contributing to `hayro`!

## Local checks

CI runs the following checks on pull requests (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml)):

```bash
# Formatting, lints, documentation, and feature combinations
cargo fmt --check --all
RUSTFLAGS="-D warnings" cargo clippy --workspace --exclude hayro-demo --exclude hayro-bench --tests --examples
RUSTFLAGS="-D warnings" cargo doc --workspace --exclude hayro-demo --exclude hayro-bench --no-deps
RUSTFLAGS="-D warnings" cargo hack check --each-feature --workspace --exclude hayro-demo --exclude hayro-bench

# Minimum Supported Rust Version (MSRV: 1.92)
cargo +1.92 check --workspace

# no_std compatibility (requires `rustup target add thumbv6m-none-eabi`)
cargo check -p hayro-ccitt -p hayro-jbig2 -p hayro-jpeg2000 -p hayro-syntax --no-default-features --target thumbv6m-none-eabi
```

## Running tests

```bash
cargo test --release --workspace --exclude hayro-demo --exclude hayro-bench --exclude hayro-fuzz
```

See [`tests/README.md`](tests/README.md) for details on running subsets of the visual regression
suite (for example `cargo test -p hayro-tests --release -- pdfjs_` or passing `--offline` to skip
uncached downloads). Reference images and `tests/references.lock` are updated automatically by CI
when a pull request is merged into `main`, so there is nothing to update locally.

## Pull requests

- **Scope**: Keep pull requests focused on a single logical change. Since crate APIs are still
  evolving, consider opening an issue first before starting large API or cross-crate changes.
- **Fork setup**: Please open pull requests from a **personal fork** (with *"Allow edits from
  maintainers"* enabled) rather than a GitHub Organization fork, as GitHub blocks maintainers from
  pushing rebase or polish commits to organization-owned forks.
- **Rebasing**: Avoid force-pushing rebases just to keep up with `main` or resolve routine merge
  conflicts unless asked—rebasing resets CI status, and conflicts are normally resolved during
  review.
