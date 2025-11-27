Test inputs are not committed to git. Instead, they are downloaded on demand from https://hayro-assets.dev under the `jpeg2000` namespace. Each namespace has a manifest file that describes every entry and optional metadata:

- A plain string is treated as the file id and the test is expected to render and match a snapshot.
- A JSON object with an `id` and `render: false` marks a load-only test; the decoder must run without panicking, but no snapshot is checked.

The manifests live next to the crate (currently `manifest_serenity.json` and `manifest_openjpeg.json`). Files are stored locally under `test-inputs/<namespace>/<id>` and ignored by git.

### Synchronizing inputs

Run the helper script whenever you need to populate or refresh the inputs:

```bash
python3 sync.py
```

Use `--force` to redownload files even if a cached copy exists. The script downloads every entry in every manifest and mirrors the remote directory layout under `test-inputs/`.

## Generating baseline snapshots

Snapshots are stored under `snapshots/` and also ignored by git. To seed them, first ensure the decoder is built from a known-good revision, then run the harness once:

```bash
REPLACE=1 cargo test
```

This renders every manifest entry, writes the PNG snapshots, and logs any failures. After the baseline exists, run the suite normally to verify changes:

```bash
cargo test
```

If the decoder output changes intentionally, rerun with `REPLACE=1` to update the affected snapshots and then rerun without `REPLACE` to confirm everything passes.
