# Plain-text parser corpus

These small files are synthetic, non-copyrighted regression cases for the
plain-text adapter. They exercise headings and paragraph behavior; they are not a claim of
general language accuracy. Expected chapter counts and sequence checks are
asserted by `crates/folio-text/tests/structure_corpus.rs`.

The folders cover `zh-cn`, `zh-tw`, `ja`, `en`, `mixed`, `hard-wrap`, `plain`,
and `malformed`. Encoding conversion cases are generated in unit tests so
fixtures stay reviewable UTF-8 text.
