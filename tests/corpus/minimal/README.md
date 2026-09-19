# Minimal EPUB corpus

每个 fixture 只改变一个主要变量，避免把 table、font、SVG、ruby 等特性混在一个差异里。`manifest.tsv` 是 Gate A 的 21 项清单；`tests/scripts/build_minimal_corpus.sh` 可以在本机生成标准 EPUB ZIP，不把生成的二进制书籍提交到仓库。

覆盖：text、heading、bold、italic、font-size、indent、margin、align、image、SVG、table、link、footnote、ruby、font、pagebreak、list、nested-list、writing-mode、navigation、EPUB2。
