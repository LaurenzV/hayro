import os
import json
import requests
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed

class TestGenerator:
    def __init__(self):
        self.script_dir = Path(__file__).resolve().parent
        self.custom_manifest_path = self.script_dir / 'manifest_custom.json'
        self.pdfjs_manifest_path = self.script_dir / 'manifest_pdfjs.json'
        self.pdfbox_manifest_path = self.script_dir / 'manifest_pdfbox.json'
        self.corpus_manifest_path = self.script_dir / 'manifest_corpus.json'
        self.pdfs_dir = self.script_dir / 'pdfs'
        self.downloads_dir = self.script_dir / 'downloads'
        self.corpus_dir = self.pdfs_dir / 'corpus'
        self.output_file = self.script_dir / 'tests' / 'render.rs'
        
    def ensure_downloads_dir(self):
        """Create downloads directory if it doesn't exist."""
        self.downloads_dir.mkdir(exist_ok=True)

    def ensure_corpus_dir(self):
        """Create corpus directory if it doesn't exist."""
        self.corpus_dir.mkdir(exist_ok=True)
        
    def download_pdf(self, entry_id: str, url: str, subdir: str | None = None) -> bool:
        """Download a PDF to the appropriate downloads subdirectory."""
        if subdir:
            dest_dir = self.downloads_dir / subdir
        else:
            dest_dir = self.downloads_dir
        dest_dir.mkdir(exist_ok=True)
        dest_path = dest_dir / f"{entry_id}.pdf"

        if dest_path.exists():
            print(f"✔ Skipping {entry_id} (already downloaded)")
            return True

        print(f"📥 Downloading {entry_id} from {url[:70]}...")

        try:
            response = requests.get(url, stream=True, timeout=30)
            response.raise_for_status()

            with open(dest_path, 'wb') as f:
                for chunk in response.iter_content(chunk_size=8192):
                    f.write(chunk)

            print("✔ Downloaded")
            return True
        except requests.RequestException as e:
            print(f"✘ Failed to download {entry_id}: {e}")
            if dest_path.exists():
                dest_path.unlink()
            return False
            
    def load_manifests(self) -> list:
        """Load and parse both manifest files, combining them."""
        all_entries = []
        
        # Load custom manifest
        if self.custom_manifest_path.exists():
            with open(self.custom_manifest_path, 'r') as f:
                custom_entries = json.load(f)
                all_entries.extend(custom_entries)
                print(f"📋 Loaded {len(custom_entries)} entries from custom manifest")
        else:
            print("⚠ Custom manifest not found, skipping")
            
        # Load PDF.js manifest
        if self.pdfjs_manifest_path.exists():
            with open(self.pdfjs_manifest_path, 'r') as f:
                pdfjs_entries = json.load(f)
                all_entries.extend(pdfjs_entries)
                print(f"📋 Loaded {len(pdfjs_entries)} entries from PDF.js manifest")
        else:
            print("⚠ PDF.js manifest not found, skipping")
            
        if not all_entries:
            raise FileNotFoundError("No manifest files found or all manifests are empty")
            
        return all_entries
        
    def _remote_source(self, *, is_pdfjs: bool, is_pdfbox: bool, is_corpus: bool) -> tuple[str, str | None]:
        """Return the base URL and downloads subdirectory for remote entries."""
        if is_pdfjs:
            return "https://hayro-assets.dev/pdfjs/", "pdfjs"
        if is_pdfbox:
            return "https://hayro-assets.dev/pdfbox/", "pdfbox"
        if is_corpus:
            return "https://hayro-assets.dev/corpus/", "corpus"
        return "https://hayro-assets.dev/custom/", None

    def _normalize_entry(self, entry, *, assume_link: bool = False):
        """Ensure manifest entries are dictionaries with at least id/link fields."""
        if isinstance(entry, str):
            return {"id": entry, "link": assume_link}
        if assume_link and not entry.get("link"):
            entry = dict(entry)
            entry["link"] = True
        return entry

    def _schedule_entry(self, entry: dict, rust_functions: list, download_futures: dict,
                        executor: ThreadPoolExecutor, processed_count: int, skipped_count: int,
                        *, is_pdfjs: bool = False, is_pdfbox: bool = False, is_corpus: bool = False) -> tuple[int, int]:
        """Validate an entry and either queue a download or record it immediately."""
        entry_id = entry['id']
        is_link = entry.get('link', False)
        is_ignored = entry.get('ignore', False)

        if is_ignored:
            print(f"⏭ Skipping {entry_id} (ignored)")
            return processed_count, skipped_count + 1

        if is_link:
            base_url, subdir = self._remote_source(is_pdfjs=is_pdfjs, is_pdfbox=is_pdfbox, is_corpus=is_corpus)
            url = f"{base_url}{entry_id}.pdf"

            index = len(rust_functions)
            rust_functions.append(None)
            future = executor.submit(
                self.download_pdf,
                entry_id,
                url,
                subdir,
            )
            download_futures[future] = (entry, is_pdfjs, is_pdfbox, is_corpus, index)
        else:
            if 'file' not in entry:
                print(f"✘ Missing file path for entry {entry_id}")
                return processed_count, skipped_count + 1

            relative_path = entry['file'].replace('pdfs/', '')
            if is_pdfjs:
                base_dir = self.pdfs_dir / 'pdfjs'
            elif is_pdfbox:
                base_dir = self.pdfs_dir / 'pdfbox'
            elif is_corpus:
                base_dir = self.pdfs_dir / 'corpus'
            else:
                base_dir = self.pdfs_dir

            target_path = base_dir / relative_path

            if not target_path.exists():
                print(f"✘ PDF file not found: {target_path}")
                return processed_count, skipped_count + 1

            rust_functions.append(
                self.generate_rust_function(
                    entry,
                    is_pdfjs=is_pdfjs,
                    is_pdfbox=is_pdfbox,
                    is_corpus=is_corpus,
                )
            )
            processed_count += 1

        return processed_count, skipped_count
        
    def generate_rust_function(self, entry: dict, is_pdfjs: bool = False, is_pdfbox: bool = False, is_corpus: bool = False) -> str:
        """Generate Rust test function for a manifest entry."""
        entry_id = entry['id']
        is_link = entry.get('link', False)
        first_page = entry.get('first_page')
        last_page = entry.get('last_page')
        
        # Generate page range string if specified
        if first_page is not None and last_page is not None:
            # Both start and end specified: "3..=7"
            length = f'Some("{first_page}..={last_page}")'
        elif first_page is not None:
            # Only start specified: "3.."
            length = f'Some("{first_page}..")'
        elif last_page is not None:
            # Only end specified: "..=7"
            length = f'Some("..={last_page}")'
        else:
            # No page range specified
            length = "None"
        
        # Generate file path and function name
        if is_pdfjs:
            if is_link:
                file_path = f"downloads/pdfjs/{entry_id}.pdf"
            else:
                # Remove pdfs/ prefix and add pdfjs subdirectory
                original_file = entry['file'].replace('pdfs/', '')
                file_path = f"pdfs/pdfjs/{original_file}"
            func_name = f"pdfjs_{entry_id.replace('-', '_').replace('.', '_')}"
        elif is_pdfbox:
            if is_link:
                file_path = f"downloads/pdfbox/{entry_id}.pdf"
            else:
                # Remove pdfs/ prefix and add pdfbox subdirectory
                original_file = entry['file'].replace('pdfs/', '')
                file_path = f"pdfs/pdfbox/{original_file}"
            func_name = f"pdfbox_{entry_id.replace('-', '_').replace('.', '_')}"
        elif is_corpus:
            if is_link:
                file_path = f"downloads/corpus/{entry_id}.pdf"
            else:
                # Remove pdfs/ prefix and add corpus subdirectory
                original_file = entry['file'].replace('pdfs/', '')
                file_path = f"pdfs/corpus/{original_file}"
            func_name = f"corpus_{entry_id.replace('-', '_').replace('.', '_')}"
        else:
            if is_link:
                file_path = f"downloads/{entry_id}.pdf"
            else:
                file_path = entry['file']
            func_name = entry_id.replace('-', '_').replace('.', '_')
            
        return f'#[test] fn {func_name}() {{ run_render_test("{func_name}", "{file_path}", {length}); }}'
        
    def generate_tests(self):
        """Main function to generate tests from manifest."""
        print("🚀 Starting test generation from manifest...")
        
        # Ensure downloads and corpus directories exist
        self.ensure_downloads_dir()
        self.ensure_corpus_dir()
        
        # Process all entries and generate Rust functions
        rust_functions = []
        processed_count = 0
        skipped_count = 0
        download_futures = {}
        max_workers = min(16, (os.cpu_count() or 4) * 2)
        
        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            # Load and process custom manifest
            if self.custom_manifest_path.exists():
                with open(self.custom_manifest_path, 'r') as f:
                    custom_entries = json.load(f)
                    print(f"📋 Processing {len(custom_entries)} custom entries")
                    
                    for entry in custom_entries:
                        processed_count, skipped_count = self._schedule_entry(
                            self._normalize_entry(entry),
                            rust_functions,
                            download_futures,
                            executor,
                            processed_count,
                            skipped_count,
                        )
            else:
                print("⚠ Custom manifest not found, skipping")
                
            # Load and process PDF.js manifest
            if self.pdfjs_manifest_path.exists():
                with open(self.pdfjs_manifest_path, 'r') as f:
                    pdfjs_entries = json.load(f)
                    print(f"📋 Processing {len(pdfjs_entries)} PDF.js entries")
                    
                    for entry in pdfjs_entries:
                        processed_count, skipped_count = self._schedule_entry(
                            self._normalize_entry(entry, assume_link=True),
                            rust_functions,
                            download_futures,
                            executor,
                            processed_count,
                            skipped_count,
                            is_pdfjs=True,
                        )
            else:
                print("⚠ PDF.js manifest not found, skipping")
                
            # Load and process pdfbox manifest
            if self.pdfbox_manifest_path.exists():
                with open(self.pdfbox_manifest_path, 'r') as f:
                    pdfbox_entries = json.load(f)
                    print(f"📋 Processing {len(pdfbox_entries)} pdfbox entries")

                    for entry in pdfbox_entries:
                        processed_count, skipped_count = self._schedule_entry(
                            self._normalize_entry(entry, assume_link=True),
                            rust_functions,
                            download_futures,
                            executor,
                            processed_count,
                            skipped_count,
                            is_pdfbox=True,
                        )
            else:
                print("⚠ Pdfbox manifest not found, skipping")

            # Load and process corpus manifest
            if self.corpus_manifest_path.exists():
                with open(self.corpus_manifest_path, 'r') as f:
                    corpus_entries = json.load(f)
                    print(f"📋 Processing {len(corpus_entries)} corpus entries")

                    for entry in corpus_entries:
                        processed_count, skipped_count = self._schedule_entry(
                            self._normalize_entry(entry, assume_link=True),
                            rust_functions,
                            download_futures,
                            executor,
                            processed_count,
                            skipped_count,
                            is_corpus=True,
                        )
            else:
                print("⚠ Corpus manifest not found, skipping")

            if download_futures:
                for future in as_completed(download_futures):
                    entry, is_pdfjs, is_pdfbox, is_corpus, index = download_futures[future]
                    entry_id = entry['id']
                    try:
                        success = future.result()
                    except Exception as exc:
                        print(f"✘ Failed to download {entry_id}: {exc}")
                        success = False

                    if success:
                        rust_functions[index] = self.generate_rust_function(
                            entry,
                            is_pdfjs=is_pdfjs,
                            is_pdfbox=is_pdfbox,
                            is_corpus=is_corpus,
                        )
                        processed_count += 1
                    else:
                        print(f"✘ Failed to download {entry_id}")
                        skipped_count += 1
                        rust_functions[index] = None
            
        rust_functions = [fn for fn in rust_functions if fn]

        if not rust_functions:
            print("✘ No test functions generated")
            return
                
        # Write Rust test file
        try:
            with open(self.output_file, 'w') as f:
                f.write('use crate::run_render_test;\n\n')
                f.write('\n'.join(rust_functions))
                
            print(f"\n🎉 Generated {len(rust_functions)} Rust test functions")
            print(f"📄 Output written to: {self.output_file}")
            print(f"📊 Summary: {processed_count} processed, {skipped_count} skipped")
            
        except Exception as e:
            print(f"✘ Failed to write test file: {e}")

def main():
    generator = TestGenerator()
    generator.generate_tests()

if __name__ == '__main__':
    main()
