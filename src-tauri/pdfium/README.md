# PDFium library

This directory holds the platform PDFium shared library used for PDF text
extraction (`libpdfium.dylib` on macOS, `pdfium.dll` on Windows, `libpdfium.so`
on Linux). The binary itself is git-ignored.

- It is bundled into the app via `bundle.resources` in `tauri.conf.json` and
  loaded at runtime from the resource directory.
- `build.rs` also copies it next to the compiled binary for development.
- CI fetches it per platform in `.github/workflows/release.yml`.

To populate it locally, download the matching build from
<https://github.com/bblanchon/pdfium-binaries/releases> and place the library
file here, e.g. on macOS:

```sh
curl -fsSL -o /tmp/pdfium.tgz \
  https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-mac-univ.tgz
tar -xzf /tmp/pdfium.tgz -C /tmp lib/libpdfium.dylib
cp /tmp/lib/libpdfium.dylib src-tauri/pdfium/libpdfium.dylib
```
