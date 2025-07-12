"""
PDF.js Test Synchronization Script

This script synchronizes PDF test files from a PDF.js repository to the local project.
It supports three types of test selection:

1. Whitelist: Explicitly listed tests that are always included (if not blacklisted)
2. Alphabetical: First N tests in alphabetical order (configurable via --max-alphabetical)
3. Blacklist: Pattern-based exclusion of tests (supports wildcards like annotation_*)

Usage:
    python sync_pdfjs.py                           # Sync only whitelisted tests
    python sync_pdfjs.py --max-alphabetical 20    # Sync whitelist + first 20 alphabetical tests
    python sync_pdfjs.py --preview                 # Preview selection without syncing
    python sync_pdfjs.py --list-blacklisted       # Show blacklisted tests
"""

import json
import hashlib
import requests
import fnmatch
import shutil
from pathlib import Path
from typing import List, Dict, Any

def load_list_from_file(file_path: Path) -> List[str]:
    """Load a list of patterns from a text file, one per line."""
    if not file_path.exists():
        return []
    
    patterns = []
    with open(file_path, 'r') as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith('#'):  # Skip empty lines and comments
                patterns.append(line)
    return patterns

class PDFJSSync:
    def __init__(self, max_alphabetical_tests: int = 0):
        self.script_dir = Path(__file__).resolve().parent
        self.pdfjs_test_dir = Path("/Users/lstampfl/Programming/GitHub/pdf.js/test")
        self.pdfjs_pdfs_dir = self.pdfjs_test_dir / "pdfs"
        self.pdfjs_manifest_path = self.pdfjs_test_dir / "test_manifest.json"
        
        self.our_pdfs_dir = self.script_dir / "pdfs"
        self.our_downloads_dir = self.script_dir / "downloads"
        self.our_pdfjs_manifest_path = self.script_dir / "manifest_pdfjs.json"
        
        # Maximum number of alphabetical tests to include (in addition to whitelist)
        self.max_alphabetical_tests = max_alphabetical_tests
        
        # Load whitelist and blacklist from files
        self.whitelist_path = self.script_dir / "whitelist.txt"
        self.blacklist_path = self.script_dir / "blacklist.txt"
        self.whitelist = load_list_from_file(self.whitelist_path)
        self.blacklist = load_list_from_file(self.blacklist_path)
        
    def load_pdfjs_manifest(self) -> List[Dict[str, Any]]:
        """Load the PDF.js test manifest."""
        if not self.pdfjs_manifest_path.exists():
            raise FileNotFoundError(f"PDF.js manifest not found: {self.pdfjs_manifest_path}")
            
        with open(self.pdfjs_manifest_path, 'r') as f:
            return json.load(f)
            
    def matches_whitelist(self, test_id: str) -> bool:
        """Check if test_id matches any pattern in the whitelist."""
        for pattern in self.whitelist:
            if fnmatch.fnmatch(test_id, pattern):
                return True
        return False
        
    def matches_blacklist(self, test_id: str) -> bool:
        """Check if test_id matches any pattern in the blacklist."""
        for pattern in self.blacklist:
            if fnmatch.fnmatch(test_id, pattern):
                return True
        return False
        
    def has_excluded_flags(self, entry: Dict[str, Any]) -> bool:
        """Check if entry has flags that should be excluded."""
        excluded_flags = ['annotations', 'enableXfa', 'forms', 'print', 'optionalContent']
        for flag in excluded_flags:
            if entry.get(flag, False):
                return True
        return False
        
    def calculate_md5(self, file_path: Path) -> str:
        """Calculate MD5 hash of a file."""
        hash_md5 = hashlib.md5()
        with open(file_path, "rb") as f:
            for chunk in iter(lambda: f.read(4096), b""):
                hash_md5.update(chunk)
        return hash_md5.hexdigest()
        
    def download_pdf_from_link(self, link_file_path: Path, expected_md5: str, dest_name: str) -> bool:
        """Download PDF from a .link file and verify MD5."""
        # Create pdfjs subdirectory in downloads
        pdfjs_downloads_dir = self.our_downloads_dir / "pdfjs"
        pdfjs_downloads_dir.mkdir(exist_ok=True)
        dest_path = pdfjs_downloads_dir / f"{dest_name}.pdf"
        
        # Check if already downloaded with correct MD5
        if dest_path.exists():
            actual_md5 = self.calculate_md5(dest_path)
            if actual_md5 == expected_md5:
                print(f"✔ {dest_name} already downloaded with correct MD5")
                return True
            else:
                print(f"✘ {dest_name} exists but MD5 mismatch!")
                print(f"  Expected: {expected_md5}")
                print(f"  Actual:   {actual_md5}")
                raise RuntimeError(f"Existing file {dest_name} has wrong MD5. Expected: {expected_md5}, Actual: {actual_md5}")
        
        # Read URL from .link file
        url = link_file_path.read_text().strip()
        print(f"📥 Downloading {dest_name} from {url[:60]}...")
        
        try:
            response = requests.get(url, stream=True, timeout=60)
            response.raise_for_status()
            
            with open(dest_path, 'wb') as f:
                for chunk in response.iter_content(chunk_size=8192):
                    f.write(chunk)
            
            # Verify MD5
            actual_md5 = self.calculate_md5(dest_path)
            if actual_md5 == expected_md5:
                print(f"✔ Downloaded and verified MD5")
                return True
            else:
                print(f"✘ MD5 verification failed!")
                print(f"  Expected: {expected_md5}")
                print(f"  Actual:   {actual_md5}")
                dest_path.unlink()  # Delete the incorrect file
                raise RuntimeError(f"MD5 verification failed for {dest_name}. Expected: {expected_md5}, Actual: {actual_md5}")
                
        except requests.RequestException as e:
            print(f"✘ Failed to download {dest_name}: {e}")
            return False
            
    def copy_pdf_file(self, source_path: Path, dest_name: str) -> bool:
        """Copy a PDF file from PDF.js to our pdfs directory."""
        # Create pdfjs subdirectory in pdfs
        pdfjs_pdfs_dir = self.our_pdfs_dir / "pdfjs"
        pdfjs_pdfs_dir.mkdir(exist_ok=True)
        dest_path = pdfjs_pdfs_dir / f"{dest_name}.pdf"
        
        try:
            if dest_path.exists():
                print(f"✔ {dest_name}.pdf already exists, skipping copy")
                return True
                
            shutil.copy2(source_path, dest_path)
            print(f"📄 Copied {dest_name}.pdf")
            return True
            
        except Exception as e:
            print(f"✘ Failed to copy {dest_name}.pdf: {e}")
            return False
            
    def copy_link_file(self, source_path: Path, dest_name: str) -> bool:
        """Copy a .link file from PDF.js to our pdfs directory."""
        # Create pdfjs subdirectory in pdfs
        pdfjs_pdfs_dir = self.our_pdfs_dir / "pdfjs"
        pdfjs_pdfs_dir.mkdir(exist_ok=True)
        dest_path = pdfjs_pdfs_dir / f"{dest_name}.link"
        
        try:
            if dest_path.exists():
                print(f"✔ {dest_name}.link already exists, skipping copy")
                return True
                
            shutil.copy2(source_path, dest_path)
            print(f"🔗 Copied {dest_name}.link")
            return True
            
        except Exception as e:
            print(f"✘ Failed to copy {dest_name}.link: {e}")
            return False
            
    def convert_pdfjs_entry_to_our_format(self, entry: Dict[str, Any]) -> Dict[str, Any]:
        """Convert a PDF.js manifest entry to our manifest format."""
        
        # Determine the actual filename
        if entry.get("link", False):
            filename = f"{entry['id']}.link"
        else:
            file_path = entry.get("file", f"pdfs/{entry['id']}.pdf")
            if file_path.startswith("pdfs/"):
                filename = file_path[5:]  # Remove "pdfs/" prefix
            else:
                filename = file_path
        
        our_entry = {
            "id": entry["id"],
            "file": f"pdfs/{filename}"
        }
        
        # Copy MD5 if it's a link
        if entry.get("link", False):
            our_entry["md5"] = entry["md5"]
            our_entry["link"] = True
            
        # Convert page range (PDF.js uses firstPage/lastPage, we use first_page/last_page)
        if "firstPage" in entry:
            our_entry["first_page"] = entry["firstPage"] - 1  # PDF.js is 1-indexed, we are 0-indexed
        if "lastPage" in entry:
            our_entry["last_page"] = entry["lastPage"] - 1    # PDF.js is 1-indexed, we are 0-indexed
            
        return our_entry
        
    def load_existing_pdfjs_manifest(self) -> List[Dict[str, Any]]:
        """Load our existing PDF.js manifest if it exists."""
        if self.our_pdfjs_manifest_path.exists():
            with open(self.our_pdfjs_manifest_path, 'r') as f:
                return json.load(f)
        return []
        
    def cleanup_removed_entries(self, existing_entries: List[Dict[str, Any]], current_whitelist_ids: set):
        """Remove files for entries that are no longer in the whitelist."""
        removed_count = 0
        
        for entry in existing_entries:
            entry_id = entry["id"]
            if entry_id not in current_whitelist_ids:
                print(f"🧹 Cleaning up {entry_id} (no longer in whitelist)...")
                
                # Remove PDF file from pdfjs subdirectory
                pdf_path = self.our_pdfs_dir / "pdfjs" / f"{entry_id}.pdf"
                if pdf_path.exists():
                    pdf_path.unlink()
                    print(f"  ✔ Removed pdfjs/{entry_id}.pdf")
                
                # Remove link file from pdfjs subdirectory
                link_path = self.our_pdfs_dir / "pdfjs" / f"{entry_id}.link"
                if link_path.exists():
                    link_path.unlink()
                    print(f"  ✔ Removed pdfjs/{entry_id}.link")
                    
                # Remove downloaded file from pdfjs subdirectory
                download_path = self.our_downloads_dir / "pdfjs" / f"{entry_id}.pdf"
                if download_path.exists():
                    download_path.unlink()
                    print(f"  ✔ Removed downloads/pdfjs/{entry_id}.pdf")
                    
                removed_count += 1
                
        if removed_count > 0:
            print(f"🧹 Cleaned up {removed_count} removed entries")
        
    def cleanup_stale_files(self):
        """Remove PDF.js files from old locations (root directories) that are now in subdirectories."""
        print("🧹 Cleaning up stale PDF.js files from old locations...")
        
        # Load our current PDF.js manifest to know which files should be in subdirectories
        if not self.our_pdfjs_manifest_path.exists():
            print("⚠ No PDF.js manifest found, skipping stale file cleanup")
            return
            
        with open(self.our_pdfjs_manifest_path, 'r') as f:
            pdfjs_entries = json.load(f)
            
        cleaned_count = 0
        
        for entry in pdfjs_entries:
            entry_id = entry["id"]
            is_link = entry.get("link", False)
            
            # Clean up stale PDF files from root pdfs directory
            if not is_link:
                # Get the actual filename from the manifest
                file_path = entry["file"]
                if file_path.startswith("pdfs/"):
                    filename = file_path[5:]  # Remove "pdfs/" prefix
                else:
                    filename = file_path
                    
                stale_pdf_path = self.our_pdfs_dir / filename
                if stale_pdf_path.exists():
                    stale_pdf_path.unlink()
                    print(f"  ✔ Removed stale {filename}")
                    cleaned_count += 1
            
            # Clean up stale link files from root pdfs directory
            if is_link:
                stale_link_path = self.our_pdfs_dir / f"{entry_id}.link"
                if stale_link_path.exists():
                    stale_link_path.unlink()
                    print(f"  ✔ Removed stale {entry_id}.link")
                    cleaned_count += 1
            
            # Clean up stale downloaded files from root downloads directory
            stale_download_path = self.our_downloads_dir / f"{entry_id}.pdf"
            if stale_download_path.exists():
                stale_download_path.unlink()
                print(f"  ✔ Removed stale downloads/{entry_id}.pdf")
                cleaned_count += 1
                
        if cleaned_count > 0:
            print(f"🧹 Cleaned up {cleaned_count} stale files")
        else:
            print("✔ No stale files found")

    def sync(self):
        """Main synchronization function."""
        print("🚀 Starting PDF.js test synchronization...")
        print(f"📋 Whitelist patterns: {len(self.whitelist)} entries")
        print(f"🚫 Blacklist patterns: {len(self.blacklist)} entries")
        print(f"🔤 Max alphabetical tests: {self.max_alphabetical_tests}")
        print(f"🚫 Excluded flags: annotations, enableXfa, forms, print, optionalContent")
        
        # This will be loaded later in the filtering section
        
        # Load PDF.js manifest
        try:
            pdfjs_manifest = self.load_pdfjs_manifest()
            print(f"📄 Loaded PDF.js manifest with {len(pdfjs_manifest)} entries")
        except Exception as e:
            print(f"✘ Failed to load PDF.js manifest: {e}")
            return
            
        # Load existing manifest to know which tests are already ported
        existing_entries = self.load_existing_pdfjs_manifest()
        existing_ids = {entry["id"] for entry in existing_entries}
        
        # Filter entries using combined whitelist + alphabetical + blacklist + flags logic
        matching_entries = []
        
        # Step 1: Add explicitly whitelisted entries (not blacklisted, not with excluded flags)
        whitelisted_entries = []
        for entry in pdfjs_manifest:
            if (self.matches_whitelist(entry["id"]) and 
                not self.matches_blacklist(entry["id"]) and 
                not self.has_excluded_flags(entry)):
                whitelisted_entries.append(entry)
                matching_entries.append(entry)
                
        print(f"📋 Found {len(whitelisted_entries)} whitelisted entries")
        
        # Step 2: Add first N alphabetical entries (excluding already whitelisted and existing)
        if self.max_alphabetical_tests > 0:
            whitelisted_ids = {entry["id"] for entry in whitelisted_entries}
            all_existing_ids = existing_ids | whitelisted_ids
            
            alphabetical_entries = self.get_first_n_alphabetical_tests(
                pdfjs_manifest, 
                self.max_alphabetical_tests, 
                all_existing_ids
            )
            
            matching_entries.extend(alphabetical_entries)
            print(f"🔤 Added {len(alphabetical_entries)} alphabetical entries (max: {self.max_alphabetical_tests})")
        
        # Calculate statistics
        total_tests = len(pdfjs_manifest)
        excluded_by_flags = len([e for e in pdfjs_manifest if self.has_excluded_flags(e)])
        excluded_by_blacklist = len([e for e in pdfjs_manifest if self.matches_blacklist(e["id"])])
        already_ported = len(existing_ids)
        available_for_porting = total_tests - excluded_by_flags - excluded_by_blacklist
        not_yet_ported = available_for_porting - already_ported
        
        print(f"📊 Statistics:")
        print(f"  Total tests in PDF.js: {total_tests}")
        print(f"  Excluded by flags: {excluded_by_flags}")
        print(f"  Excluded by blacklist: {excluded_by_blacklist}")
        print(f"  Already ported: {already_ported}")
        print(f"  Available for porting: {available_for_porting}")
        print(f"  Not yet ported: {not_yet_ported}")
        print(f"🎯 Total matching entries for this sync: {len(matching_entries)}")
        
        # Get IDs that should be kept (not cleaned up)
        # This includes: explicitly whitelisted tests, tests already in our manifest, 
        # and tests that are NOT blacklisted AND don't have excluded flags
        keep_ids = set()
        
        # Always keep tests that are already in our manifest (they're working)
        keep_ids.update(existing_ids)
        
        for entry in pdfjs_manifest:
            test_id = entry["id"]
            # Keep if explicitly whitelisted
            if self.matches_whitelist(test_id):
                keep_ids.add(test_id)
            # Or keep if not blacklisted and doesn't have excluded flags
            elif not self.matches_blacklist(test_id) and not self.has_excluded_flags(entry):
                keep_ids.add(test_id)
        
        # Clean up entries that should no longer be kept (blacklisted or have excluded flags)
        if existing_entries:
            self.cleanup_removed_entries(existing_entries, keep_ids)
        
        if not matching_entries:
            print("ℹ No entries matched the whitelist patterns.")
            # Still write empty manifest to clear it
            with open(self.our_pdfjs_manifest_path, 'w') as f:
                json.dump([], f, indent=2)
            print("📄 Created empty manifest_pdfjs.json")
            return
            
        # Process each matching entry
        our_manifest_entries = []
        success_count = 0
        failed_count = 0
        
        for entry in matching_entries:
            entry_id = entry["id"]
            is_link = entry.get("link", False)
            
            print(f"\n📦 Processing {entry_id} ({'link' if is_link else 'pdf'})...")
            
            if is_link:
                # Handle .link files - use the file path from manifest
                file_path = entry.get("file", f"pdfs/{entry_id}.pdf")
                if file_path.startswith("pdfs/"):
                    actual_filename = file_path[5:]  # Remove "pdfs/" prefix
                else:
                    actual_filename = file_path
                    
                link_file_path = self.pdfjs_pdfs_dir / f"{actual_filename}.link"
                if not link_file_path.exists():
                    print(f"✘ Link file not found: {link_file_path}")
                    failed_count += 1
                    continue
                    
                # Copy the .link file
                if not self.copy_link_file(link_file_path, entry_id):
                    failed_count += 1
                    continue
                    
                # Download the PDF
                if not self.download_pdf_from_link(link_file_path, entry["md5"], entry_id):
                    failed_count += 1
                    continue
                    
            else:
                # Handle regular PDF files - use the actual filename from manifest
                file_path = entry.get("file", f"pdfs/{entry_id}.pdf")
                if file_path.startswith("pdfs/"):
                    actual_filename = file_path[5:]  # Remove "pdfs/" prefix
                else:
                    actual_filename = file_path
                    
                pdf_file_path = self.pdfjs_pdfs_dir / actual_filename
                if not pdf_file_path.exists():
                    print(f"✘ PDF file not found: {pdf_file_path}")
                    failed_count += 1
                    continue
                    
                # Copy the PDF file preserving the original filename
                dest_filename = actual_filename.replace('.pdf', '')  # Remove .pdf extension for dest name
                if not self.copy_pdf_file(pdf_file_path, dest_filename):
                    failed_count += 1
                    continue
                    
            # Convert to our manifest format
            our_entry = self.convert_pdfjs_entry_to_our_format(entry)
            our_manifest_entries.append(our_entry)
            success_count += 1
            
        # Merge with existing entries that should be kept
        all_manifest_entries = []
        new_entry_ids = {entry["id"] for entry in our_manifest_entries}
        
        # Add existing entries that should be kept (not in current selection)
        for existing_entry in existing_entries:
            if existing_entry["id"] not in new_entry_ids and existing_entry["id"] in keep_ids:
                all_manifest_entries.append(existing_entry)
                
        # Add newly processed entries
        all_manifest_entries.extend(our_manifest_entries)
        
        # Sort by ID for consistent ordering
        all_manifest_entries.sort(key=lambda x: x["id"])
        
        # Write our PDF.js manifest
        try:
            with open(self.our_pdfjs_manifest_path, 'w') as f:
                json.dump(all_manifest_entries, f, indent=2)
                
            print(f"\n🎉 Synchronization complete!")
            print(f"📄 Updated manifest_pdfjs.json with {len(all_manifest_entries)} total entries")
            print(f"📊 Summary: {success_count} new/updated, {failed_count} failed, {len(existing_entries) - len([e for e in existing_entries if e['id'] not in keep_ids])} preserved")
            
            # Clean up stale files from old locations
            self.cleanup_stale_files()
            
        except Exception as e:
            print(f"✘ Failed to write manifest_pdfjs.json: {e}")

    def get_first_n_alphabetical_tests(self, all_entries: List[Dict[str, Any]], n: int, existing_ids: set = None) -> List[Dict[str, Any]]:
        """Get the first N tests in alphabetical order, excluding blacklisted tests and existing tests."""
        if n <= 0:
            return []
            
        if existing_ids is None:
            existing_ids = set()
            
        # Filter out blacklisted tests, tests with excluded flags, and existing tests
        filtered_entries = []
        for entry in all_entries:
            if (not self.matches_blacklist(entry["id"]) and 
                not self.has_excluded_flags(entry) and 
                entry["id"] not in existing_ids):
                filtered_entries.append(entry)
        
        # Sort alphabetically and return first N entries
        sorted_entries = sorted(filtered_entries, key=lambda x: x["id"].lower())
        return sorted_entries[:n]
        
    def preview_selection(self):
        """Preview which tests would be selected without running the sync."""
        try:
            pdfjs_manifest = self.load_pdfjs_manifest()
            print(f"📄 Loaded PDF.js manifest with {len(pdfjs_manifest)} total entries")
        except Exception as e:
            print(f"✘ Failed to load PDF.js manifest: {e}")
            return
            
        # Load existing manifest
        existing_entries = self.load_existing_pdfjs_manifest()
        existing_ids = {entry["id"] for entry in existing_entries}
        
        # Get whitelisted entries
        whitelisted_entries = [entry for entry in pdfjs_manifest 
                             if (self.matches_whitelist(entry["id"]) and 
                                 not self.matches_blacklist(entry["id"]) and 
                                 not self.has_excluded_flags(entry))]
        
        print(f"\n📋 Whitelisted entries ({len(whitelisted_entries)}):")
        for entry in sorted(whitelisted_entries, key=lambda x: x["id"]):
            print(f"  - {entry['id']}")
            
        # Get alphabetical entries
        if self.max_alphabetical_tests > 0:
            whitelisted_ids = {entry["id"] for entry in whitelisted_entries}
            all_existing_ids = existing_ids | whitelisted_ids
            
            alphabetical_entries = self.get_first_n_alphabetical_tests(
                pdfjs_manifest, 
                self.max_alphabetical_tests, 
                all_existing_ids
            )
            
            print(f"\n🔤 Additional alphabetical entries ({len(alphabetical_entries)}):")
            for entry in alphabetical_entries:
                print(f"  - {entry['id']}")
        else:
            alphabetical_entries = []
                
        # Show statistics
        total_tests = len(pdfjs_manifest)
        excluded_by_flags = len([e for e in pdfjs_manifest if self.has_excluded_flags(e)])
        excluded_by_blacklist = len([e for e in pdfjs_manifest if self.matches_blacklist(e["id"])])
        already_ported = len(existing_ids)
        available_for_porting = total_tests - excluded_by_flags - excluded_by_blacklist
        not_yet_ported = available_for_porting - already_ported
        
        print(f"\n📊 Statistics:")
        print(f"  Total tests in PDF.js: {total_tests}")
        print(f"  Excluded by flags: {excluded_by_flags}")
        print(f"  Excluded by blacklist: {excluded_by_blacklist}")
        print(f"  Already ported: {already_ported}")
        print(f"  Available for porting: {available_for_porting}")
        print(f"  Not yet ported: {not_yet_ported}")
        
        total_selected = len(whitelisted_entries) + len(alphabetical_entries)
        print(f"\n🎯 Total entries that would be selected: {total_selected}")

def main():
    import argparse
    
    parser = argparse.ArgumentParser(description='Sync PDF.js test files')
    parser.add_argument('--max-alphabetical', type=int, default=0,
                        help='Maximum number of alphabetical tests to include (default: 0)')
    parser.add_argument('--list-blacklisted', action='store_true',
                        help='List tests that would be blacklisted and exit')
    parser.add_argument('--preview', action='store_true',
                        help='Preview which tests would be selected without syncing')
    
    args = parser.parse_args()
    
    syncer = PDFJSSync(max_alphabetical_tests=args.max_alphabetical)
    
    if args.list_blacklisted:
        # Load PDF.js manifest and show blacklisted entries
        try:
            pdfjs_manifest = syncer.load_pdfjs_manifest()
            blacklisted_entries = [entry for entry in pdfjs_manifest if syncer.matches_blacklist(entry["id"])]
            print(f"📋 Blacklisted entries ({len(blacklisted_entries)}):")
            for entry in sorted(blacklisted_entries, key=lambda x: x["id"]):
                print(f"  - {entry['id']}")
        except Exception as e:
            print(f"✘ Failed to load PDF.js manifest: {e}")
        return
        
    if args.preview:
        syncer.preview_selection()
        return
    
    syncer.sync()

if __name__ == '__main__':
    main() 