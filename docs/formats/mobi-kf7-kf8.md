# MOBI / KF7 / KF8

`folio-mobi` owns Palm/MOBI record handling and the explicit KF7+KF8 combo
container. `folio-kf7` and `folio-kf8` own their target lowering and resource
encoding. Detection distinguishes KF7 from KF8 using validated record
metadata, not an extension alone.

- KF7/MOBI preserves readable text and headings, compatible navigation,
  images, links and page breaks. SVG, fonts, ruby, math, notes and complex or
  positioned layout may be approximated or flattened according to the plan.
- KF8/AZW3 carries richer HTML/CSS, images, fonts, SVG, links and common
  layout. Unsupported CSS or semantic relationships remain diagnostics.
- KF7+KF8 is FolioForge's explicit composite compatibility container; it does
  not claim to reproduce an undocumented Amazon dual-format boundary.

The target writer can use no compression or PalmDOC compression where the
request allows it. Generated artifacts are validated before atomic commit.
