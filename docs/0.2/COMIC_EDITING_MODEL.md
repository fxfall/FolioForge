# Comic Editing Model

This contract defines source-relative comic edits for FolioForge 0.2. The
model is platform-neutral and serializable; no GUI may invent its own
coordinate or ordering rules.

## Immutable source identity

`ComicNativeBook` is immutable. A `SourcePageId` is stable across reopening,
page reordering and process restarts. It is not a vector index. Its matching
strategy uses a versioned source identity and normalized source member
location, with a stable discriminator where needed. Full image hashing is
not the default. Edit-plan application must report missing, ambiguous or
stale identities instead of silently binding to another page.

`ComicSourcePage` records the source ID, safe source locator, encoded resource
reference, original dimensions, detected format, source name and explicit
source metadata. It does not hold decoded pixels or inferred layout truth.

## Normalized coordinates

All manual crops, panels and Webtoon split anchors use source-relative
normalized coordinates in the closed unit square. This coordinate contract is
versioned and independent of preview resolution and output device size.

For `NormalizedRect { x, y, width, height }`:

- each value is finite;
- `x` and `y` are at least 0;
- width and height are greater than 0;
- `x + width <= 1` and `y + height <= 1`;
- rectangles are never silently clamped or reordered.

Invalid edits return a structured validation error. Pixel coordinates are
derived in Core from original dimensions with one documented rounding rule.

## Delta-only ComicEditPlan

`ComicEditPlan` contains only user changes, not a copy of unchanged source
data. It has an explicit schema version and deterministic serialization, and
contains no absolute temporary paths or GUI state.

Its compact semantic groups are:

- Book edits: reading direction, cover source page, explicit composition order
  and volume boundaries.
- Page edits keyed by `SourcePageId`: inclusion, optional order override,
  rotation correction, manual crop, spread and page-side override,
  panel regions, Webtoon split anchors and per-page processing overrides.
- Chapter edits: add/remove or move markers against stable source identities.
- Fusion edits: explicit source/volume order and provenance.
- Revision metadata: schema/revision IDs needed for deterministic history and
  stale-plan diagnosis.

Device resolution, output format, global upscale/grayscale, JPEG quality,
quantization, E-Ink processing and automatic output-size splitting belong to
`ComicOutputOptions`/`ComicRenderPlan`, not this edit plan.

## Commands, validation and history

GUI/CLI callers submit semantic `ComicEditCommand` values; they do not mutate
internal plan fields. The Core applies a command to a plan, validates the
result against the immutable native book, and returns a new plan plus a
structured report. Required command families include remove/restore/move
page, crop set/clear, rotation, spread, cover, direction, panel regions,
Webtoon splits and chapter add/remove.

Undo/redo is a caller-owned sequence of plan revisions. Any Core session
helper remains UI-framework-independent. Applying the same command sequence
to the same source plan yields the same validated plans.

Plan loading validates schema version, source-book identity, page IDs,
coordinate ranges, order uniqueness, chapter/panel relationships and
references. Unknown future schema versions fail explicitly; supported older
versions use documented migrations. No partial application is reported as
success.

## Analysis precedence

`ComicAnalysis` stores suggestions separately from edits. Every suggestion
has evidence, confidence and a stable analysis algorithm version. Explicit
user overrides take precedence without erasing the original suggestion. The
edit report shows manual crops, spread decisions, panels, page removals/order,
cover and Webtoon anchors distinctly from automatic suggestions.

## Composition output

The plan does not mutate source facts. Composition resolves edits in one
deterministic pass and produces effective reading order, included pages,
cover, crops, rotation, spreads/sides, chapters, panels and Webtoon slices.
Target geometry and physical pixel execution happen later in RenderPlan.
