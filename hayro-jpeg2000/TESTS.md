Test inputs are not committed to git. Instead, they are downloaded on demand from https://hayro-assets.dev under the `jpeg2000` namespace. Each namespace has a manifest file that describes every entry and optional metadata:

- A plain string uses the same value for the test id and the file path, and the test is expected to render and match a snapshot.
- A JSON object uses the following fields:
  - `id`: human-readable test id (typically the filename without the extension).
  - `path`: the actual filename to download and decode.
  - `render` (optional, default `true`): set to `false` for crash-only coverage without snapshot checks.
  - `strict` / `resolve_palette_indices` (optional): override the default decode settings for the test.

The manifests live next to the crate (currently `manifest_serenity.json`, `manifest_openjpeg.json`, and `manifest_custom.json`). Files are stored locally under `test-inputs/<namespace>/<path>` and ignored by git.

## Synchronizing inputs

Run the helper script whenever you need to populate or refresh the inputs:

```bash
python3 sync.py
```

Use `--force` to redownload files even if a cached copy exists. The script downloads every entry in every manifest and mirrors the remote directory layout under `test-inputs/`.

## Generating baseline snapshots

Snapshots are stored under `snapshots/` and also ignored by git. To seed them, first ensure the decoder is built from a known-good revision, then run the harness once:

```bash
REPLACE=1 cargo test --release
```

This renders every manifest entry, writes the PNG snapshots, and logs any failures. After the baseline exists, run the suite normally to verify changes:

```bash
cargo test  --release
```

If the decoder output changes intentionally, rerun with `REPLACE=1` to update the affected snapshots and then rerun without `REPLACE` to confirm everything passes.
