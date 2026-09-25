# KCC Reference Boundary

Reference project: [ciromattia/kcc](https://github.com/ciromattia/kcc),
Kindle Comic Converter. Its repository README and CLI help are the primary
behavior references. Snapshot checked 2026-09-24; pin a concrete release and
binary hash in every A/B baseline before using results.

KCC documents comic optimization for e-readers, fullscreen/fixed-layout
outputs, device profiles and image-size reduction. The currently published
README lists folder images (JPG/PNG/GIF/WebP), CBZ/ZIP, CBR/RAR, CB7/7Z and
PDF input; MOBI/AZW3, EPUB/KEPUB, CBZ, PDF and image-folder output. It describes
spread split/rotate modes, manga direction, panels, Webtoon processing,
automatic crop, image tone/color, target resizing, cover options and output
size splitting. Its CLI help is the concrete reference for option behavior.
See the [upstream README](https://github.com/ciromattia/kcc#readme) and
[source tree](https://github.com/ciromattia/kcc/tree/master/kindlecomicconverter).

## How FolioForge uses the reference

- Compare observable page order, crop, spreads, panels, output dimensions,
  images, output size and runtime through
  [COMIC_AB_PROTOCOL.md](COMIC_AB_PROTOCOL.md).
- Re-express useful behavior in FolioForge's Rust model and shared IR/export
  path. Do not port KCC source structure, Python execution, GUI, temporary
  paths, option names, algorithms line-for-line or external tool assumptions.
- KCC behavior never establishes source semantics when the source is
  ambiguous; uncertainty remains explicit in FolioForge analysis.
- KCC, its packages and generated files are not Core/CI/runtime dependencies.
- Intentional deviations and unsupported features remain visible in
  `COMIC_FEATURE_MATRIX.md` and the run record.

## Source and license boundary

The KCC repository is maintained independently and has its own license and
release history. This project uses its published behavior and controlled
outputs as reference material; it does not copy or embed KCC source, assets,
executables or output bytes. If future work proposes code reuse, stop and
perform a separate license/provenance review before importing anything.
