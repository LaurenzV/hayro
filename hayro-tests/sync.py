import json
from pathlib import Path
import requests
from rich.console import Console, Group
from rich.live import Live
from rich.progress import BarColumn, Progress, TextColumn
from rich.text import Text

SCRIPT_DIR = Path(__file__).resolve().parent
PDFS_DIR = SCRIPT_DIR / "pdfs"
DOWNLOADS_DIR = SCRIPT_DIR / "downloads"
OUTPUT_FILE = SCRIPT_DIR / "tests" / "render.rs"

MANIFESTS = [
    ("custom", SCRIPT_DIR / "manifest_custom.json", False),
    ("pdfjs", SCRIPT_DIR / "manifest_pdfjs.json", True),
    ("pdfbox", SCRIPT_DIR / "manifest_pdfbox.json", True),
    ("corpus", SCRIPT_DIR / "manifest_corpus.json", True),
]

REMOTE_SOURCES = {
    "custom": ("https://hayro-assets.dev/custom/", "custom"),
    "pdfjs": ("https://hayro-assets.dev/pdfjs/", "pdfjs"),
    "pdfbox": ("https://hayro-assets.dev/pdfbox/", "pdfbox"),
    "corpus": ("https://hayro-assets.dev/corpus/", "corpus"),
}

CONSOLE = Console(log_time=False)


def load_manifest(path: Path, assume_link: bool) -> list[dict]:
    if not path.exists():
        return []
    raw_entries = json.loads(path.read_text())
    entries = []
    for item in raw_entries:
        if isinstance(item, str):
            entries.append({"id": item, "link": assume_link})
        else:
            entry = dict(item)
            entry.setdefault("link", assume_link)
            entries.append(entry)
    return entries


def download_path(kind: str, entry_id: str) -> Path:
    _, subdir = REMOTE_SOURCES[kind]
    target_dir = DOWNLOADS_DIR if subdir is None else DOWNLOADS_DIR / subdir
    return target_dir / f"{entry_id}.pdf"


def download_pdf(entry_id: str, url: str, subdir: str | None) -> tuple[bool, str]:
    target_dir = DOWNLOADS_DIR if subdir is None else DOWNLOADS_DIR / subdir
    target_dir.mkdir(parents=True, exist_ok=True)
    destination = target_dir / f"{entry_id}.pdf"

    if destination.exists():
        return True, "cached"

    try:
        response = requests.get(url, timeout=30)
        response.raise_for_status()
        destination.write_bytes(response.content)
    except requests.RequestException as exc:
        if destination.exists():
            destination.unlink()
        return False, str(exc)

    return True, "downloaded"


def expected_local_file(entry: dict) -> Path | None:
    file_field = entry.get("file")
    if not file_field:
        return None
    return SCRIPT_DIR / file_field


def build_test(entry: dict, kind: str) -> str:
    entry_id = entry["id"]
    first_page = entry.get("first_page")
    last_page = entry.get("last_page")

    if first_page is not None and last_page is not None:
        length = f'Some("{first_page}..={last_page}")'
    elif first_page is not None:
        length = f'Some("{first_page}..")'
    elif last_page is not None:
        length = f'Some("..={last_page}")'
    else:
        length = "None"

    func_stub = entry_id.replace("-", "_").replace(".", "_")
    if kind in ("pdfjs", "pdfbox", "corpus"):
        func_name = f"{kind}_{func_stub}"
    else:
        func_name = func_stub

    if entry.get("link"):
        if kind == "custom":
            file_path = f"downloads/custom/{entry_id}.pdf"
        else:
            file_path = f"downloads/{kind}/{entry_id}.pdf"
    else:
        file_path = entry.get("file", "")
        if kind in ("pdfjs", "pdfbox", "corpus") and file_path:
            relative = file_path.replace("pdfs/", "")
            file_path = f"pdfs/{kind}/{relative}"

    return f'#[test] fn {func_name}() {{ run_render_test("{func_name}", "{file_path}", {length}); }}'


def collect_entries() -> tuple[list[tuple[dict, str, bool]], int, int]:
    plan: list[tuple[dict, str, bool]] = []
    cached_count = 0
    download_total = 0

    for kind, path, assume_link in MANIFESTS:
        entries = load_manifest(path, assume_link)
        for entry in entries:
            if entry.get("ignore"):
                continue
            is_cached = False
            if entry.get("link"):
                download_total += 1
                target_file = download_path(kind, entry["id"])
                is_cached = target_file.exists()
                if is_cached:
                    cached_count += 1
            plan.append((entry, kind, is_cached))

    return plan, download_total, cached_count


def write_tests(rust_functions: list[str]) -> None:
    header = "use crate::run_render_test;\n\n"
    content = header + "\n".join(rust_functions)
    OUTPUT_FILE.write_text(content)


def main() -> None:
    DOWNLOADS_DIR.mkdir(exist_ok=True)
    (PDFS_DIR / "corpus").mkdir(parents=True, exist_ok=True)

    plan, total_downloads, cached_count = collect_entries()
    rust_functions: list[str] = []
    failures: list[tuple[str, str]] = []

    progress: Progress | None = None
    task_id: int | None = None
    live: Live | None = None
    label_text = Text("", style="cyan")

    def show_label(text: str, style: str = "cyan") -> None:
        if live:
            label_text.style = style
            label_text.plain = text
            live.refresh()

    def clear_label() -> None:
        if live:
            label_text.plain = ""
            live.refresh()

    try:
        if total_downloads:
            progress = Progress(
                BarColumn(bar_width=None, style="cyan", complete_style="green", finished_style="green"),
                TextColumn("{task.completed}/{task.total}", justify="right", style="bold"),
                console=CONSOLE,
                expand=True,
            )
            task_id = progress.add_task("", total=total_downloads)
            if cached_count:
                progress.update(task_id, completed=cached_count)
            live = Live(Group(Text("Downloading PDF files...", style="bold"), progress, label_text), console=CONSOLE, refresh_per_second=12)
            live.start()

        for entry, kind, is_cached in plan:
            clear_label()
            include_entry = True
            if entry.get("link"):
                base_url, subdir = REMOTE_SOURCES[kind]
                url = f"{base_url}{entry['id']}.pdf"
                label = f"{kind}:{entry['id']}"
                if is_cached:
                    show_label(label, "green")
                else:
                    show_label(label, "cyan")
                    success, detail = download_pdf(entry["id"], url, subdir)
                    if progress and task_id is not None:
                        progress.advance(task_id)
                    if success:
                        show_label(label, "green")
                    else:
                        show_label(f"{label} failed", "red")
                        failures.append((entry["id"], detail))
                        include_entry = False
                # cached entries were counted up-front
            else:
                expected_path = expected_local_file(entry)
                if not expected_path or not expected_path.exists():
                    failures.append((entry["id"], "missing local file"))
                    show_label(f"{entry['id']} missing local file", "red")
                    include_entry = False

            if include_entry:
                rust_functions.append(build_test(entry, kind))

        clear_label()
    finally:
        if progress:
            progress.stop()
        if live:
            live.stop()

    if rust_functions:
        write_tests(rust_functions)

    if failures:
        CONSOLE.print("Failed downloads:")
        for entry_id, message in failures:
            CONSOLE.print(f"- {entry_id}: {message}")
    else:
        CONSOLE.print("All files are ready.")


if __name__ == "__main__":
    main()
