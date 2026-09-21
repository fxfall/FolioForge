# Phase 7.7 — GitHub Multi-Platform Build & Release CI

Status: implementation complete in the source tree; hosted-runner validation is
the remaining release gate. This phase is release plumbing only. It does not
add a format, change Semantic IR behavior, add Library/Reader capability, or
create a second conversion pipeline.

## Boundary and classification

- Change classification: minor maintenance/release infrastructure.
- Core remains the single CLI pipeline: input → Semantic IR → compatibility
  plan → exporter → validator.
- GUI remains SwiftUI → existing FFI → Core. The GUI artifact is self-contained
  and does not require the separately downloadable CLI.
- `folio-library` is compiled and tested by source CI but is not packaged and
  no Library database is allowed in a Core or GUI artifact.
- Public artifacts are intentionally unsigned. Developer ID signing,
  notarization and installers remain outside the 0.1 release scope.

## Workflow split

The public repository has exactly three workflow files:

| Workflow | Trigger | Responsibility |
| --- | --- | --- |
| `ci.yml` | push and pull request | Rust format, workspace tests, Clippy and release build; no release artifacts |
| `build.yml` | `main` push and manual dispatch | native multi-platform Core/GUI builds, packages and uploaded workflow artifacts |
| `release.yml` | `v*` tags | clean tagged rebuild, validation, six release packages, SHA256SUMS and GitHub Release publication |

The tagged workflow never downloads artifacts from `build.yml`; it rebuilds
from the tagged source checkout.

## Native build matrix

| Artifact | Runner | Target |
| --- | --- | --- |
| Linux Core x86_64 | `ubuntu-26.04` | `x86_64-unknown-linux-gnu` |
| Linux Core ARM64 | `ubuntu-26.04-arm` | `aarch64-unknown-linux-gnu` |
| Windows Core x86_64 | `windows-2025` | `x86_64-pc-windows-msvc` |
| Windows Core ARM64 | `windows-11-vs2026-arm` | `aarch64-pc-windows-msvc` |
| macOS 27 Core ARM64 | `xcode-27` | `aarch64-apple-darwin` |
| macOS 27 GUI ARM64 | `xcode-27` | SwiftPM arm64 target `arm64-apple-macosx27.0` plus the existing FFI archive |

ARM jobs build and execute their own binaries on the native ARM runner. No
QEMU, `cross`, `cargo-zigbuild` or x64-host Windows ARM cross build is used.
The Ubuntu 26.04 and Xcode 27 images are preview-capable hosted targets, so a
future runner-label change is isolated to the workflow files.

## Artifact contract

The release workflow publishes exactly these six archives plus `SHA256SUMS`:

```text
FolioForge-Core-0.1.0-linux-x86_64.tar.gz
FolioForge-Core-0.1.0-linux-aarch64.tar.gz
FolioForge-Core-0.1.0-windows-x86_64.zip
FolioForge-Core-0.1.0-windows-aarch64.zip
FolioForge-Core-0.1.0-macos27-aarch64.tar.gz
FolioForge-GUI-0.1.0-macos27-aarch64.zip
```

Core archives contain only `bin/folio` or `bin/folio.exe` and `LICENSE`.
The Service binary, Library database, private corpus, repository target tree
and machine-specific paths are not release contents.

## Gates implemented

Each Core job runs `--version` and `--help`, builds the target binary and
verifies the archive layout and the absence of SQLite Library data. Extended
fixture conversion, semantic parity, KFX probes and third-party converter
comparisons remain in the ignored local development bundle.

The GUI job builds with `FOLIOFORGE_MACOS_TARGET_VERSION=27.0` and checks:

- arm64 Mach-O executable;
- minimum system version `27.0`;
- checked-in logo/resource bundle and generated app icon;
- system-only dynamic dependencies, with repository/Homebrew paths rejected;
- no Library database;
- ZIP integrity;
- intentionally unsigned executable, matching the public 0.1 signing policy.

The public build helpers are `packaging/package_core_artifact.sh`,
`packaging/package_core_artifact.ps1` and `packaging/build_macos_app.sh`.
They contain no fixture generator or Python dependency. All build and scratch
data are created below `FOLIOFORGE_VALIDATION_ROOT` or the hosted runner
temporary directory.

## Validation record

Local validation on 2026-09-21 passed shell/Python syntax, workflow YAML
parsing, Rust formatting, the local architecture/degradation/matrix gates, the
Core macOS arm64 package, and the complete unsigned macOS 27 arm64 GUI package.
The GUI build used Swift 6.4 and confirmed the existing macOS
deprecation/linker warnings are non-fatal. The public GitHub workflow does not
require the local Python or oracle bundle.

The first hosted push (`a0868d3`, CI run `35545575349`) was rejected before
starting because GitHub does not expose the `runner` context in a job-level
`env` block. All job-level `runner.temp` references were removed; each build
job now initializes external paths from the runner-provided `$RUNNER_TEMP` and
persists them through `GITHUB_ENV`. The artifact action paths remain
step-level expressions, where the runner context is valid.
