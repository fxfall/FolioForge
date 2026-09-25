# FolioForge Reader Runtime — 0.2.1 Contract

| Field | Value |
| --- | --- |
| Status | R0–R12 complete and accepted (2026-09-25) |
| Target product version | `0.2.1` |
| Baseline | 0.2 Comic Core work; frozen 0.1 behavior is unchanged |

This contract adds a stateless display/runtime domain to the 0.2 architecture.
It supersedes the original Comic Core plan's exclusion of Reader only for the
0.2.1 Reader work described here. It does not add Library, persistent reading
progress, bookmarks, highlights, annotations, cloud sync, or account state.

## Architecture and ownership

```text
source importer / Comic composition
                 ↓
             Folio IR
             ↙      ↘
       exporters     folio-reader
          ↓               ↓
        output      generic render model
                          ↓
                       FFI / GUI
```

`folio-reader` consumes generic Folio IR. It is format-blind and must not
depend on `folio-comic`, importers, exporters, `folio-compat`, Core, FFI,
Service, Library, SQLite, AppKit, or SwiftUI. It must not re-run Comic
analysis/edit/composition or implement target conversion. Exporters do not
depend on Reader.

Reader owns transient session state, generic reading-order/navigation,
resource resolution through the existing generic resource contract, viewport
and fixed-page geometry, and display render models. Comic owns source
identities/analysis/edit/composition and target render planning/transforms.
SwiftUI displays Reader results; it does not resolve source paths, traverse
book order, or calculate its own page fit geometry.

Target preview remains a distinct projection: compatibility planning and
Comic target render plans produce generic projected state; Reader may display
that state but does not decide target compatibility or execute conversion.

## Session and location

`ReaderSession` is transient and holds one immutable generic `Book` plus
session-local location, viewport and derived caches. Dropping a session drops
its local state; no database or persistent reading history is introduced.
Folio IR itself remains immutable.

`ReaderLocation` is source-format independent. `DocumentId` is the fixed-page
identity for the current IR representation; optional `NodeId`, text offset and
semantic context provide a reflowable foundation. EPUB CFI, KFX PID, DOM
XPath and filesystem paths are not Reader identity. Source positions may be
kept as auxiliary provenance only.

## Layout and display behavior

Reader consumes `LayoutMode::Reflowable` or `LayoutMode::Fixed`. The initial
functional priority is Fixed Layout single page: page identity, validated
viewport/placement, Fit/Fill, reading direction and next/previous. Spread
composition begins only after single-page behavior is stable and consumes
generic direction, page-side and spread hints; it must not redetect Comic
semantics.

Reader produces stable render models rather than exposing raw IR to SwiftUI.
The GUI performs only display/gesture plumbing. Zoom and pan are transient
presentation state, never Semantic IR.

### Current fixed-page support boundary

R3 accepts only `LayoutMode::Fixed` documents whose entire root is one
childless `NodeKind::Image`, with no computed style declarations or
presentation variants. It resolves that exact `ResourceId` through
`Book::load_resource`, reads raster dimensions from the encoded header without
decoding the full pixel buffer, and returns an encoded `ResourceView` with a
single viewport placement. Fit, Fill and Actual Size geometry, clipping, zoom
scale and pan are computed once in Reader. Unsupported or ambiguous page trees
fail with `UnsupportedLayout`; malformed image data and unsafe dimensions use
typed errors.

The current Comic identity projection has no source-declared page viewport.
For the proven one-image page shape, Reader uses the image's intrinsic
dimensions as a transient display coordinate space and includes an explicit
`IntrinsicImageSizeUsedAsPageViewport` diagnostic. This is not a claim that
the original source declared that viewport and is not sufficient evidence for
fixed-layout export fidelity. Private Comic integration coverage follows two
real Fixed Layout EPUBs through Semantic IR, deterministic CBZ, Core Comic
session and Reader; it checks first/middle/last page byte identity, document
index and prior Core preview aspect. This proves the exercised image path, not
source viewport/spread semantics. Navigation uses document identity and the generic presentation direction
(`rtl`/`right-to-left`); an explicit Reader direction can override it. Missing
direction defaults to LTR. Source document storage order is never rewritten.

## Resource and error rules

There is one authoritative Reader resource path. Reuse `Book::load_resource`
and its `ResourceLoader`; do not duplicate path lookup or introduce a second
resource store. Resolve resources lazily, keep decoded caches bounded and
session-scoped, release them when the session closes, and never decode every
page on open. Cache content is derived and never semantic authority.

Reader failures use typed stable codes; GUI behavior must not parse display
strings. Invalid viewport/location, missing resources, unsupported layout,
bad page geometry, decode failures, cancellation and closed-session requests
must fail safely without user-input panics.

## Migration gates

At the start of migration, `folio-preview` was retained as Preview A until
equivalent Reader B behavior was proven. R11/R12 completed that gate: the old
renderer crate is removed, while four captured Preview A JSON responses remain
only as ignored local golden fixtures for regression tests. Comic-image
display is checked against source bytes, dimensions, generic IR placement and
reading order. Text, navigation, resource identity, diagnostics and
reflowable behavior were compared separately.

Gate order:

1. R0 Preview audit/A baseline — complete.
2. R1 Reader boundary/session/location/viewport/error skeleton — complete.
3. R2 resource resolution and R3 strict fixed single-page model/navigation —
   complete after targeted and full private regression gates.
4. R4 Core-owned FFI Reader lifecycle, fixed-page/resource transport and
   reuse of an existing Comic IR session — complete after private regressions.
5. R5 SwiftUI Comic display from Reader output — complete for the strict
   single-image page subset, with a typed-boundary Core-preview fallback.
6. R6 reflowable display migration/A-B — complete.
7. R7 generic navigation — complete; R8 generic spread mode — complete.
8. R9 Reader preview API; R10 measured performance/cache; R11 Reader GUI
   migration; R12 remove duplicate rendering implementation — all complete.

### R4 Core and FFI boundary

Core owns the Reader session registry and imports an input to generic Folio IR
before opening a `ReaderSession`. The generic open path can fall back to the
Comic importer for a local image folder or ZIP/CBZ; Reader itself remains
format-blind. An existing Comic Core session can create a Reader session from
its already-projected IR through `folio_reader_open_comic`, avoiding a second
source import and keeping page/resource lookup out of Swift. The two session
lifetimes are independent.

FFI exposes open, close, summary, current fixed-page model, resource resolution,
navigation, viewport and direction operations as owned JSON results with
stable machine-readable error codes. Encoded resources are transported as
base64 with a 64 MiB Core transport bound; the underlying Reader resource
resolver retains its separate bounded resource-loading contract. FFI calls do
not expose Rust references, IR trees or source-format internals to clients.

### R5 SwiftUI Comic display

For supported fixed single-image pages, SwiftUI now draws only the Reader FFI
placement/resource result. The page grid continues to use Core thumbnails;
selection is mapped through Core's display index to the immutable IR document
index, while Reader owns next/previous order, LTR/RTL, Fit/Fill/Actual Size,
zoom/pan and page placement geometry. Unsupported page shapes and the FFI
resource-transport ceiling explicitly fall back to the existing Core preview;
navigation remains available if the Reader session itself is open. The Comic
conversion route and 0.1 Books compatibility-preview route are unchanged.

Reader remains a partial fixed-page renderer. This gate does not migrate
reflowable previews, switch the general preview authority, add spread
composition, or authorize deleting `folio-preview`.

### R6 Reflowable display migration

Core now applies edits and `folio-compat` target planning/projection, then
passes only the resulting generic IR to `folio-reader` for per-document
reflowable render models. Core composes the existing target profile, loss
report and outward compatibility-preview DTO. Neither Reader nor Swift gains
target-compatibility authority. `folio-preview` remains a byte-for-byte Preview
A oracle for later A/B/review and is no longer a Core runtime dependency.

The private A/B compares every serialized DTO field and complete HTML for
simple/rich EPUB, Markdown and standalone HTML fixtures; all four are
byte-identical. This confirms the migrated renderer preserves current
reflowable preview output. It does not yet move the FFI symbol/client-facing
operation to a Reader-specific API, change preview semantics, add generic
or delete Preview A; FFI/API authority and cleanup gates remain R9/R11/R12.

### R7 Generic navigation

Reader projects TOC, landmarks and page-list entries from immutable IR into
source-format-free navigation references. A reference identifies a nested
entry by collection plus child-index path; its resolved location is included
only when an exact document and, when present, a unique anchor are proven.
External, empty, missing, duplicate and ambiguous targets stay visible with a
typed unresolved reason. Direct page navigation uses `DocumentId`. No label,
filename similarity or nearby-page fallback is used. The EPUB navigation A/B
keeps prior HTML internal links byte-identical while Reader navigates to the
same imported document/anchor identity. Core/FFI expose these operations; the
stable C header declares the full Reader function set.

### R8 Generic spread mode

R8 records AD-0.2.1-007 before changing the model: generic IR did not carry
page side or spread intent, and the EPUB importer discarded the corresponding
spine declarations. The additive IR contract uses optional per-document
`PageSide` and `SpreadHint` metadata; empty metadata is omitted from
serialization so books without declarations retain their prior serialized
shape. EPUB maps only recognized `page-progression-direction` (`ltr`/`rtl`)
and spine `page-spread-left`, `page-spread-right`, and
`rendition:page-spread-center` properties. Equivalent aliases are accepted;
conflicting or unsupported values remain unassigned and diagnostic.

Reader stays single-page by default. Synthetic spreads are a transient,
user-selected view: the first page stays alone, centered/single-page hints
split a group, explicit complementary sides determine physical placement, and
unhinted pairing follows only generic Reader direction. No image-size,
filename, source-format, or Comic-specific inference is performed. Reader
navigation moves between spread groups while direct document selection and
stored reading order remain page-granular. Comic/folder sources without
explicit hints are paired only after the caller opts in.

R8 verification: private Reader geometry fixtures prove cover isolation,
two-slot placement, centered singleton behavior, explicit side placement,
LTR/RTL ordering, page-granular direct selection and spread-group
next/previous. EPUB fixtures prove recognized declarations map to the expected
IR document and conflicts stay unassigned with diagnostics. The complete
private Rust regression/Clippy runner, format checks, architecture boundary,
C-header syntax, `git diff --check`, and macOS 27 arm64 FFI Release plus
SwiftUI Debug/Release builds passed. Existing local-corpus-only tests remained
ignored by their documented environment gates. A formatting check initially
caught unformatted assertions in the new EPUB test; `rustfmt` corrected the
test, after which the full runner passed. Build and scratch outputs stayed
outside the checkout.

### R9 Reader Preview API

Core now exposes a Reader Preview request/result contract with three explicit
modes: source Semantic IR, edited Semantic IR, and a Core-prepared target
projection. Core alone imports inputs, applies the existing edit plan and
compatibility/degradation projection, and supplies only the resulting generic
Book to Reader. A target request requires target context; semantic requests
reject target context. Reader chooses reflowable or fixed-page presentation
from generic IR layout and returns stable render content; it does not choose a
target profile or perform conversion. Reflowable location is validated as
navigation context while the current preview remains a document bundle.

The new FFI entry point is `folio_reader_preview`; it returns the tagged
render-content DTO and typed `invalid_preview_request` errors. Fixed-page
resource transport uses the existing 64 MiB bound and base64 representation.
At the R9 checkpoint the previous Books Preview endpoint remained available;
R11 later migrated SwiftUI to `folio_reader_preview`, and R12 audited the
frozen 0.1 public contract.

R9 tests prove all three modes, required target context, edited metadata,
fixed-page resource identity, and exact equality of target-preview document
DTOs and complete HTML against Preview A. The full private Rust regression and
Clippy runner, formatting, architecture boundary, C-header syntax and
`git diff --check` passed. Host arm64 macOS 27 FFI Release and SwiftUI
Debug/Release builds passed; existing unrelated Swift deprecation and
Command Line Tools search-path warnings remain. Build and scratch outputs
stayed outside the checkout. The first API compile exposed a mismatched
degradation-plan type and a missing private-test JSON import; both were
corrected and the targeted plus full suites passed. A first FFI build was
stopped immediately after it created a repository-local Cargo target because
external build variables had been omitted; that newly created directory was
moved intact to the external validation area, and the correctly isolated
Release/Swift builds then passed. R9 is accepted; performance profiling is the
next gate.

### R10 Measured performance and bounded display-image reuse

Before changing caches, the Reader path was profiled using two local manga
EPUBs after the existing lossless spine-image-to-CBZ extraction and
Comic-Core-to-IR projection. Both CBZ imports preserved all 241/241 and
234/234 page bytes and order. This normalized image-page path was used because
the original fixed-layout EPUB pages contain richer XHTML trees than the
current strict single-root-image Reader support subset; a first direct-EPUB
timing attempt correctly failed with `unsupported_layout` and was not treated
as evidence of Reader performance.

On the optimized arm64 macOS Release test binary, the 475-page sequential
Reader model pass completed in about 0.36 s with process peak RSS around
10.2–10.7 MiB. First-page model latency was 0.64–1.21 ms across the two runs;
navigation-plus-page-model median was 0.66–0.69 ms and P95 0.78–1.15 ms.
Repeated current-page model latency was 0.62–0.93 ms. Across each profile,
page counts and returned encoded image bytes were identical: 162,080,409 bytes
for the 241-page sample and 154,768,502 bytes for the 234-page sample. A direct
code-path audit records the actual boundary: archive loaders return encoded
byte vectors, Reader publishes shared encoded resource views, FFI serializes
base64 JSON, and Swift decodes to `Data`; the benchmark's byte totals are
returned-resource volume, not a false exact count of internal memory copies.
It excludes FFI/base64 and AppKit display costs.

The only demonstrated repeat-display waste was SwiftUI recreating AppKit image
objects from the same Reader resource on view recomposition. Added a
session/resource-keyed `NSCache` for the original `NSImage` and its retained
`CGImage` representation, with a four-entry limit and 128 MiB estimated raster
cost limit. Images whose estimated raster exceeds the budget still render but
are not cached. The cache is cleared when its Reader session closes; preview
sessions use a request-scoped key. The original image representation is
returned unchanged, so no image conversion, orientation rewrite or source-byte
change is introduced. No decoded image is created during ReaderSession open,
and no entire comic is preloaded.

A private 12-iteration AppKit probe over both first-page images measured cache
misses at about 47–49 μs and cache hits at about 0.25 μs; each sample's
estimated raster was about 13.1 MB. It also verifies object reuse by identical
session/resource key, separation across resource IDs and sessions, and
eviction on session teardown. This measures image-object retrieval/CGImage
representation access, not full interactive display frame time. The direct
Reader release profile before/after had identical page/resource volume and
sub-1% median navigation variation; no Rust resource cache, geometry cache,
thumbnail cache change or prefetch was justified by those measurements.
Reader's encoded resources remain bounded by the existing 64 MiB per-resource
limit and the GUI cache's aggregate 128 MiB budget.

R10 verification: the private real-sample benchmark ran in arm64 Release
before and after the GUI cache; the AppKit cache/identity probe passed;
macOS 27 SwiftUI Debug/Release builds, formatting, architecture boundary and
diff checks passed. A first direct Reader benchmark used raw EPUB IR and failed
because its XHTML page tree is outside R3's proven single-image shape; the
corrected run used the already validated EPUB-image→CBZ→Comic IR route. The
first `/usr/bin/time` around the CBZ generator included its initial dependency
compile and is not used as a runtime measurement. An initial Swift benchmark
compile caught a main-actor call at the script entry point; it was corrected
with `MainActor.assumeIsolated` and rerun. All source, outputs and private
metrics stayed in ignored project-local or external validation locations. R10
was accepted before the subsequent R11/R12 gates below.

### R11 Reader GUI authority migration

The SwiftUI compatibility-preview pane and inspector now call the Reader
preview bridge and decode Reader-owned reflowable/fixed-page render content.
Reader-provided fixed-page placements and the existing bounded image cache
drive rendering; SwiftUI does not resolve IR resources or calculate page fit.
Core's target compatibility profile, degradation plan, import and edits remain
authoritative. Fixed-page Reader transport failures continue to use the
explicit Core preview fallback where the Comic GUI contract requires it.

The direct Reader target response and frozen old Core response were compared
against four private Preview A goldens (simple EPUB, rich EPUB, Markdown and
HTML): shared DTO fields and complete HTML bytes match. The old `folio-preview`
crate was not needed to generate or execute the post-migration side. A direct
FFI Reader preview test covers the serialized request/response contract. The
macOS 27 arm64 SwiftPM Debug and Release builds passed. An initial FFI test
sent lower-case `compatible`; the wire enum is case-sensitive and uses
`Compatible`. Correcting the test fixture, rather than loosening production
deserialization, made all six Reader FFI tests pass.

### R12 duplicate implementation removal and compatibility audit

Removed `crates/folio-preview` from the workspace, removed its private test
and integration dependency, and deleted the obsolete standalone
`render_reflowable_preview` path and unused Swift preview DTOs. Reader's
`ReflowableDocumentRenderModel` is the canonical model; Core's frozen 0.1
`PreviewDocument` name is a type alias rather than a second struct or mapping.
`folio-preview` mentions in historical migration notes describe the former
R0–R10 state, not a remaining crate or runtime dependency.

Architecture Drift `AD-0.2.1-011` records that deleting Core `preview`, the
`folio_preview` C symbol or Service `/preview` would break the frozen 0.1
public contract. Resolution: keep only those outward operations as thin
compatibility adapters to the single Reader renderer; their response shape and
behavior are protected by the frozen local Preview A fixtures. No old
renderer, parallel HTML implementation or GUI DTO remains. This is an
internal implementation migration and does not revise the 0.1 API contract.
Decision `D-0.2.1-012` keeps Preview A only as private golden data.

The final source audit also replaced repeated from-zero synthetic-spread scans
with session-local page-to-group and group-range indexes, rebuilt only when
spread mode or direction changes. A 501-page regression checks group traversal,
previous navigation and index rebuilding after changing to RTL. The new
derived indexes remain transient session state and do not mutate Folio IR.

Private tests and snapshots remain local under ignored root `tests/`,
`.folioforge-dev/`, or external `FOLIOFORGE_VALIDATION_ROOT`; they are not
included in public GitHub artifacts. The public workspace/package version and
0.1 packaging metadata are not changed by the R0/R1 skeleton.
