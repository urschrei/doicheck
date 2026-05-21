use std::path::Path;

fn main() {
    copy_pdfium_next_to_binary();
    tauri_build::build()
}

/// Copy the PDFium library from `pdfium/` next to the output binary so it can be
/// loaded in development and when running the raw target binary. The packaged
/// app loads it from the resource directory instead (see `bundle.resources` in
/// tauri.conf.json). Skipped silently if the library is not present.
fn copy_pdfium_next_to_binary() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let lib_name = if cfg!(target_os = "windows") {
        "pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    };
    let src = Path::new(&manifest_dir).join("pdfium").join(lib_name);
    println!("cargo:rerun-if-changed={}", src.display());
    if !src.exists() {
        return;
    }
    // OUT_DIR is target/<profile>/build/<pkg-hash>/out; the profile dir (which
    // holds the binary) is three levels up.
    let out_dir = std::env::var("OUT_DIR").unwrap();
    if let Some(profile_dir) = Path::new(&out_dir).ancestors().nth(3) {
        let _ = std::fs::copy(&src, profile_dir.join(lib_name));
    }
}
