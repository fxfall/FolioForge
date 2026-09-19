# HTML / HTMLZ

The HTML adapter imports local HTML; the HTMLZ adapter reads a ZIP package and
uses safe relative-path traversal. The import path maps headings, paragraphs,
inline formatting, links, anchors, lists, tables, images, SVG and referenced
stylesheets when their relationship is available.

HTML is not treated as a browser application. Scripts, unsafe external
references and unsupported layout are not executed; the compatibility plan
reports any resulting loss. HTMLZ archives must not escape their root through
absolute or parent paths.
