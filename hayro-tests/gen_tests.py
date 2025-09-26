import os
import json
import hashlib
import requests
from pathlib import Path

class TestGenerator:
    def __init__(self):
        self.script_dir = Path(__file__).resolve().parent
        self.custom_manifest_path = self.script_dir / 'manifest.json'
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
        
    def calculate_md5(self, file_path: Path) -> str:
        """Calculate MD5 hash of a file."""
        hash_md5 = hashlib.md5()
        with open(file_path, "rb") as f:
            for chunk in iter(lambda: f.read(4096), b""):
                hash_md5.update(chunk)
        return hash_md5.hexdigest()
        
    def download_pdf(self, link_path: Path, expected_md5: str = None, is_external: bool = False) -> bool:
        """Download PDF from link file and verify MD5 if provided."""
        stem = link_path.stem
        
        # Store downloads in appropriate subdirectory for external entries
        if is_external:
            # Determine subdirectory based on link file location
            if 'pdfjs' in str(link_path):
                dest_dir = self.downloads_dir / "pdfjs"
            elif 'pdfbox' in str(link_path):
                dest_dir = self.downloads_dir / "pdfbox"
            elif 'corpus' in str(link_path):
                dest_dir = self.downloads_dir / "corpus"
            else:
                dest_dir = self.downloads_dir
            dest_dir.mkdir(exist_ok=True)
            dest_path = dest_dir / f"{stem}.pdf"
        else:
            dest_path = self.downloads_dir / f"{stem}.pdf"
        
        # If file exists, check MD5
        if dest_path.exists():
            if expected_md5:
                actual_md5 = self.calculate_md5(dest_path)
                if actual_md5 == expected_md5:
                    # print(f"✔ {stem} exists with correct MD5")
                    return True
                else:
                    print(f"⚠ {stem} exists but MD5 mismatch. Re-downloading...")
                    print(f"  Expected: {expected_md5}")
                    print(f"  Actual:   {actual_md5}")
            else:
                print(f"✔ Skipping {stem} (already downloaded, no MD5 verification)")
                return True
        
        # Download the file
        url = link_path.read_text().strip()
        print(f"📥 Downloading {stem} from {url[:50]}...")
        
        try:
            response = requests.get(url, stream=True, timeout=30)
            response.raise_for_status()
            
            with open(dest_path, 'wb') as f:
                for chunk in response.iter_content(chunk_size=8192):
                    f.write(chunk)
            
            # Verify MD5 if provided
            if expected_md5:
                actual_md5 = self.calculate_md5(dest_path)
                if actual_md5 == expected_md5:
                    print(f"✔ Downloaded and verified MD5")
                    return True
                else:
                    print(f"✘ MD5 verification failed!")
                    print(f"  Expected: {expected_md5}")
                    print(f"  Actual:   {actual_md5}")
                    dest_path.unlink()  # Delete the incorrect file
                    return False
            else:
                print(f"✔ Downloaded (no MD5 verification)")
                return True
                
        except requests.RequestException as e:
            print(f"✘ Failed to download {stem}: {e}")
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
            
    def process_entry(self, entry: dict, is_pdfjs: bool = False, is_pdfbox: bool = False, is_corpus: bool = False) -> bool:
        """Process a single manifest entry, downloading if necessary."""
        entry_id = entry['id']
        file_path = entry['file']
        is_link = entry.get('link', False)
        is_ignored = entry.get('ignore', False)
        expected_md5 = entry.get('md5')
        
        if is_ignored:
            print(f"⏭ Skipping {entry_id} (ignored)")
            return False
            
        if is_link:
            # Handle link files - they should be in pdfs/pdfjs/, pdfs/pdfbox/, or corpus/ for respective entries
            if is_pdfjs:
                link_path = self.pdfs_dir / f"pdfjs/{file_path.replace('pdfs/', '')}"
            elif is_pdfbox:
                link_path = self.pdfs_dir / f"pdfbox/{file_path.replace('pdfs/', '')}"
            elif is_corpus:
                link_path = self.pdfs_dir / f"corpus/{file_path.replace('pdfs/', '')}"
            else:
                link_path = self.pdfs_dir / file_path.replace('pdfs/', '')

            if not link_path.exists():
                print(f"✘ Link file not found: {link_path}")
                return False

            success = self.download_pdf(link_path, expected_md5, is_pdfjs or is_pdfbox or is_corpus)
            if not success:
                print(f"✘ Failed to download or verify {entry_id}")
                return False
        else:
            # Check if PDF file exists - in pdfs/pdfjs/, pdfs/pdfbox/, or corpus/ for respective entries
            if is_pdfjs:
                pdf_path = self.pdfs_dir / f"pdfjs/{file_path.replace('pdfs/', '')}"
            elif is_pdfbox:
                pdf_path = self.pdfs_dir / f"pdfbox/{file_path.replace('pdfs/', '')}"
            elif is_corpus:
                pdf_path = self.pdfs_dir / f"corpus/{file_path.replace('pdfs/', '')}"
            else:
                pdf_path = self.pdfs_dir / file_path.replace('pdfs/', '')

            if not pdf_path.exists():
                print(f"✘ PDF file not found: {pdf_path}")
                return False
                
        return True
        
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
        
        # Load and process custom manifest
        if self.custom_manifest_path.exists():
            with open(self.custom_manifest_path, 'r') as f:
                custom_entries = json.load(f)
                print(f"📋 Processing {len(custom_entries)} custom entries")
                
                for entry in custom_entries:
                    if self.process_entry(entry, is_pdfjs=False):
                        rust_functions.append(self.generate_rust_function(entry, is_pdfjs=False))
                        processed_count += 1
                    else:
                        skipped_count += 1
        else:
            print("⚠ Custom manifest not found, skipping")
            
        # Load and process PDF.js manifest
        if self.pdfjs_manifest_path.exists():
            with open(self.pdfjs_manifest_path, 'r') as f:
                pdfjs_entries = json.load(f)
                print(f"📋 Processing {len(pdfjs_entries)} PDF.js entries")
                
                for entry in pdfjs_entries:
                    if self.process_entry(entry, is_pdfjs=True):
                        rust_functions.append(self.generate_rust_function(entry, is_pdfjs=True))
                        processed_count += 1
                    else:
                        skipped_count += 1
        else:
            print("⚠ PDF.js manifest not found, skipping")
            
        # Load and process pdfbox manifest
        if self.pdfbox_manifest_path.exists():
            with open(self.pdfbox_manifest_path, 'r') as f:
                pdfbox_entries = json.load(f)
                print(f"📋 Processing {len(pdfbox_entries)} pdfbox entries")

                for entry in pdfbox_entries:
                    if self.process_entry(entry, is_pdfbox=True):
                        rust_functions.append(self.generate_rust_function(entry, is_pdfbox=True))
                        processed_count += 1
                    else:
                        skipped_count += 1
        else:
            print("⚠ Pdfbox manifest not found, skipping")

        # Load and process corpus manifest
        if self.corpus_manifest_path.exists():
            with open(self.corpus_manifest_path, 'r') as f:
                corpus_entries = json.load(f)
                print(f"📋 Processing {len(corpus_entries)} corpus entries")

                for entry in corpus_entries:
                    if self.process_entry(entry, is_corpus=True):
                        rust_functions.append(self.generate_rust_function(entry, is_corpus=True))
                        processed_count += 1
                    else:
                        skipped_count += 1
        else:
            print("⚠ Corpus manifest not found, skipping")
            
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
