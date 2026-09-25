# Changelog

## 0.3.1

### Features

- Align the Rust Core, FFI, CLI, Service, desktop app, and release-package
  versions to `0.3.1`.
- Provide localized book conversion, compatibility review, batch processing,
  Reader preview, and the supported image-folder/ZIP/CBZ comic workflow.
- Publish native desktop packages for Apple Silicon macOS, Linux x86_64/ARM64,
  and Windows x86_64/ARM64.

### Fixes

- Windows desktop launch no longer opens a console window; the command-line
  application remains a console program.
- Release packages reject mismatched Core, app-bundle, and archive versions.
