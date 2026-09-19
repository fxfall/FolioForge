# FN-01 — one noteref and one footnote

Positive control for one EPUB3 `noteref -> footnote` relationship and one
explicit backlink. The number `[1]` is deliberately present, but numbering is
not sufficient evidence. The relationship is stated by `epub:type` and the
fragment target.

Expected negative control: a decoder must not classify an unrelated ordinary
`[1]` link as a footnote merely because this fixture contains a note.
