# Comic Controlled A/B Protocol

KCC is a behavior reference. Its bytes are not output truth. FolioForge and
KCC must receive equivalent source facts and comparable output settings; the
comparison measures structure, visual result and performance, not byte
identity.

## Baseline identity

Every accepted A/B run records:

- FolioForge commit, package version, build profile and processing algorithm
  versions;
- KCC release/version and executable SHA-256;
- OS, architecture, CPU/memory class and relevant runtime/codec versions;
- source fixture manifest and SHA-256 for each public synthetic input;
- exact FolioForge/KCC arguments, device profile and output format;
- warm/cold run policy, repetitions, elapsed time and peak RSS measurement
  method;
- generated output hashes for reproducibility, while not requiring equal
  FolioForge/KCC hashes.

Pin the KCC version for a baseline. Do not compare against an unrecorded
"latest" binary.

## Controlled fixtures

Build non-private fixtures that isolate one behavior at a time: numbered
pages (`1, 2, 10`, zero-padded variants and mixed case), Unicode/nested names,
LTR/RTL order, cover + portrait + landscape spreads, split/rotate/both,
bordered and borderless art, page numbers/margins, two/four panels, tall
Webtoon pages with known split anchors, grayscale/color/screentones, EXIF
orientation, animated/malformed image inputs, transparent PNG, PDF raster
pages, archive traversal/duplicate names and resource-limit boundaries.

Keep a separate representative performance set: small manga, 300-page manga,
large color pages, PDF, Webtoon and spread-heavy sources. Private books may be
used locally only with their manifest and output outside the repository.

## Comparison metrics

For each page/book compare:

- accepted/rejected input and page count;
- page identity/order, reading direction, cover, chapter boundaries and
  volume split;
- spread decision, split order/rotation, page side, panels and Webtoon slice
  boundaries;
- crop rectangles and retained edge/page-number areas;
- output pixel dimensions, color mode, encoding, file size and diagnostics;
- image similarity using a documented crop metric, SSIM/perceptual hash,
  histogram/black-level and edge-retention metrics as appropriate;
- wall time, CPU time, pages/sec, decoded pixels/sec, peak RSS and output size.

Image hashes and visual metrics are supplementary: every intentional
difference is explained by source evidence, target capability or a documented
algorithm choice. Human visual review is secondary to the machine-readable
record, never a substitute for it.

## Execution and privacy

Run KCC only as an external comparison tool. FolioForge runtime, CI and clean
builds must not depend on KCC, Python modules, KindleGen, 7-Zip or KCC output.
Store input/output artifacts, metrics, screenshots and private book names in
the ignored maintainer bundle under `.folioforge-dev/` and the external
`FOLIOFORGE_VALIDATION_ROOT`. Commit only sanitized aggregate baselines and
reproducible fixture definitions that contain no private book data.

Every major change listed in the 0.2 plan has an independent controlled A/B
before acceptance. Any observed algorithm difference or known limitation is
recorded in the public feature contract and private detailed run log.
