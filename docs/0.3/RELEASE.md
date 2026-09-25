# FolioForge 0.3.0 GUI Release

FolioForge 0.3.0 publishes the macOS SwiftUI client and native Slint desktop
clients. This is a GUI/product release: Rust workspace/Core crates remain on
the frozen 0.1.0 package version, and this release does not claim a Core
conversion-engine version bump.

## Downloads

The tagged GitHub Release contains these six platform packages and a
`SHA256SUMS` file:

| Package | Frontend | Architecture |
| --- | --- | --- |
| `FolioForge-0.3.0-macos-arm64-swiftui.zip` | SwiftUI | macOS ARM64, macOS 27+ |
| `FolioForge-0.3.0-macos-arm64-slint.zip` | Slint | macOS ARM64, macOS 27+ |
| `FolioForge-0.3.0-linux-x86_64-slint.tar.gz` | Slint | Linux x86_64 |
| `FolioForge-0.3.0-linux-aarch64-slint.tar.gz` | Slint | Linux ARM64 |
| `FolioForge-0.3.0-windows-x86_64-slint.zip` | Slint | Windows x86_64 |
| `FolioForge-0.3.0-windows-arm64-slint.zip` | Slint | Windows ARM64 |

The macOS SwiftUI app remains the reference macOS client. The Slint macOS
package is a separately built companion frontend. All macOS packages are
unsigned and not notarized; no maintainer signing identity or certificate is
used by GitHub Actions.

Linux packages contain a native GNU/Linux executable and are dynamically
linked to the runner's system libraries. A graphical session plus X11/XCB,
xkbcommon, Fontconfig and FreeType runtime libraries are required. Wayland
desktop sessions need XWayland support for the current native Winit backend.

Every archive includes the project `LICENSE`, this release's Slint runtime
notes, and a `VERSION` file. Verify archives with `sha256sum -c SHA256SUMS`
after downloading all six packages and the checksum file into one directory.

## Release gates

The `v0.3.0` tag workflow rebuilds each target natively from the tagged commit,
checks the macOS 27 ARM64 app bundles remain unsigned, validates package
contents and Linux dynamic-library resolution, and creates checksums before
publishing. It does not download the ordinary branch build artifacts or
include private tests, fixtures, or development logs.
