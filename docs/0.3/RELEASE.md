# FolioForge 0.3.1 Release

Product version: `0.3.1`.

FolioForge 0.3.1 includes the current Rust Core and desktop functionality in
one versioned release. Cargo packages, Core/FFI/CLI/Service version responses,
desktop bundle metadata, and downloaded package `VERSION` files must all match
the release tag.

## Features and fixes

- Book conversion, compatibility analysis, structured diagnostics, batch
  processing, progress, and cancellation are available through the shared Core
  contracts.
- The desktop application includes English and Simplified Chinese, a Reader
  preview, and the supported comic workflow for image folders and ZIP/CBZ
  sources, with CBZ output.
- Windows desktop packages now launch without opening a console window. The
  command-line application continues to use the console normally.
- Release packaging rejects any mismatch between the tag, Rust package
  versions, app bundle version, and archive version metadata.

## Downloads

The tagged release contains six native packages and `SHA256SUMS`:

| Package | Platform |
| --- | --- |
| `FolioForge-0.3.1-macos-arm64-swiftui.zip` | macOS ARM64, macOS 27+ |
| `FolioForge-0.3.1-macos-arm64-slint.zip` | macOS ARM64, macOS 27+ |
| `FolioForge-0.3.1-linux-x86_64-slint.tar.gz` | Linux x86_64 |
| `FolioForge-0.3.1-linux-aarch64-slint.tar.gz` | Linux ARM64 |
| `FolioForge-0.3.1-windows-x86_64-slint.zip` | Windows x86_64 |
| `FolioForge-0.3.1-windows-arm64-slint.zip` | Windows ARM64 |

Linux packages are native GNU/Linux executables and require the documented
X11/XCB, xkbcommon, Fontconfig and FreeType runtime libraries. Wayland sessions
require XWayland for the current windowing backend.

All macOS packages published by GitHub Actions are unsigned and not
notarized. No local signing identity or certificate is used. Verify all
downloaded packages with `sha256sum -c SHA256SUMS`.
