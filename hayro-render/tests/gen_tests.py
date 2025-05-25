import os

# Directory where your PDFs are located
pdf_directory = os.path.join(os.path.dirname(__file__), '../assets')  # relative to current Python file
downloads_directory = os.path.join(os.path.dirname(__file__), '../downloads')  # relative to current Python file
# Output Rust file
output_file = os.path.join(os.path.dirname(__file__), 'tests.rs')

ignore_list = ["pdftc_010k_0083", "integration_matplotlib", "pattern_shading_type6", "pattern_shading_type6_2", "image_jbig2_3", "pdfjs_issue2642"]

lengths = {
    "color_space_calrgb": "0..=8",
    "font_standard_2": "0..=0",
    "font_truetype_7": "1..=1",
    "font_type1_10": "0..=1",
    "font_cid_2": "0..=0",
    "issue_cmap_lookup": "0..=0",
    "issue_stream_abbreviations": "0..=0",
    "font_truetype_8": "0..=0",
    "image_inline_8": "2..=2",
    "stream_flate_bpc_1": "0..=0",
    "stream_jpx_1": "2..=2",
    "stream_jpx_3": "2..=2",
    "font_truetype_slow_post_lookup": "0..=0",
    "rendering_conflation_artifacts": "2..=2",
    "image_jbig2_2": "0..=4",
    "pdfjs_issue2642": "0..=0",
}

def generate_rust_function(file_stem, is_download):
    length = f"Some({lengths[file_stem]})" if file_stem in lengths else "None"
    is_download = str(is_download).lower()
    return f"#[test] fn {file_stem}() {{ run_test(\"{file_stem}\", {is_download}, {length}); }}"

def main():
    rust_functions = []
    
    names = [f for f in os.listdir(pdf_directory)]
    names.sort()

    for filename in names:
        if filename.endswith('.link') | filename.endswith('.pdf'):
            file_stem = os.path.splitext(filename)[0]
            if file_stem not in ignore_list:
                is_download = filename.endswith('.link')
                rust_functions.append(generate_rust_function(file_stem, is_download))

    with open(output_file, 'w') as f:
        f.write('use crate::run_test;\n\n')
        f.write('\n'.join(rust_functions))

    print(f"Generated {len(rust_functions)} Rust test functions into {output_file}")

if __name__ == '__main__':
    main()
