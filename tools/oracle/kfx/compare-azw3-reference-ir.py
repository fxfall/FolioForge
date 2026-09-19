#!/usr/bin/env python3
"""Compare AZW3 reference outputs through FolioForge's semantic IR.

The extractor intentionally emits no visible book text.  Text is represented
by counts and audit-only hashes so that a corpus report can be checked into a
local audit directory without becoming a copy of the books under test.

The same FolioForge semantic importer is used for every input family.  This
does not make Calibre or Bōkō authoritative; it gives us one stable IR schema
for measuring where their EPUB structures differ from the AZW3-native import
and from FolioForge's serialized EPUB.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import unicodedata
from collections import Counter
from pathlib import Path
from typing import Any, Iterable


FEATURE_PROPERTIES = (
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "line-height",
    "text-align",
    "text-indent",
    "writing-mode",
    "direction",
    "display",
    "margin",
    "margin-left",
    "margin-right",
    "margin-top",
    "margin-bottom",
    "padding",
    "white-space",
)


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def stable_json_hash(value: Any) -> str:
    payload = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return sha256_text(payload)


def normalized_text(value: str) -> str:
    value = value.replace("\r\n", "\n").replace("\r", "\n")
    return unicodedata.normalize("NFC", value)


def whitespace_stripped_text(value: str) -> str:
    """Return a comparison form that ignores Unicode whitespace only.

    This is an audit metric, not a production normalization rule.  EPUB
    serializers and KFX conversion tools are allowed to make different line
    wrapping decisions, so this metric separates those changes from dropped
    or reordered non-whitespace characters.
    """

    return re.sub(r"\s+", "", normalized_text(value))


def text_from_nodes(nodes: Iterable[dict[str, Any]]) -> str:
    fragments: list[str] = []
    for node in nodes:
        kind = node.get("kind")
        if isinstance(kind, dict):
            text_node = kind.get("Text")
            if isinstance(text_node, dict):
                value = text_node.get("value")
                if isinstance(value, str):
                    fragments.append(value)
        children = node.get("children")
        if isinstance(children, list):
            fragments.append(text_from_nodes(children))
    return "".join(fragments)


def walk_nodes(nodes: Iterable[dict[str, Any]]) -> Iterable[dict[str, Any]]:
    for node in nodes:
        yield node
        children = node.get("children")
        if isinstance(children, list):
            yield from walk_nodes(children)


def node_kind(node: dict[str, Any]) -> str:
    kind = node.get("kind")
    if isinstance(kind, dict) and kind:
        return next(iter(kind))
    if isinstance(kind, str):
        return kind
    return "Unknown"


def node_heading_label(node: dict[str, Any]) -> str:
    children = node.get("children")
    if not isinstance(children, list):
        return ""
    return text_from_nodes(children)


def summarize_nodes(nodes: list[dict[str, Any]]) -> dict[str, Any]:
    all_nodes = list(walk_nodes(nodes))
    text_values: list[str] = []
    roles: Counter[str] = Counter()
    kinds: Counter[str] = Counter()
    links: list[str] = []
    heading_levels: list[int] = []
    heading_labels: list[str] = []

    for node in all_nodes:
        role = node.get("role")
        if isinstance(role, str):
            roles[role] += 1
        kinds[node_kind(node)] += 1
        kind = node.get("kind")
        if isinstance(kind, dict):
            text_node = kind.get("Text")
            if isinstance(text_node, dict) and isinstance(text_node.get("value"), str):
                text_values.append(text_node["value"])
            link_node = kind.get("Link")
            if isinstance(link_node, dict) and isinstance(link_node.get("href"), str):
                links.append(link_node["href"])
            heading = kind.get("Heading")
            if isinstance(heading, dict) and isinstance(heading.get("level"), int):
                heading_levels.append(heading["level"])
                heading_labels.append(node_heading_label(node))

    text = "".join(text_values)
    normalized = normalized_text(text)
    return {
        "node_count": len(all_nodes),
        "role_counts": dict(sorted(roles.items())),
        "kind_counts": dict(sorted(kinds.items())),
        "text_fragment_count": len(text_values),
        "text_unicode_scalar_count": len(text),
        "text_sha256": sha256_text(text),
        "text_normalized_sha256": sha256_text(normalized),
        "link_count": len(links),
        "link_href_sha256": stable_json_hash(links),
        "heading_count": len(heading_levels),
        "heading_levels": heading_levels,
        "heading_labels_sha256": stable_json_hash(heading_labels),
    }


def summarize_document(document: dict[str, Any]) -> dict[str, Any]:
    nodes = document.get("nodes")
    if not isinstance(nodes, list):
        nodes = []
    summary = summarize_nodes(nodes)
    summary.update(
        {
            "id": document.get("id"),
            "href": document.get("href"),
            "media_type": document.get("media_type"),
            "title_present": bool(document.get("title")),
        }
    )
    return summary


def summarize_resources(resources: Any) -> dict[str, Any]:
    if not isinstance(resources, list):
        resources = []
    kinds: Counter[str] = Counter()
    media_types: Counter[str] = Counter()
    paths: list[str] = []
    sizes: list[int] = []
    feature_resources: dict[str, int] = {}
    for resource in resources:
        if not isinstance(resource, dict):
            continue
        kind = resource.get("kind")
        media_type = resource.get("media_type")
        path = resource.get("path")
        size = resource.get("size")
        if isinstance(kind, str):
            kinds[kind] += 1
        if isinstance(media_type, str):
            media_types[media_type] += 1
        if isinstance(path, str):
            paths.append(path)
        if isinstance(size, int):
            sizes.append(size)
        if isinstance(kind, str) and ("font" in kind.lower() or "image" in kind.lower()):
            feature_resources[kind] = feature_resources.get(kind, 0) + 1
    return {
        "count": len(resources),
        "kind_counts": dict(sorted(kinds.items())),
        "media_type_counts": dict(sorted(media_types.items())),
        "font_or_image_kind_counts": dict(sorted(feature_resources.items())),
        "path_sequence_sha256": stable_json_hash(paths),
        "total_bytes": sum(sizes),
    }


def summarize_styles(styles: Any) -> dict[str, Any]:
    if not isinstance(styles, list):
        styles = []
    property_counts: Counter[str] = Counter()
    feature_values: dict[str, Counter[str]] = {
        property_name: Counter() for property_name in FEATURE_PROPERTIES
    }
    for style in styles:
        if not isinstance(style, dict) or not isinstance(style.get("properties"), dict):
            continue
        for property_name, value in style["properties"].items():
            if not isinstance(property_name, str):
                continue
            property_counts[property_name] += 1
            if property_name in feature_values and isinstance(value, str):
                feature_values[property_name][value] += 1
    return {
        "count": len(styles),
        "property_counts": dict(sorted(property_counts.items())),
        "feature_values": {
            property_name: dict(sorted(values.items()))
            for property_name, values in feature_values.items()
            if values
        },
    }


def summarize_navigation(navigation: Any) -> dict[str, Any]:
    """Summarize the ordered navigation tree without emitting labels."""

    if not isinstance(navigation, dict):
        navigation = {}
    toc_points = navigation.get("toc")
    if not isinstance(toc_points, list):
        toc_points = []

    flattened: list[dict[str, Any]] = []

    def visit(points: list[Any], depth: int) -> None:
        for point in points:
            if not isinstance(point, dict):
                continue
            flattened.append(
                {
                    "depth": depth,
                    "href": point.get("href"),
                    "label": point.get("label"),
                }
            )
            children = point.get("children")
            if isinstance(children, list):
                visit(children, depth + 1)

    visit(toc_points, 0)
    return {
        "toc_count": len(flattened),
        "toc_sequence_sha256": stable_json_hash(flattened),
        "toc_label_sequence_sha256": stable_json_hash(
            [item.get("label") for item in flattened]
        ),
        "toc_href_sequence_sha256": stable_json_hash(
            [item.get("href") for item in flattened]
        ),
        "toc_depth_sequence_sha256": stable_json_hash(
            [item.get("depth") for item in flattened]
        ),
    }


def summarize_semantic(document: dict[str, Any]) -> dict[str, Any]:
    metadata = document.get("metadata")
    if not isinstance(metadata, dict):
        metadata = {}
    documents = document.get("documents")
    if not isinstance(documents, list):
        documents = []
    document_summaries = [
        summarize_document(item) for item in documents if isinstance(item, dict)
    ]
    whole_text = "".join(
        text_from_nodes(item.get("nodes", []))
        for item in documents
        if isinstance(item, dict) and isinstance(item.get("nodes"), list)
    )
    whole_normalized = normalized_text(whole_text)
    whole_whitespace_stripped = whitespace_stripped_text(whole_text)
    hrefs = [item.get("href") for item in document_summaries]
    headings = [
        {"levels": item["heading_levels"], "labels_sha256": item["heading_labels_sha256"]}
        for item in document_summaries
    ]
    navigation = document.get("navigation")
    navigation_summary = summarize_navigation(navigation)
    navigation_edges = []
    if isinstance(navigation, dict):
        anchor_graph = navigation.get("anchor_graph")
        if isinstance(anchor_graph, dict) and isinstance(anchor_graph.get("edges"), list):
            navigation_edges = [
                {
                    "relation": edge.get("relation"),
                    "source": edge.get("source"),
                    "target": edge.get("target"),
                }
                for edge in anchor_graph["edges"]
                if isinstance(edge, dict)
            ]
    return {
        "available": True,
        "metadata": {
            key: metadata.get(key)
            for key in (
                "title",
                "authors",
                "language",
                "publisher",
                "series",
                "series_index",
            )
            if metadata.get(key) is not None
        },
        "document_count": len(document_summaries),
        "document_href_sequence_sha256": stable_json_hash(hrefs),
        "documents": document_summaries,
        "whole_text_unicode_scalar_count": len(whole_text),
        "whole_text_sha256": sha256_text(whole_text),
        "whole_text_normalized_sha256": sha256_text(whole_normalized),
        "whole_text_whitespace_stripped_unicode_scalar_count": len(
            whole_whitespace_stripped
        ),
        "whole_text_whitespace_stripped_sha256": sha256_text(whole_whitespace_stripped),
        "heading_count": sum(item["heading_count"] for item in document_summaries),
        "heading_sequence_sha256": stable_json_hash(headings),
        "navigation_edge_count": len(navigation_edges),
        "navigation_edges_sha256": stable_json_hash(navigation_edges),
        "navigation_toc": navigation_summary,
        "resources": summarize_resources(document.get("resources")),
        "styles": summarize_styles(document.get("styles")),
        "font_face_count": len(document.get("font_faces", []))
        if isinstance(document.get("font_faces"), list)
        else 0,
    }


def compare_summaries(left: dict[str, Any], right: dict[str, Any]) -> dict[str, Any]:
    if not left.get("available") or not right.get("available"):
        return {"available": False}
    return {
        "available": True,
        "document_count_equal": left["document_count"] == right["document_count"],
        "document_href_sequence_equal": left["document_href_sequence_sha256"]
        == right["document_href_sequence_sha256"],
        "whole_text_sha256_equal": left["whole_text_sha256"] == right["whole_text_sha256"],
        "whole_text_normalized_sha256_equal": left["whole_text_normalized_sha256"]
        == right["whole_text_normalized_sha256"],
        "whole_text_whitespace_stripped_sha256_equal": left[
            "whole_text_whitespace_stripped_sha256"
        ]
        == right["whole_text_whitespace_stripped_sha256"],
        "whole_text_unicode_scalar_delta": right["whole_text_unicode_scalar_count"]
        - left["whole_text_unicode_scalar_count"],
        "whole_text_whitespace_stripped_unicode_scalar_delta": right[
            "whole_text_whitespace_stripped_unicode_scalar_count"
        ]
        - left["whole_text_whitespace_stripped_unicode_scalar_count"],
        "heading_sequence_equal": left["heading_sequence_sha256"]
        == right["heading_sequence_sha256"],
        "navigation_edges_equal": left["navigation_edges_sha256"]
        == right["navigation_edges_sha256"],
        "navigation_toc_equal": left["navigation_toc"] == right["navigation_toc"],
        "resource_profile_equal": left["resources"] == right["resources"],
        "style_feature_profile_equal": left["styles"] == right["styles"],
        "font_face_count_equal": left["font_face_count"] == right["font_face_count"],
    }


def run_semantic(folio: Path, input_path: Path) -> dict[str, Any]:
    completed = subprocess.run(
        [str(folio), "inspect", str(input_path), "--semantic"],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        return {
            "available": False,
            "returncode": completed.returncode,
            "stderr_sha256": sha256_text(completed.stderr),
        }
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return {
            "available": False,
            "returncode": completed.returncode,
            "stdout_sha256": sha256_text(completed.stdout),
            "stderr_sha256": sha256_text(completed.stderr),
        }
    return summarize_semantic(payload)


def path_for_stem(root: Path | None, stem: str, suffix: str) -> Path | None:
    """Find a reference artifact while tolerating tool-added suffixes."""

    if root is None or not root.is_dir():
        return None
    exact = root / f"{stem}{suffix}"
    if exact.is_file():
        return exact
    candidates = sorted(
        path
        for path in root.iterdir()
        if path.is_file()
        and path.suffix.lower() == suffix
        and path.stem.startswith(f"{stem}.")
    )
    if len(candidates) == 1:
        return candidates[0]
    return None


def source_entry(path: Path | None, folio: Path) -> dict[str, Any]:
    if path is None:
        return {"available": False, "reason": "missing"}
    summary = run_semantic(folio, path)
    summary["source_sha256"] = file_sha256(path)
    return summary


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(args: argparse.Namespace) -> int:
    source_root = args.source_root
    if source_root is None or not source_root.is_dir():
        source_map: dict[str, Path] = {}
    else:
        source_map = {
            path.stem: path
            for path in source_root.iterdir()
            if path.is_file() and path.suffix.lower() == ".azw3"
        }
    roots = {
        "calibre_epub": (args.calibre_root, ".epub"),
        "boko_epub": (args.boko_root, ".epub"),
        "folioforge_epub": (args.folioforge_root, ".epub"),
        "kfx_input_epub": (args.kfx_input_epub_root, ".epub"),
        "kfx_native": (args.kfx_root, ".kfx"),
    }
    stems = sorted(source_map)
    if args.limit is not None:
        stems = stems[: args.limit]
    books: dict[str, Any] = {}
    for stem in stems:
        entries = {
            "azw3_native": source_entry(source_map[stem], args.folio),
        }
        for name, (root, suffix) in roots.items():
            entries[name] = source_entry(
                path_for_stem(root, stem, suffix), args.folio
            )
        comparisons: dict[str, Any] = {}
        names = list(entries)
        for index, left_name in enumerate(names):
            for right_name in names[index + 1 :]:
                comparisons[f"{left_name}__vs__{right_name}"] = compare_summaries(
                    entries[left_name], entries[right_name]
                )
        books[stem] = {"sources": entries, "comparisons": comparisons}

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "comparison": "azw3-reference-output-semantic-ir",
                "text_policy": "body text is never emitted; hashes and counts only",
                "sources": ["azw3_native", *roots.keys()],
                "books": books,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--folio", required=True, type=Path)
    parser.add_argument("--source-root", required=True, type=Path)
    parser.add_argument("--calibre-root", type=Path)
    parser.add_argument("--boko-root", type=Path)
    parser.add_argument("--folioforge-root", type=Path)
    parser.add_argument("--kfx-input-epub-root", type=Path)
    parser.add_argument("--kfx-root", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--limit", type=int)
    args = parser.parse_args()
    try:
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.error(f"semantic IR comparison failed: {type(error).__name__}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
