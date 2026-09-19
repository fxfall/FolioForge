#!/bin/sh
set -eu

manifest=$(CDPATH= cd -- "$(dirname -- "$0")/../corpus/minimal" && pwd)/manifest.tsv
output_root=${1:-"/tmp/folioforge-minimal-corpus"}
work=$(mktemp -d "${TMPDIR:-/tmp}/folioforge-corpus.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM

mkdir -p "$output_root"
output_root=$(CDPATH= cd -- "$output_root" && pwd)

while IFS="$(printf '\t')" read -r fixture description; do
    [ -n "$fixture" ] || continue
    case "$fixture" in
        \#*) continue ;;
    esac

    package="$work/$fixture"
    mkdir -p "$package/META-INF" "$package/OEBPS"
    printf '%s' 'application/epub+zip' > "$package/mimetype"
    cat > "$package/META-INF/container.xml" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>
EOF

    version=3.0
    body='<p>Plain text fixture: Hello, 你好，こんにちは。</p>'
    style_item=
    style_body=
    extra_manifest=
    spine_toc=
    guide=

    case "$fixture" in
        002-heading)
            body='<h1 id="heading">A heading</h1><p>Body text.</p>'
            ;;
        003-bold)
            body='<p>Normal <strong>bold</strong> text.</p>'
            ;;
        004-italic)
            body='<p>Normal <em>italic</em> text.</p>'
            ;;
        005-font-size)
            style_item='<item id="style" href="style.css" media-type="text/css"/>'
            style_body='p.large { font-size: 2em; }'
            body='<p class="large">Large text.</p>'
            ;;
        006-indent)
            style_item='<item id="style" href="style.css" media-type="text/css"/>'
            style_body='p.indent { text-indent: 2em; }'
            body='<p class="indent">Indented text.</p>'
            ;;
        007-margin)
            style_item='<item id="style" href="style.css" media-type="text/css"/>'
            style_body='p.margin { margin: 1em; }'
            body='<p class="margin">Margin text.</p>'
            ;;
        008-align)
            style_item='<item id="style" href="style.css" media-type="text/css"/>'
            style_body='p.align { text-align: center; }'
            body='<p class="align">Centered text.</p>'
            ;;
        009-image)
            extra_manifest='<item id="image" href="image.png" media-type="image/png"/>'
            body='<p><img src="image.png" alt="one pixel"/></p>'
            if command -v xxd >/dev/null 2>&1; then
                printf '%s' '89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c6360000000020001e221bc33000000000049454e44ae426082' | xxd -r -p > "$package/OEBPS/image.png"
            else
                printf '%s\n' 'fixture image bytes' > "$package/OEBPS/image.png"
            fi
            ;;
        010-svg)
            extra_manifest='<item id="image" href="image.svg" media-type="image/svg+xml"/>'
            body='<p><img src="image.svg" alt="vector image"/></p>'
            cat > "$package/OEBPS/image.svg" <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#d9ead3"/></svg>
EOF
            ;;
        011-table)
            body='<table><tr><th>Key</th><th>Value</th></tr><tr><td>A</td><td>B</td></tr></table>'
            ;;
        012-link)
            body='<p id="source"><a href="chapter.xhtml#target">Jump</a></p><p id="target">Target.</p>'
            ;;
        013-footnote)
            body='<p>Main text <a epub:type="footnote" href="chapter.xhtml#note">[1]</a></p><p id="note">Footnote.</p>'
            ;;
        014-ruby)
            body='<p><ruby>漢<rt>かん</rt></ruby>字。</p>'
            ;;
        015-font)
            style_item='<item id="style" href="style.css" media-type="text/css"/>'
            extra_manifest='<item id="font" href="font.woff" media-type="font/woff"/>'
            style_body='@font-face { font-family: Fixture; src: url("font.woff"); } body { font-family: Fixture; }'
            body='<p>Font resource.</p>'
            printf '%s' 'fixture font bytes' > "$package/OEBPS/font.woff"
            ;;
        016-pagebreak)
            body='<p>Before.</p><hr/><p>After.</p>'
            ;;
        017-list)
            body='<ol><li>One</li><li>Two</li></ol><ul><li>Three</li></ul>'
            ;;
        018-nested-list)
            body='<ul><li>Outer<ul><li>Inner</li></ul></li></ul>'
            ;;
        019-writing-mode)
            style_item='<item id="style" href="style.css" media-type="text/css"/>'
            style_body='body { writing-mode: vertical-rl; }'
            body='<p>縦書き。</p>'
            ;;
        020-navigation)
            extra_manifest='<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>'
            spine_toc=' toc="ncx"'
            body='<h1 id="chapter">Navigation fixture</h1><p>Chapter body.</p>'
            cat > "$package/OEBPS/nav.xhtml" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="chapter.xhtml#chapter">Chapter</a></li></ol></nav></body></html>
EOF
            cat > "$package/OEBPS/toc.ncx" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx" version="2005-1"><navMap><navPoint id="one"><navLabel><text>Chapter</text></navLabel><content src="chapter.xhtml#chapter"/></navPoint></navMap></ncx>
EOF
            ;;
        021-epub2)
            version=2.0
            extra_manifest='<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>'
            spine_toc=' toc="ncx"'
            guide='<guide><reference type="text" title="Start" href="chapter.xhtml"/></guide>'
            body='<h1 id="chapter">EPUB2 chapter</h1><p>EPUB2 body.</p>'
            cat > "$package/OEBPS/toc.ncx" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx" version="2005-1"><navMap><navPoint id="one"><navLabel><text>Chapter</text></navLabel><content src="chapter.xhtml#chapter"/></navPoint></navMap></ncx>
EOF
            ;;
    esac

    cat > "$package/OEBPS/chapter.xhtml" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>$fixture</title></head><body>$body</body></html>
EOF

    if [ -n "$style_item" ]; then
        printf '%s\n' "$style_body" > "$package/OEBPS/style.css"
    fi

    cat > "$package/OEBPS/content.opf" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="$version" unique-identifier="book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="book-id">urn:folioforge:$fixture</dc:identifier><dc:title>$fixture</dc:title><dc:creator>FolioForge Fixture</dc:creator><dc:language>en</dc:language></metadata>
  <manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>$style_item$extra_manifest</manifest>
  <spine$spine_toc><itemref idref="chapter"/></spine>
  $guide
</package>
EOF

    output="$output_root/$fixture.epub"
    rm -f "$output"
    (cd "$package" && zip -X -q -0 "$output" mimetype && zip -X -q -r "$output" META-INF OEBPS)
done < "$manifest"

printf 'generated minimal EPUB fixtures in %s\n' "$output_root"
