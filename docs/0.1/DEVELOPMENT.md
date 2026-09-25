# FolioForge 0.1 Development Contract

This document describes the code that is released as FolioForge 0.1.0. It is
the first document to read before changing Core, Semantic IR, an importer, a
compatibility rule, an exporter, FFI, batch execution, Service, SwiftUI, or
Library.

## Source of truth

- Current architecture: this document.
- Core and client contracts: [API.md](API.md).
- Format support and evidence boundary: [FORMAT_SUPPORT.md](FORMAT_SUPPORT.md)
  and the [format contracts](../formats/).
- Final release evidence: [AUDIT.md](AUDIT.md).
- Known unsupported or unexercised behavior: [KNOWN_LIMITS.md](KNOWN_LIMITS.md).
- Library data contract: [../library/DATA_MODEL.md](../library/DATA_MODEL.md).

Historical stage documents are not authoritative and are not kept in the
public working tree.

## 1. Frozen 0.1 scope

The 0.1 code is feature-frozen. Maintenance may correct documentation/code
drift, release blockers, build failures, test failures, security defects and
unambiguous dead code. It must not add a format, a second conversion pipeline,
Reader behavior, Library product features, cloud synchronization or unrelated
large-scale performance work.

The historical `0.1.0` release used product version `0.1.0`; this contract
does not constrain later product versions. Current product releases keep Core
and desktop version numbers synchronized, while Core ABI, inspection, and
Library schema versions remain independent protocol versions.

## 2. Architecture

The pipeline is:

```text
InputSource
  -> format detection and adapter import
  -> Folio Semantic IR
  -> validation and normalization
  -> compatibility planning / EditPlan
  -> target lowering and writer
  -> binary validation
  -> semantic round-trip check
  -> atomic OutputSink commit
```

The Rust workspace owns all format and semantic decisions. `folio-core` is
GUI-independent orchestration. `folio-model` defines the IR; adapters are in
their format crates; `folio-compat` declares target degradation; writers are
format-local. `folio-batch`, `folio-service`, `folio-ffi` and the SwiftUI app
consume these contracts rather than reimplementing them.

## 3. Dependency boundaries

- `folio-model`: metadata, documents, nodes, resources, styles, navigation,
  anchors, diagnostics and presentation intent. It has no format dependency.
- `folio-format`: shared adapter/import/export contracts only.
- `folio-input`: safe file/directory sources, path normalization, limits and
  detection. It does not interpret format semantics.
- Format crates: `folio-epub`, `folio-kf7`, `folio-kf8`, `folio-kfx`,
  `folio-mobi`, `folio-text`, `folio-markdown`, `folio-html`, `folio-fb2` and
  `folio-docx` own their parsers and writers.
- `folio-compat`: target capability profiles and explicit degradation plans;
  it does not parse or write files.
- `folio-core`: the single registry and conversion orchestrator.
- `folio-library`: optional SQLite/index data only. It depends on Core's
  public inspection contract but Core never depends on Library.
- `folio-online`: opt-in metadata boundary. Online data is never a conversion
  authority and a book file is never uploaded.
- FFI, Service and SwiftUI are transport/presentation adapters. They do not
  become a second capability registry or metadata authority.

The public architecture boundary is enforced by the Rust workspace layout and
is checked locally by maintainer-only Rust regressions and a static
architecture script under the ignored repository-root `tests/` directory.
Neither is a runtime dependency or a public release input.

## 4. Semantic IR

The IR represents book meaning and presentation intent, not a copy of any
format's private field IDs. A `Book` contains metadata, ordered documents,
nodes, resources, font faces, computed styles, navigation, anchors and
diagnostics. Nodes carry stable IDs, semantic role, source location, style
reference and child order. Resource identities are separate from their
placement occurrences.

Important semantic relationships include:

- document and reading order;
- headings and navigation targets;
- anchors, links, footnote/endnote relations and backlinks;
- images, SVG and alternative text;
- ruby, MathML and accessible text;
- tables, lists, poetry, asides and page breaks;
- fixed/reflowable layout intent, writing direction and computed styles;
- conditional/presentation variants when the source proves their relation.

An importer may emit `Unknown`, unresolved diagnostics or input loss. It must
not invent a relationship from a matching string, a navigation ID or a
resource hash alone.

## 5. Conversion pipeline

`FormatRegistry` is the only registry used by Core, CLI, batch, Service and
FFI. `ConversionRequest` carries input path, output path, target,
`ConversionOptions` and an optional non-destructive `BookEditPlan`.

The default options are deterministic output, `Compatible` degradation, no
PalmDOC compression and no per-stage metrics. The compatibility modes are:

- `Strict`: fail when a required feature cannot be represented safely.
- `Compatible`: use declared target fallbacks and report each loss.
- `Readable`: allow the broadest user-visible approximation while retaining
  diagnostics.

Every successful conversion validates the generated target before the atomic
write. Reports include source/target format, metadata, feature counts,
resource summary, warnings, degradation report, compatibility report, output
validation and round-trip status. Real Amazon KFX input may carry a warning
when inferred semantic fragments do not satisfy the normal round-trip gate;
that warning is not hidden.

## 6. Format implementations

The current adapter list is EPUB, plain text, Markdown, HTML, HTMLZ, FB2,
DOCX, KF7, KF8, KFX and the explicit FolioForge KFX compatibility container.
The detailed matrix is in [FORMAT_SUPPORT.md](FORMAT_SUPPORT.md); format-local
contracts are in `docs/formats/`.

KFX has two distinct identities:

1. Real Amazon KFX input is parsed through the native container/Ion path when
   it is DRM-free and reflowable. Unproven structure remains unresolved.
2. FolioForge's writer emits an explicit internal compatibility container for
   deterministic writer/round-trip tests. It is not claimed to be an Amazon
   publishing KFX writer.

## 7. Compatibility and degradation

`folio-capabilities` is the single target feature matrix. Levels are `Native`,
`Compatible`, `Approximate`, `Flattenable` and `Unsupported`. The planner
selects a target-safe fallback and emits a diagnostic; writers do not silently
discard semantic nodes.

EPUB3 is the loss-minimizing semantic target. KF7, KF8 and KFX have distinct
profiles even when a feature name is shared. MathML, vertical writing,
fixed-layout, complex tables, floats, fonts, ruby, notes and links must use
the target profile rather than a format-name heuristic.

## 8. Metadata and editing

`BookEditPlan` is a non-destructive overlay. It can edit metadata, cover
choice/upload, typography, fonts, selected style declarations, document order,
titles and navigation labels. The plan is applied before compatibility
planning, so the report describes the actual output request. It is not stored
as a hidden second IR and does not mutate the source file.

## 9. Resources and output sinks

Resources are loaded through the IR resource loader and are emitted only when
referenced by the accepted semantic book. The Core atomic sink writes a
temporary sibling and commits only after validation. Service work files use an
isolated bounded directory; FFI callers receive owned Rust allocations and
must release them with the matching free function.

## 10. Batch and parallel conversion

`folio-batch` shares the same Core request and report types. It discovers safe
relative paths, applies collision policy, emits item progress, supports
best-effort or strict batch behavior and keeps per-item output provenance.
Parallelism is bounded by the batch engine; a UI or HTTP caller must not spawn
an unbounded conversion pool.

## 11. Library data foundation

`folio-library` is optional and conversion remains usable without a database.
The current schema stores books, people, series, tags, identifiers, storage
roots, file variants, inspected metadata and conversion records. Search is a
rebuildable SQLite FTS5 table. File identity uses relative path, size, mtime
and optional content hash; a failed database transaction does not report a
successful file synchronization.

The Library does not store Semantic IR trees, full inspection JSON or large
resource BLOBs. Its current data contract is
[DATA_MODEL.md](../library/DATA_MODEL.md).

## 12. FFI, Service and SwiftUI

The C ABI exposes product version, ABI version, capabilities, conversion,
analysis, preview, inspection, validation, batch discovery/conversion,
progress callbacks, cancellation and explicit online metadata helpers. Rust
owns returned memory.

The Service provides local health, capabilities, preflight, conversion,
progress/events, download, preview and explicitly disabled-by-default online
metadata endpoints. Request limits and safe archive paths are enforced before
conversion.

The SwiftUI target is a macOS 27 Swift Package for the 0.1 release. It
displays queue, editor, compatibility preview, diagnostics, capability and KFX
inspection data. It calls FFI and never parses an ebook itself. Its checked-in
logo resource is the same FolioForge logo used by the package script and
Service endpoint.

## 13. Validation contract

The public validation workflow runs formatting, production-target Clippy and
release builds; it intentionally does not execute or publish regression tests.
Local Rust unit/integration tests and synthetic fixtures live under the
ignored repository-root `tests/` directory. Extended degradation goldens,
semantic-equivalence fixtures, KFX semantic probes, Service smoke tests and
Calibre/Bōkō/KP3 comparisons remain local-only under `tests/` and
`.folioforge-dev/`.

Release validation must use a fresh external artifact root selected by
`FOLIOFORGE_VALIDATION_ROOT`; the repository must contain no generated output.
The final results and exact commands are recorded in [AUDIT.md](AUDIT.md).

## 14. Performance and reproducibility

Determinism is the default. Optional stage metrics are stable JSON keys and
are enabled with `--metrics` or the corresponding API option. Benchmarks use
anonymous public seeds and write generated bytes outside the repository. A
benchmark is evidence, not a compatibility authority, and a cache miss must
never make a clean build fail.

## 15. Change acceptance

Before an accepted maintenance change:

1. update the relevant current contract;
2. run the local architecture gate and targeted private tests;
3. run format, production-target Clippy, release builds and the applicable
   local A/B or round-trip checks;
4. update the final audit if release behavior changed;
5. leave no local paths, private data or generated artifacts in the public tree.

No stage document, external converter or visual comparison may override the
source relationship and target capability evidence described here.
