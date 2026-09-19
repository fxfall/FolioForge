# Markdown

`folio-markdown` accepts Markdown source and lowers common headings,
paragraphs, emphasis, links, lists, code-like blocks and inline text into the
Semantic IR. The adapter is intentionally conservative: unsupported Markdown
extensions are not silently interpreted as ebook semantics. Links and local
resource paths must pass the shared input safety rules.

Markdown has no embedded ebook package metadata by default, so title/author
may be supplied through the edit plan or CLI options.
