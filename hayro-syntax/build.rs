//! build-script for hayro-syntax
fn main() {
    println!(
        "cargo::rustc-check-cfg=cfg(reader_opt_ext_cache, reader_opt_multi_buffer, reader_opt_recording_marker)"
    );
    // optimization: use external reader cache
    println!("cargo::rustc-cfg=reader_opt_ext_cache");
    // optimization: complex multi buffer system instead of simple single-buffer system
    println!("cargo::rustc-cfg=reader_opt_multi_buffer");
    // optimization: recording marked during read instead of fetching again on take_marker
    println!("cargo::rustc-cfg=reader_opt_recording_marker");
}
