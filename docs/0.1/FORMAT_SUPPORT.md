# FolioForge 0.1 Format Support

This matrix describes the current code, not an intended future implementation.
`Native` means the target model can carry the feature directly; `Compatible`
means a target-specific representation is used; `Approximate` means the
meaning or appearance is intentionally reduced; `Flattenable` means structure
can be made readable but not preserved; `Unsupported` means strict mode
rejects it.

## Input/output matrix

| Format | Input | Output | Current boundary |
| --- | --- | --- | --- |
| EPUB/EPUB3 | Yes | Yes | OCF/OPF, spine, XHTML, CSS, resources, nav and NCX are mapped into IR. |
| KF7/MOBI | Yes | Yes | Palm/MOBI records, text, images and compatible structure; old-device features are target-limited. |
| KF8/AZW3 | Yes | Yes | Kindle HTML/CSS/resource model through the shared IR; unsupported CSS is diagnosed. |
| KF7+KF8 Combo | Yes | Yes | Explicit FolioForge composite container; not an inferred undocumented Amazon boundary. |
| KFX | Yes, DRM-free reflowable | Yes, internal writer | Native Ion/CONT recovery is diagnostic-first; fixed-layout and protected content are outside 0.1. |
| TXT | Yes | Via IR | Encoding and paragraph policy are explicit; chapter inference remains heuristic and reported. |
| Markdown | Yes | Via IR | Common headings, paragraphs, emphasis, links, lists and code-like blocks. |
| HTML | Yes | Via IR | Local HTML and referenced resources under safe source rules. |
| HTMLZ | Yes | Via IR | ZIP HTML package with safe relative paths. |
| FB2 | Yes | Via IR | XML body, metadata, images, notes and common inline semantics. |
| DOCX/OOXML | Yes | Via IR | Main document, styles, numbering, relationships, images, links, notes and tracked-view diagnostics. |

“Via IR” means the format is an input adapter but has no separate target
extension; it can be converted to any target that the capability plan accepts.

## Target feature profiles

The executable source of truth is `crates/folio-capabilities/src/lib.rs`.

| Feature | EPUB3 | KF7 | KF8 | KFX writer profile |
| --- | --- | --- | --- | --- |
| Text, headings | Native | Native | Native | Native |
| Navigation | Native | Compatible | Compatible | Native |
| Images | Native | Compatible | Native | Native |
| SVG | Native | Approximate | Native | Native |
| Fonts | Native | Approximate | Native | Native |
| Ruby | Native | Approximate | Compatible | Native |
| Math | Native | Approximate | Approximate | Native |
| Vertical writing | Native | Unsupported | Compatible | Native |
| Tables/floats | Native | Flattenable | Compatible | Compatible/native as represented |
| Fixed position/layout | Native | Flattenable | Compatible | Native in writer model; real-input scope is separate |
| Footnote/endnote | Native | Approximate | Compatible | Native in writer model |
| Page break/drop cap/poetry | Native | Native/Approximate | Native/Compatible | Native |
| Link/semantic structure | Native | Compatible | Native | Native |

The KFX writer column is not a claim that arbitrary Amazon KFX input can be
reconstructed. Real KFX input has its own input diagnostics and unresolved
state.

## Evidence policy

The Semantic IR is the only cross-format authority. Calibre KFX Input, Bōkō
and Kindle Previewer are external compatibility references used in private
validation; their output does not replace raw source evidence and none is a
runtime dependency. Exact resource identity, footnote/backlink, conditional
content, illustrated layout and MathML relationships are accepted only when
the source structure and native input evidence agree.
