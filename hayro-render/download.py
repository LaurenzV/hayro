from pathlib import Path
import requests

SCRIPT_DIR = Path(__file__).resolve().parent
ASSETS_DIR = SCRIPT_DIR / 'assets'
DOWNLOADS_DIR = SCRIPT_DIR / 'downloads'

def ensure_downloads_dir():
    DOWNLOADS_DIR.mkdir(exist_ok=True)

def download_pdf(link_path: Path):
    stem = link_path.stem
    dest_path = DOWNLOADS_DIR / f"{stem}.pdf"

    if dest_path.exists():
        print(f"✔ Skipping {stem} (already downloaded)")
        return

    url = link_path.read_text().strip()
    print(f"Downloading {stem}...")

    try:
        head_response = requests.get(url, stream=True, timeout=10)
        head_response.raise_for_status()

        response = requests.get(url, stream=True, timeout=10)
        with open(dest_path, 'wb') as f:
            for chunk in response.iter_content(chunk_size=8192):
                f.write(chunk)

        print("✔ Downloaded")
    except requests.RequestException as e:
        print(f"✘ Failed to download {stem}: {e}")

def main():
    ensure_downloads_dir()
    for link_file in ASSETS_DIR.glob('*.link'):
        download_pdf(link_file)

if __name__ == "__main__":
    main()
