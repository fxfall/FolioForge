# FB2

`folio-fb2` parses FictionBook XML metadata, body sections, common inline
semantics, notes and embedded images into the shared IR. XML namespaces and
base64 resources are validated before import. FB2-specific presentation that
has no semantic equivalent becomes a diagnostic or target fallback.

The writer target is the common ebook targets through IR; FolioForge 0.1 does
not promise an FB2 writer.
