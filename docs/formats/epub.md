# EPUB / EPUB3

`folio-epub` is the primary semantic interchange adapter. It reads an OCF ZIP,
the container rootfile, OPF metadata/manifest/spine, XHTML reading documents,
CSS, EPUB navigation and optional NCX. Relative resource paths are normalized
through `folio-input` safety rules.

The importer maps headings, paragraphs, inline styles, links/anchors,
footnote/endnote relations, lists, tables, ruby, MathML, images/SVG,
alternative text, fonts, page breaks and fixed/reflowable metadata into the
Semantic IR. The writer emits deterministic EPUB3 with OPF, XHTML, CSS,
resources, navigation and NCX compatibility data, then validates the result.

EPUB3 is the loss-minimizing target and all public capability features are
declared `Native` by the target matrix. A valid source can still produce a
diagnostic when malformed markup or an unsafe relationship prevents proof.
