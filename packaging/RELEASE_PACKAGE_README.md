# FolioForge 0.3.0 desktop packages

Each archive contains the FolioForge desktop app for the architecture in its
filename, a `VERSION` file, and the project `LICENSE`.

## Run the included app

- `*-macos-arm64-swiftui.zip`: open `FolioForge.app` (Apple Silicon, macOS 27
  or later).
- `*-macos-arm64-slint.zip`: open `FolioForge-Slint.app` (Apple Silicon,
  macOS 27 or later).
- `*-linux-*-slint.tar.gz`: extract the archive and run `./folioforge-slint`
  from a graphical desktop session.
- `*-windows-*-slint.zip`: extract the archive and run
  `folioforge-slint.exe`.

The Linux builds are native GNU/Linux executables dynamically linked to system
libraries. They require X11/XCB, xkbcommon, Fontconfig and FreeType runtime
libraries; Wayland sessions need XWayland support for the current native Winit
backend.

All macOS packages are intentionally unsigned and not notarized. On first
launch, macOS may require the user to approve opening the downloaded app.
Verify downloaded files against `SHA256SUMS` from the GitHub Release.
