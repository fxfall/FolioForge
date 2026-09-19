# KFX placement-oracle corpus

This directory contains content-free observations from Calibre KFX Input's
`--json-content` output. The input manifest contains anonymous SHA-256 IDs;
book paths, metadata, text, and image bytes are not copied into the repository.
Raw Calibre JSON is temporary and deleted immediately after sanitization.

The pinned oracle environment is Calibre 9.14 and KFX Input 2.34.2. The runner
downloads the KFX Input archive from Calibre's official plugin index, verifies
its SHA-256, and installs it in a temporary Calibre configuration. It does not
alter the user's Calibre preferences or install the plugin persistently.

Run the corpus on a machine containing the matching DRM-free files:

```sh
tools/oracle/kfx/run-calibre.sh /path/to/local/books tests/kfx-placement-oracle/expected
```

Each expected JSON record retains every type-2 image occurrence sorted by KFX
position, using per-book anonymous `img_####` resource aliases. Frequency rows
preserve repeated uses of the same resource; a resource set alone is not an
equivalent oracle. Type-1 text chunks and all other content fields are omitted.

These records are development-only behavior-oracle data. Production FolioForge
code must not read them, depend on Calibre, or use oracle positions to infer
resource identity or placement.

`run-folio-audit.py` captures sanitized traces through exact KFX resource IDs,
native placements, Semantic IR image nodes, and the EPUB resource map. The
Calibre KFX Input JSON occurrence list is ordered by KFX position. FolioForge
reconstructs the corresponding reading sequence from reading-order (`$538` /
`$258`), section (`$260`), and story (`$259`) fragments; a regression fixture
pins that graph traversal independently of physical fragment storage order.

To rerun the audited import/export path after building the CLI, use:

```sh
python3 tools/oracle/kfx/run-folio-audit.py \
  --book-root /path/to/local/books \
  --manifest tests/kfx-placement-oracle/inputs.json \
  --folio target/debug/folio \
  --output-dir tests/kfx-placement-oracle/reports
bash tools/oracle/kfx/run-epub-reference.sh /path/to/local/books \
  tests/kfx-placement-oracle/reports/epub-reference.json
```

The EPUB comparison follows OPF spine order and XHTML/SVG document order. It
requires each image reference to resolve through the package manifest to the
exact audited IR resource, then compares reference multiplicity and sequence
with both IR and oracle. The checked-in summary contains anonymous counts and
parity booleans only; generated EPUBs and raw KFX Input JSON are temporary.

The auxiliary `reports/identity-comparison.json` also examines alternate SID
interpretations and frequency-constrained candidate matching for diagnosis
only. Those candidate counts are never consumed by conversion code and must not
be used to infer identity or placement. `raw_reference` records physical KFX
container traversal; its order is not the reading sequence. The production
native/semantic/IR sequence is derived from the explicit reading-order graph
and is the stage compared to the oracle's position-sorted sequence.
