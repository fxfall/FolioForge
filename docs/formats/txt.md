# Plain text

`folio-text` detects common Unicode and legacy encodings, reports its selected
encoding, and creates ordered paragraphs through the shared IR. The caller can
choose automatic or explicit encoding and paragraph policies. Hard wraps,
blank-line conventions, mixed scripts and malformed sequences are handled by
the text import options and reported in the input report.

Text has no authoritative navigation or resource metadata. Chapter inference
is therefore a policy decision, not a recovered source fact. Use `analyze` to
review the selected compatibility plan before producing an ebook.
