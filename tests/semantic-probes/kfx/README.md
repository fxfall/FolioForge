# FolioForge KFX Semantic Probe Corpus

This directory is the controlled semantic probe corpus for FolioForge 0.1.
It is intentionally separate from the 82-book regression corpus.

The evidence order is fixed:

1. The fixture source XHTML/EPUB is the statement of authorial semantics.
2. Kindle Previewer 3 and the KFX Output plugin generate the observed KFX.
3. FolioForge records an anonymous structural audit of the original KFX.
4. Calibre KFX Input and Bōkō are compatibility references only. They can
   identify a missing investigation, but they do not define FolioForge
   semantics.
5. A production decoder may be changed only after the source relationship and
   the raw KFX structural evidence agree across the relevant probes.

The source package for each probe is built deterministically by
`tools/dev/kfx_probe.py` from the fixture's `source.xhtml`, `manifest.json`,
and optional `assets/` directory. Generated KPF, KFX, JSON, EPUB, logs, and
reports never belong in this directory; pass an artifact root outside the
repository with `--artifact-root` or `FOLIOFORGE_KFX_PROBE_ROOT`.

## Manifest contract

Every fixture has:

* `manifest.json`: source-level semantics and acceptance boundaries;
* `source.xhtml`: a valid EPUB3 reading document;
* `README.md`: the purpose, positive assertion, and negative control.

`tools/dev/kfx_probe.py` materializes these fields as a `ProbeExpectation`
object. It is the single source-level contract consumed by validation and
build reports; it never contains KFX numeric identifiers. A source-only
validation report is `NotExercised`. After a reproducible KFX is produced it
may become `Encountered`, but it cannot become `Recognized`, `Mapped`, or
`Approximated` until the raw KFX evidence and the semantic decoder review are
recorded.

Manifests must describe relationships such as `noteref -> footnote` or
`MathML -> accessible text`. They must not hard-code KFX field IDs, symbols,
fragment IDs, generated anchors, or Calibre output IDs. Those are observations
written to the external audit artifact.

## Commands

From the repository root:

```text
tools/dev/folio-dev kfx-probe list
tools/dev/folio-dev kfx-probe validate FN-01
tools/dev/folio-dev kfx-probe build FN-01
tools/dev/folio-dev kfx-probe build --all
tools/dev/folio-dev kfx-probe evidence /path/to/kfx-dir \
  --output "$FOLIOFORGE_VALIDATION_ROOT/real-kfx-evidence.json"
tools/dev/folio-dev kfx-probe coverage \
  --artifact-root "$FOLIOFORGE_VALIDATION_ROOT" \
  --output "$FOLIOFORGE_VALIDATION_ROOT/probe-coverage.json"
```

`build` runs source validation, KP3 -> KPF, KPF -> KFX, the FolioForge
content-free audits, Calibre KFX Input EPUB/JSON reference output, and stores
all evidence under the caller-supplied external artifact root. A batch
continues after an individual toolchain failure and writes that Probe's
`result.json` with `status: blocked_toolchain`; the command returns non-zero at
the end if any Probe was blocked.

## Coverage vocabulary

Reports use the 0.1 probe states `Encountered`, `Recognized`, `Mapped`,
`Approximated`, `Unsupported`, `Unknown`, and `NotExercised`. `Unknown` means
that the source or KFX was not understood well enough to classify; it is not a
synonym for `Unsupported`. Unknown visible semantics are not silently dropped.

The source expectations remain in [`semantic-review.json`](semantic-review.json).
Current product support is summarized in
[`docs/0.1/FORMAT_SUPPORT.md`](../../../docs/0.1/FORMAT_SUPPORT.md) and
[`docs/formats/kfx.md`](../../../docs/formats/kfx.md).
