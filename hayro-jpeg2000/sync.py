#!/usr/bin/env python3
import argparse
import json
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

SCRIPT_DIR = Path(__file__).resolve().parent
TEST_INPUTS_DIR = SCRIPT_DIR / "test-inputs"
REMOTE_BASE = "https://hayro-assets.dev/jpeg2000"
MANIFESTS = [
    ("serenity", SCRIPT_DIR / "manifest_serenity.json"),
]


def load_manifest(path: Path) -> list[dict]:
    if not path.exists():
        return []
    raw_entries = json.loads(path.read_text())
    entries: list[dict] = []
    for item in raw_entries:
        if isinstance(item, str):
            entries.append({"id": item, "render": True})
        else:
            entry = dict(item)
            entry.setdefault("render", True)
            entries.append(entry)
    return entries


def download_file(namespace: str, entry_id: str, *, force: bool) -> tuple[bool, str]:
    target_dir = TEST_INPUTS_DIR / namespace
    target_dir.mkdir(parents=True, exist_ok=True)
    destination = target_dir / entry_id
    was_cached = destination.exists()

    if was_cached and not force:
        return True, "cached"

    url = f"{REMOTE_BASE}/{namespace}/{entry_id}"
    request = Request(url, headers={"User-Agent": "hayro-jpeg2000-sync/1.0"})
    try:
        with urlopen(request, timeout=60) as response:
            data = response.read()
    except (HTTPError, URLError) as exc:
        return False, str(exc)

    temp_path = destination.with_suffix(destination.suffix + ".tmp")
    temp_path.write_bytes(data)
    temp_path.replace(destination)

    if was_cached:
        return True, "updated"
    return True, "downloaded"


def main() -> None:
    parser = argparse.ArgumentParser(description="Download jpeg2000 test inputs")
    parser.add_argument("--force", action="store_true", help="redownload files even if cached")
    args = parser.parse_args()

    TEST_INPUTS_DIR.mkdir(exist_ok=True)

    failures: list[tuple[str, str]] = []
    total = 0
    for namespace, manifest_path in MANIFESTS:
        entries = load_manifest(manifest_path)
        for entry in entries:
            total += 1
            entry_id = entry["id"]
            label = f"{namespace}/{entry_id}"
            success, status = download_file(namespace, entry_id, force=args.force)
            print(f"[{status}] {label}")
            if not success:
                failures.append((label, status))

    if failures:
        print("\nFailed downloads:")
        for label, message in failures:
            print(f"- {label}: {message}")
    else:
        if total:
            print("\nAll test inputs are ready.")
        else:
            print("No manifest entries were found.")


if __name__ == "__main__":
    main()
