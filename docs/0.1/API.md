# FolioForge 0.1 Public API

Product version: `0.1.0`.

## CLI

The binary is `folio` (`folio-cli`). Public subcommands are:

- `capabilities` — emit the single format/target/capability registry.
- `analyze INPUT --to TARGET [--mode strict|compatible|readable]` — return
  input diagnostics and the compatibility plan without writing an output.
- `inspect INPUT [--semantic|--ion|...]` — inspect the normalized Semantic IR
  or explicitly requested KFX diagnostics.
- `validate INPUT` — validate a source or generated artifact.
- `convert INPUT --to TARGET [--output PATH]` — import, plan, lower, validate,
  round-trip check and atomically write one artifact.

Targets are `epub`, `kf7`/`mobi`, `kf8`/`azw3`, `kf7kf8combo`/`combo` and
`kfx`. Output extensions are respectively `.epub`, `.mobi`, `.azw3`, `.mobi`
and `.kfx`.

Conversion options include deterministic/non-deterministic IDs, `none` or
`palmdoc` compression where applicable, metrics, compatibility mode, target
fallback options, text encoding/paragraph options and a serialized
`BookEditPlan`.

## Rust Core

`folio-core` is the client-neutral orchestration API. The stable contract
types are:

- `Target`, `FormatRegistry`, `ConversionRequest`, `ConversionOptions`;
- `ConversionReport`, `InputReport`, `SemanticReport`,
  `CompatibilityReport`, `OutputReport`, `RoundTripReport`;
- `ProgressStage`, `ProgressEvent`, `ConversionMetrics`,
  `CancellationToken`;
- `preflight`, `analyze`, `preview`, `inspect`, `validate`, `convert` and
  `convert_with_progress`.

`CORE_VERSION` is the product package version (`0.1.0`). `ABI_VERSION` is an
independent integer protocol version (`1`).

## Semantic model

`folio-model` is the format-neutral contract for `Book`, `Document`, `Node`,
`Resource`, `FontFace`, `ComputedStyle`, `Navigation`, `AnchorGraph`,
`PresentationIntent`, metadata and diagnostics. IDs and source locations are
opaque semantic evidence; no caller may infer source meaning from a KFX field
number or output filename.

## C FFI

The static/shared library is `folio-ffi`. JSON is UTF-8 and path values are
local paths supplied by the host. Exported entry points are:

```text
folio_version
folio_capabilities
folio_convert
folio_convert_with_cancellation
folio_convert_with_progress
folio_analyze
folio_preview
folio_inspect
folio_validate
folio_batch_list_directory
folio_batch_convert_with_progress
folio_online_metadata_search
folio_online_metadata_merge_plan
folio_cancellation_new
folio_cancellation_cancel
folio_cancellation_free
folio_result_free
folio_string_free
```

Every operation that returns `FolioResult*` returns `code == 0` for success
and `code == 1` with `{"error": ...}` for an error. Rust allocates the result
and JSON string; the host must call the matching free function exactly once.
Progress callback JSON is borrowed until the callback returns and must be
copied by the host. Cancellation is cooperative.

## Local Service

`folio-service` is the local HTTP adapter. The health and capabilities
responses expose product/Core versions and current registered formats. The
main routes are:

```text
GET  /health
GET  /
GET  /capabilities
POST /preflight
POST /convert
GET  /jobs/{id}
GET  /jobs/{id}/events
GET  /jobs/{id}/download
POST /preview
GET  /online/status
POST /online/metadata/search
POST /online/metadata/merge-plan
GET  /logo.png
```

Uploads are bounded and path-normalized. Online metadata is disabled unless
explicitly enabled in service configuration; it receives search terms only.

## Library

`folio-library` is optional. Its public types cover `LibraryDb`, `Library`,
`ScanOptions`, `ScanReport`, `BookRecord`, `FileVariant`, `BookPage`,
`SearchResult`, `ConversionRecord`, `QueryBudget` and metadata diff/sync
operations. Schema version and inspection contract version are independent of
product version; see [../library/DATA_MODEL.md](../library/DATA_MODEL.md).
