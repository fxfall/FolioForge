# FolioForge 0.1 Final Audit

Status: local F0–F5 validation and hosted F7 CI passed on 2026-09-19. F6
repository sync is complete; F8 tag/RC validation remains pending. This file
is the only final audit for the public 0.1 working tree; historical
investigation reports are not authoritative.

## 1. Commit, version and date

- Product version: `0.1.0`.
- Candidate tag: `v0.1.0-rc.1`, followed by `v0.1.0` only after the RC gates.
- Finalization base commit: `7dd277803383b2829785cfe54ec73565d39047b5`
  (clean public baseline before this audit record commit).
- Audit date: 2026-09-19.
- Repository: `fxfall/FolioForge`.

Cargo workspace, CLI, Core, FFI, Service and bundle metadata must report
`0.1.0`. Core ABI (`1`), inspection contract (`folio-core-inspection-v1`) and
Library schema (`2`) are independent protocol versions.

## 2. Architecture audit

The architecture baseline was rechecked after document cleanup:

- one InputSource → Semantic IR → compatibility → writer pipeline;
- Core remains GUI/network/Library independent;
- FormatRegistry is the single input/target authority;
- generated output is validated before atomic commit;
- KFX research tools are external to runtime conversion logic.

Result: PASS (`python3 tests/architecture/check_boundaries.py`).

## 3. Module boundary audit

The architecture gate is `tests/architecture/check_boundaries.py`. Explicitly
forbidden edges include Core → Library and Library → concrete format adapters.
Result: PASS.

## 4. Format validation

The final local run recorded the following results:

- `cargo fmt --all -- --check`: PASS.
- `cargo test --workspace --locked`: PASS.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: PASS.
- `cargo build --workspace --locked --release`: PASS.
- degradation golden set: PASS, 10 cases.
- deterministic conversion matrix: PASS, 36 cases across 12 directions.
- public KFX semantic probes: PASS for all checked-in probe sources.

The first offline test attempt stopped at the uncached `blake3` package; the
same locked test was then rerun with the configured sparse registry and the
external target/cache root. This was an environment cache miss, not a source
or test failure. The public format matrix is in
[FORMAT_SUPPORT.md](FORMAT_SUPPORT.md).

## 5. Semantic validation

The IR validator, deterministic IDs/order, metadata/edit-plan path, anchors,
links, notes, resources, styles, fonts, ruby, MathML, tables, layout intent
and target degradation reports are covered by public fixtures and workspace
tests. The workspace test run passed all 85 KFX unit tests (one private-corpus
test remains opt-in). KFX input keeps unresolved relationships unresolved; it
does not guess from string pools.

## 6. Cross-format validation

The public matrix exercises EPUB, KF7, KF8 and internal KFX writer paths over
synthetic semantic fixtures. TXT, Markdown, HTML/HTMLZ, FB2 and DOCX enter the
same IR path. Private real-book and third-party converter outputs are not
release inputs.

## 7. Library validation

`folio-library` is optional to standalone conversion. The final run passed its
three Library contract tests, including schema migrations, scanner path safety,
metadata projection, rebuildable FTS search, query budgets and conversion
provenance. The durable contract is
[../library/DATA_MODEL.md](../library/DATA_MODEL.md).

## 8. FFI / Service / SwiftUI validation

The final run must include:

- CLI, Service and FFI release builds: PASS.
- Service health, capabilities, preflight, progress, downloads, limits and
  logo smoke: PASS.
- SwiftUI macOS Debug and Release SwiftPM builds against the matching FFI
  archive: PASS.
- FFI ownership, cancellation and progress callback tests: PASS.

The local Swift linker emitted the same two missing-Command-Line-Tools
search-path warnings in each configuration, but both builds completed
successfully. The hosted macOS runner remains the clean-clone confirmation for
this environment warning.

## 9. Security and fuzz boundary

The public gate checks archive/path limits, malformed input diagnostics,
protected KFX rejection without decryption, safe relative paths, bounded
Service uploads, no implicit network use and no tracked private/secret data.
Fuzzing is a security follow-up, not a claim that every malformed external
format is accepted.

## 10. Performance

Metrics are opt-in and deterministic output remains the correctness priority.
The release benchmark smoke passed with one run for each of four public seeds
(basic-text, basic-markdown, basic-html and basic-fb2), EPUB target, using an
external artifact root. The private large-book and large-Library measurements
are not CI gates.

## 11. Build validation

The final command list and prerequisites are in
[BUILD_RELEASE.md](BUILD_RELEASE.md). The `git clean -ndX` preview was empty
after generated outputs and stale `.DS_Store` files were removed from the
public tree. The repository audit is the final clean-tree gate before a remote
push. Never use a blind `git clean -fdx`.

## 12. Supported

EPUB/EPUB3, KF7/MOBI, KF8/AZW3, TXT, Markdown, HTML/HTMLZ, FB2, DOCX/OOXML
and DRM-free reflowable KFX input are supported through the Semantic IR within
the documented feature matrix. EPUB3, KF7, KF8, the explicit KF7+KF8 combo and
the explicit FolioForge KFX compatibility writer are output targets.

## 13. Approximation

Target profiles may approximate or flatten SVG, fonts, ruby, MathML, notes,
vertical writing, complex tables, floats, fixed positioning and other layout
features for KF7/KF8. Text import may infer paragraph/chapter boundaries.
Every selected fallback is reported.

## 14. Not exercised

Private 82-book KFX output files, Calibre/Bōkō/KP3 generated books, device
screenshots, proprietary KPF behavior, very large private benchmark runs,
Developer ID signing, notarization and Gatekeeper acceptance are not clean
clone/CI requirements.

## 15. Unsupported

DRM decryption, key recovery, fixed-layout/comic KFX as a general input
promise, cloud sync, Reader pagination/annotations and unproven source
relationships are unsupported. The internal KFX compatibility container is not
Amazon production KFX.

## 16. Out of scope

New formats, new Library/Reader features, a second pipeline, proprietary
converter dependencies in Core/CI, Windows packaging without a validated
runner and production signing/notarization are outside 0.1.

## 17. Final result

Local consolidation result: F0 code freeze, F1 documentation merge, F2 audit
convergence, F3 documentation cleanup, F4 artifact/private-data cleanup and
F5 local validation are complete. F7 hosted CI is green on the corrected
public commit. This audit may be marked release-ready only when every command
in `BUILD_RELEASE.md` and the RC fresh-clone checklist has a recorded PASS,
the working tree is clean, no private corpus or local path is tracked, and the
release tag points to the audited commit.

## 18. External release gates

- F6 is complete: `origin` is configured as
  `git@github.com:fxfall/FolioForge.git`, and the remote `main` was verified at
  the current public audit commit `ecc4173ea2317f4209ae45ef2071da37d879578`.
- F7 first hosted run `35428340869` reached both jobs but failed on maintenance
  gates: Linux Clippy reported four `chunks_exact_to_as_chunks` diagnostics;
  macOS SwiftUI Debug reported one strict-concurrency capture diagnostic.
- F7 corrected hosted run `35429930923` completed successfully on both Linux
  and macOS (3m35s) after the maintenance fixes below.
- F8 RC and final tags remain intentionally pending until the audited commit
  has passed the fresh-clone checklist and the RC release workflow has
  published validated artifacts.

## 19. Release correction log

- Change classification: minor maintenance; no public API, Semantic IR,
  format capability or conversion behavior was changed.
- Linux correction: fixed-length UTF-16 byte-pair handling in `folio-text` now
  uses `as_chunks::<2>()`, preserving the even-length guard and all detection
  thresholds. Local workspace Clippy and the text importer tests pass.
- macOS correction: the preflight result collection is frozen into an
  immutable local before the `@MainActor` task consumes it, removing the
  Swift strict-concurrency error without changing queue behavior. Local
  SwiftUI Debug and Release builds pass.
- Hosted Linux correction: the remaining fixed-width KFX resource hex decoder
  now uses `as_chunks::<2>()`, preserving the even-length guard and decoder
  behavior while satisfying the hosted Clippy baseline.
- Hosted macOS correction: SwiftPM now repeats the `bz2` and `lzma` native
  link libraries at the Swift executable boundary. Cargo records those flags
  for Rust binaries, but the Swift executable consumes a static FFI archive
  and must provide them at its final link. The hosted arm64 Debug and Release
  builds passed in run `35429930923`.
- Contract drift correction: `AGENTS.md` referenced absent `docs/core/*`
  files. It now points to the actual frozen 0.1 contracts under `docs/0.1/`,
  `docs/formats/` and `docs/library/`; no historical documents were added to
  the public tree.
- RC packaging correction: the first `v0.1.0-rc.1` release run
  (`35430364917`) reached the macOS packaging step but stopped during its
  host preflight. The packaging script now accepts either supported macOS host
  architecture, installs the explicit Rust arm64 target when cross-building,
  and passes an arm64 macOS 13 triple to SwiftPM. The distributable remains
  arm64 and the change is release plumbing only.

## 20. Provenance

The repository-content and dependency provenance review is recorded in
[PROVENANCE_AUDIT.md](PROVENANCE_AUDIT.md). It found no bundled Calibre,
Bōkō, Kindle Previewer or other converter source/byte payload. Their use is
limited to explicit external comparison tooling and documented behavior
references.
