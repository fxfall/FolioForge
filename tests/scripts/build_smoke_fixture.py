#!/usr/bin/env python3
"""Create the small public EPUB used by native platform smoke tests."""

from __future__ import annotations

import base64
import sys
import zipfile
from pathlib import Path


PNG_1X1 = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk"
    "YAAAAAIAAeIhvzMAAAAASUVORK5CYII="
)


def write_entry(archive: zipfile.ZipFile, name: str, content: str | bytes) -> None:
    info = zipfile.ZipInfo(name)
    info.date_time = (2020, 1, 1, 0, 0, 0)
    if name == "mimetype":
        info.compress_type = zipfile.ZIP_STORED
    else:
        info.compress_type = zipfile.ZIP_DEFLATED
    archive.writestr(info, content)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: build_smoke_fixture.py OUTPUT_EPUB", file=sys.stderr)
        return 2

    output = Path(sys.argv[1])
    output.parent.mkdir(parents=True, exist_ok=True)
    opf = """<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0"
         unique-identifier="book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">urn:folioforge:github-smoke</dc:identifier>
    <dc:title>GitHub Smoke Fixture</dc:title>
    <dc:creator>FolioForge</dc:creator>
    <dc:language>zh</dc:language>
  </metadata>
  <manifest>
    <item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="image" href="image.png" media-type="image/png"/>
  </manifest>
  <spine><itemref idref="chapter"/></spine>
</package>
"""
    chapter = """<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"
      xmlns:epub="http://www.idpf.org/2007/ops">
  <head><title>GitHub Smoke Fixture</title></head>
  <body>
    <h1 id="chapter">跨平台原生构建</h1>
    <p>Plain text, 你好，こんにちは。</p>
    <p><a href="#note">[1]</a> <img src="image.png" alt="one pixel"/></p>
    <p id="note" epub:type="footnote">Smoke footnote.</p>
  </body>
</html>
"""
    nav = """<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"
      xmlns:epub="http://www.idpf.org/2007/ops">
  <body><nav epub:type="toc"><ol><li><a href="chapter.xhtml#chapter">Chapter</a></li></ol></nav></body>
</html>
"""
    container = """<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>
"""

    with zipfile.ZipFile(output, "w") as archive:
        write_entry(archive, "mimetype", "application/epub+zip")
        write_entry(archive, "META-INF/container.xml", container)
        write_entry(archive, "OEBPS/content.opf", opf)
        write_entry(archive, "OEBPS/chapter.xhtml", chapter)
        write_entry(archive, "OEBPS/nav.xhtml", nav)
        write_entry(archive, "OEBPS/image.png", PNG_1X1)

    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
