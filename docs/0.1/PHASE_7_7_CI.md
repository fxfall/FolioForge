# Phase 7.7 — GitHub Linux/macOS Build & Release CI

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
- `folio-library` is compiled by source CI but is not packaged and no Library
  database is allowed in a Core or GUI artifact. Its regression tests are
  maintainer-local and are not run by GitHub.
- Public artifacts are intentionally unsigned. Both GitHub macOS GUI workflows
  explicitly disable signing, independent of any certificates available on a
  runner. Local maintainer packaging may use a keychain identity, but this is
  not Developer ID release signing, notarization or an installer; those remain
  outside the 0.1 release scope.

## Workflow split

The frozen 0.1 release line uses these three workflows. The additive 0.3 GUI
release has its own tagged workflow, documented in `docs/0.3/RELEASE.md`.

| Workflow | Trigger | Responsibility |
| --- | --- | --- |
| `ci.yml` | push and pull request | Rust format, production-target Clippy and release build; no tests or release artifacts |
| `build.yml` | `main` push and manual dispatch | native Linux/macOS Core/GUI builds, packages and uploaded workflow artifacts |
| `release.yml` | `v0.1.0` and `v0.1.0-rc.*` tags | clean tagged rebuild, validation, four release packages, SHA256SUMS and GitHub Release publication |

The tagged workflow never downloads artifacts from `build.yml`; it rebuilds
from the tagged source checkout.

## Native build matrix

| Artifact | Runner | Target |
| --- | --- | --- |
| Linux Core x86_64 | `ubuntu-26.04` | `x86_64-unknown-linux-gnu` |
| Linux Core ARM64 | `ubuntu-26.04-arm` | `aarch64-unknown-linux-gnu` |
| macOS 27 Core ARM64 | `xcode-27` | `aarch64-apple-darwin` |
| macOS 27 GUI ARM64 | `xcode-27` | SwiftPM arm64 target `arm64-apple-macosx27.0` plus the existing FFI archive |

ARM jobs build and execute their own binaries on the native ARM runner. No
QEMU, `cross` or `cargo-zigbuild` is used.
The Ubuntu 26.04 and Xcode 27 images are preview-capable hosted targets, so a
future runner-label change is isolated to the workflow files.

## Artifact contract

The release workflow publishes exactly these four archives plus `SHA256SUMS`:

```text
FolioForge-Core-0.1.0-linux-x86_64.tar.gz
FolioForge-Core-0.1.0-linux-aarch64.tar.gz
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

The public build helpers are `packaging/package_core_artifact.sh` and
`packaging/build_macos_app.sh`.
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

The former Windows Core jobs and Windows packaging helper were removed on
2026-09-24. They were an unvalidated release path that remained queued without
producing a usable artifact. Windows packaging is not part of the frozen 0.1
public artifact contract; reintroducing it requires a separately validated
runner, packaging contract and release audit.
