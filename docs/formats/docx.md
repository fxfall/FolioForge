# DOCX / OOXML

`folio-docx` is an input adapter. It validates the OOXML ZIP, content types,
relationships and safe package paths, then imports `word/document.xml`, style
and numbering information, images, hyperlinks, footnotes/endnotes and the
accepted tracked-change view into the shared IR.

Direct formatting, style inheritance cycles, missing numbering parts,
unsupported relationship parts and macro-enabled packages are diagnosed.
DOCM/macro execution is not supported. DOCX is converted through the same
target planner as every other input; it does not create a DOCX-specific editor
or output path.
