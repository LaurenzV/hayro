# AGENTS.md

## Contribution guidelines
- Bug fixes and improvements are always welcome. For larger changes that involve design decisions, please open an
  issue first.
- hayro aims to be free of bugs, but keeping the code clean and easy to understand matters just as much. Before
  opening a PR, please check:
  - **Spec violations:** if hayro clearly violates the spec and at least one other viewer (ideally two or more)
    handles the case correctly, it's a good candidate for a fix.
  - **Leniency for broken files:** at least two other implementations must handle the file correctly, and the fix
    must come from a **real-world PDF** you encountered. Don't open PRs for leniency fixes found only with
    hand-crafted PDFs.
  - **Edge cases:** avoid fixes that add significant complexity for little real benefit, such as floating-point
    precision issues or places where hayro deliberately approximates (e.g. gradient conversion). Like leniency
    fixes, they should be motivated by real-world PDFs that hayro renders incorrectly.

## Implementation
- hayro implements specifications (ISO 32000-2, JPEG 2000, JBIG2/T.88, CCITT T.4/T.6,
  CFF/Type 1/CMap, the PostScript Language Reference). Every behavior must come from what the
  spec mandates, or be stated as our own choice where the spec says nothing.
- Other viewers are fine for comparing output, not as a justification. They get things wrong too.
- Never copy code from other PDF libraries.

## Code
- Malformed or hostile input must never panic, hang, or use unbounded time or memory.
  Avoid superlinear algorithms on broken files.
- Comments: none by default. Add one only for a non-obvious *why*, in one line. No narrating a fix,
  no spec quotes above checks, no comments explaining what a regression test covers, nothing that
  only makes sense to someone who saw the diff.
- Doc comments only on public API: one short line, describing that item only (not what other code
  does with it). Don't expand existing docs.
- Order items in a file by importance: public items first, then `pub(crate)`, then private ones,
  and by importance within each group.

## Tests
- Always run in release mode: `cargo test --release -p hayro-tests`. Add `-- --offline` to skip
  downloads.
- Commits that change rendering also update `tests/references.lock`: run
  `HAYRO_CI=1 cargo test --release -p hayro-tests`, check that only the expected keys changed in
  `tests/ci/references.lock`, then copy it over. CI runs on macOS, so a page can hash differently
  locally. In that case, take the hash from the CI failure message (`rendered hash …`). Other
  machines only pass once a maintainer has uploaded the image for that hash.

## Commits and PRs
- Subject line: `<crate>: <Imperative summary>`. Use `tests: ` for the test suite and `ci: ` for CI.
  The PR title is the same as the commit subject, with code identifiers in backticks.
- PR descriptions say what the problem was and how it's fixed. Benchmark numbers and new tests are
  welcome. Leave out validation steps and comparisons with other viewers.
