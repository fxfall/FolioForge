#!/usr/bin/env python3
"""Compare Calibre, Bokō, and FolioForge EPUB output streams anonymously."""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import posixpath
import subprocess
import sys
import tempfile
import urllib.parse
import zipfile
from pathlib import Path
from typing import Any
from xml.etree import ElementTree


COMPARATOR_PATH = Path(__file__).with_name("compare-calibre-text.py")
COMPARATOR_SPEC = importlib.util.spec_from_file_location(
    "compare_calibre_text_for_three_way", COMPARATOR_PATH
)
if COMPARATOR_SPEC is None or COMPARATOR_SPEC.loader is None:
    raise RuntimeError("unable to load the neutral EPUB extractor")
COMPARATOR = importlib.util.module_from_spec(COMPARATOR_SPEC)
COMPARATOR_SPEC.loader.exec_module(COMPARATOR)


CONTAINER_NS = "urn:oasis:names:tc:opendocument:xmlns:container"
OPF_NS = "http://www.idpf.org/2007/opf"
XHTML_NS = "http://www.w3.org/1999/xhtml"
EPUB_NS = "http://www.idpf.org/2007/ops"


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def command_version(command: list[str]) -> str:
    result = subprocess.run(
        command,
        capture_output=True,
        text=True,
        check=True,
        timeout=30,
    )
    return (result.stdout or result.stderr).strip()


def epub_stream_summary(path: Path) -> dict[str, Any]:
    visible_text, document_count, xml_text_node_count = (
        COMPARATOR.epub_visible_text_stream(path)
    )
    normalized = COMPARATOR.summarize_text(visible_text)
    result = {
        "raw_unicode_scalar_count": len(visible_text),
        "raw_sha256_audit_only": hashlib.sha256(
            visible_text.encode("utf-8")
        ).hexdigest(),
        "normalized_unicode_scalar_count": normalized["unicode_scalar_count"],
        "normalized_sha256_audit_only": normalized[
            "normalized_sha256_audit_only"
        ],
        "document_count": document_count,
        "xml_text_node_count": xml_text_node_count,
    }
    result["structure"] = epub_structure_summary(path)
    return result


def _local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _content_compacted_text(body: ElementTree.Element) -> str:
    """Remove XML formatting-only whitespace for a secondary content check.

    The primary comparison above intentionally preserves the raw Unicode
    stream.  Calibre and Bokō emit different indentation and line-break text
    nodes around block elements, so this secondary stream drops whitespace
    characters only after the XHTML tree has been parsed.  It is a diagnostic
    signal, not a replacement for the primary comparison.
    """

    return "".join(
        character
        for piece in body.itertext()
        if piece
        for character in piece
        if not character.isspace()
    )


def _digest_values(values: list[str]) -> str:
    digest = hashlib.sha256()
    for value in values:
        digest.update(value.encode("utf-8"))
        digest.update(b"\0")
    return digest.hexdigest()


def _image_resource_digest(
    archive: zipfile.ZipFile,
    names: set[str],
    document_path: str,
    element: ElementTree.Element,
) -> str:
    source = element.get("src")
    if source is None:
        source = element.get("{%s}href" % "http://www.w3.org/1999/xlink")
    if source is None:
        source = element.get("href")
    if not source:
        return _digest_values(["missing-image-source"])
    if source.startswith("data:"):
        return _digest_values(["data-image", source])
    source_path = source.split("#", 1)[0]
    resource_path = posixpath.normpath(
        posixpath.join(posixpath.dirname(document_path), urllib.parse.unquote(source_path))
    )
    if resource_path not in names:
        return _digest_values(["unresolved-image-source", source])
    return hashlib.sha256(archive.read(resource_path)).hexdigest()


def _navigation_attribute(element: ElementTree.Element, name: str) -> str | None:
    return element.get("{%s}%s" % (EPUB_NS, name)) or element.get(name)


def _toc_signature(
    archive: zipfile.ZipFile,
    names: set[str],
    package_dir: str,
    manifest: ElementTree.Element,
) -> dict[str, Any]:
    def signature(labels: list[str], depths: list[int]) -> dict[str, Any]:
        return {
            "point_count": len(labels),
            "max_depth": max(depths, default=0),
            "label_sequence_hash": _digest_values(labels),
            "depth_sequence_hash": _digest_values([str(depth) for depth in depths]),
        }

    nav_href = None
    ncx_href = None
    for item in manifest.findall("{%s}item" % OPF_NS):
        properties = (item.get("properties") or "").split()
        if "nav" in properties and item.get("href"):
            nav_href = item.get("href")
        if item.get("media-type") == "application/x-dtbncx+xml" and item.get("href"):
            ncx_href = item.get("href")

    if nav_href is not None:
        nav_path = posixpath.normpath(
            posixpath.join(package_dir, urllib.parse.unquote(nav_href))
        )
        if nav_path.startswith("../") or nav_path not in names:
            raise ValueError("EPUB navigation document path is invalid")
        document = ElementTree.fromstring(archive.read(nav_path))
        toc_nav = next(
            (
                element
                for element in document.iter()
                if isinstance(element.tag, str)
                and _local_name(element.tag) == "nav"
                and _navigation_attribute(element, "type") == "toc"
            ),
            None,
        )
        if toc_nav is not None:
            labels: list[str] = []
            depths: list[int] = []

            def visit(ordered_list: ElementTree.Element, depth: int) -> None:
                for list_item in list(ordered_list):
                    if _local_name(list_item.tag) != "li":
                        continue
                    anchor = next(
                        (
                            child
                            for child in list(list_item)
                            if isinstance(child.tag, str) and _local_name(child.tag) == "a"
                        ),
                        None,
                    )
                    if anchor is not None:
                        labels.append(" ".join("".join(anchor.itertext()).split()))
                        depths.append(depth)
                    nested = next(
                        (
                            child
                            for child in list(list_item)
                            if isinstance(child.tag, str) and _local_name(child.tag) == "ol"
                        ),
                        None,
                    )
                    if nested is not None:
                        visit(nested, depth + 1)

            root_list = next(
                (
                    child
                    for child in list(toc_nav)
                    if isinstance(child.tag, str) and _local_name(child.tag) == "ol"
                ),
                None,
            )
            if root_list is not None:
                visit(root_list, 1)
            return signature(labels, depths)

    if ncx_href is None:
        return signature([], [])
    ncx_path = posixpath.normpath(
        posixpath.join(package_dir, urllib.parse.unquote(ncx_href))
    )
    if ncx_path.startswith("../") or ncx_path not in names:
        raise ValueError("EPUB NCX document path is invalid")
    document = ElementTree.fromstring(archive.read(ncx_path))
    nav_map = next(
        (
            element
            for element in document.iter()
            if isinstance(element.tag, str) and _local_name(element.tag) == "navMap"
        ),
        None,
    )
    if nav_map is None:
        return signature([], [])
    labels = []
    depths = []

    def visit_ncx(parent: ElementTree.Element, depth: int) -> None:
        for point in list(parent):
            if _local_name(point.tag) != "navPoint":
                continue
            label = next(
                (
                    element
                    for element in point.iter()
                    if isinstance(element.tag, str) and _local_name(element.tag) == "text"
                ),
                None,
            )
            if label is not None:
                labels.append(" ".join("".join(label.itertext()).split()))
                depths.append(depth)
            visit_ncx(point, depth + 1)

    visit_ncx(nav_map, 1)
    return signature(labels, depths)


def epub_structure_summary(path: Path) -> dict[str, Any]:
    """Summarize EPUB spine structure without retaining output text.

    Per-document compacted hashes let the three-way audit distinguish
    packaging whitespace from a real block/content mismatch.  Counts are
    deliberately limited to reader-visible XHTML elements and never include
    filenames from the original KFX input.
    """

    with zipfile.ZipFile(path) as archive:
        names = set(archive.namelist())
        container = ElementTree.fromstring(archive.read("META-INF/container.xml"))
        rootfile = container.find(".//{%s}rootfile" % CONTAINER_NS)
        if rootfile is None or not rootfile.get("full-path"):
            raise ValueError("EPUB package rootfile is missing")
        package_path = rootfile.get("full-path")
        package_dir = posixpath.dirname(package_path)
        package = ElementTree.fromstring(archive.read(package_path))
        manifest = package.find("{%s}manifest" % OPF_NS)
        spine = package.find("{%s}spine" % OPF_NS)
        if manifest is None or spine is None:
            raise ValueError("EPUB manifest or spine is missing")
        hrefs = {
            item.get("id"): item.get("href")
            for item in manifest.findall("{%s}item" % OPF_NS)
            if item.get("id") and item.get("href")
        }
        toc = _toc_signature(archive, names, package_dir, manifest)
        document_summaries = []
        aggregate_tags = Counter()
        aggregate_direct_tags = Counter()
        aggregate_compacted_scalar_count = 0
        aggregate_compacted_digest = hashlib.sha256()
        aggregate_image_hashes: list[str] = []
        for ordinal, itemref in enumerate(spine.findall("{%s}itemref" % OPF_NS)):
            href = hrefs.get(itemref.get("idref"))
            if href is None:
                continue
            document_path = posixpath.normpath(
                posixpath.join(package_dir, urllib.parse.unquote(href))
            )
            if document_path.startswith("../") or document_path not in names:
                raise ValueError("EPUB spine document path is invalid")
            document = ElementTree.fromstring(archive.read(document_path))
            body = document.find(".//{%s}body" % XHTML_NS)
            if body is None:
                raise ValueError("EPUB spine document has no XHTML body")
            tags = Counter(
                _local_name(element.tag)
                for element in body.iter()
                if isinstance(element.tag, str)
            )
            direct_tags = Counter(
                _local_name(element.tag)
                for element in list(body)
                if isinstance(element.tag, str)
            )
            compacted = _content_compacted_text(body)
            image_hashes = [
                _image_resource_digest(archive, names, document_path, element)
                for element in body.iter()
                if isinstance(element.tag, str)
                and _local_name(element.tag) in ("img", "image")
            ]
            aggregate_tags.update(tags)
            aggregate_direct_tags.update(direct_tags)
            aggregate_compacted_scalar_count += len(compacted)
            aggregate_compacted_digest.update(compacted.encode("utf-8"))
            aggregate_image_hashes.extend(image_hashes)
            document_summaries.append(
                {
                    "spine_ordinal": ordinal,
                    "content_compacted_unicode_scalar_count": len(compacted),
                    "content_compacted_sha256_audit_only": hashlib.sha256(
                        compacted.encode("utf-8")
                    ).hexdigest(),
                    "paragraph_count": tags.get("p", 0),
                    "heading_count": sum(tags.get(f"h{level}", 0) for level in range(1, 7)),
                    "image_count": tags.get("img", 0),
                    # Calibre may wrap a cover/raster placement in SVG,
                    # while FolioForge/Bokō emit a direct XHTML img.  Keep
                    # the raw count for transparency and expose a semantic
                    # image-like count for structure parity.
                    "image_like_count": tags.get("img", 0) + tags.get("image", 0),
                    "image_sequence_sha256_audit_only": _digest_values(image_hashes),
                    "image_resource_multiset_sha256_audit_only": _digest_values(
                        sorted(image_hashes)
                    ),
                    "link_count": tags.get("a", 0),
                    "direct_body_tag_counts": dict(sorted(direct_tags.items())),
                }
            )
    return {
        "document_count": len(document_summaries),
        "content_compacted_unicode_scalar_count": aggregate_compacted_scalar_count,
        "content_compacted_sha256_audit_only": aggregate_compacted_digest.hexdigest(),
        "paragraph_count": aggregate_tags.get("p", 0),
        "heading_count": sum(
            aggregate_tags.get(f"h{level}", 0) for level in range(1, 7)
        ),
        "image_count": aggregate_tags.get("img", 0),
        "image_like_count": aggregate_tags.get("img", 0) + aggregate_tags.get("image", 0),
        "image_sequence_sha256_audit_only": _digest_values(aggregate_image_hashes),
        "image_resource_multiset_sha256_audit_only": _digest_values(
            sorted(aggregate_image_hashes)
        ),
        "toc_point_count": toc["point_count"],
        "toc_max_depth": toc["max_depth"],
        "toc_label_sequence_sha256_audit_only": toc["label_sequence_hash"],
        "toc_depth_sequence_sha256_audit_only": toc["depth_sequence_hash"],
        "link_count": aggregate_tags.get("a", 0),
        "tag_counts": dict(sorted(aggregate_tags.items())),
        "direct_body_tag_counts": dict(sorted(aggregate_direct_tags.items())),
        "documents": document_summaries,
    }


def pairwise(left: dict[str, Any], right: dict[str, Any]) -> dict[str, Any]:
    result = {
        "comparable": True,
        "raw_scalar_count_equal": left["raw_unicode_scalar_count"]
        == right["raw_unicode_scalar_count"],
        "raw_hash_equal": left["raw_sha256_audit_only"]
        == right["raw_sha256_audit_only"],
        "normalized_scalar_count_equal": left["normalized_unicode_scalar_count"]
        == right["normalized_unicode_scalar_count"],
        "normalized_hash_equal": left["normalized_sha256_audit_only"]
        == right["normalized_sha256_audit_only"],
    }
    left_structure = left.get("structure", {})
    right_structure = right.get("structure", {})
    left_documents = left_structure.get("documents", [])
    right_documents = right_structure.get("documents", [])
    result.update(
        {
            "content_compacted_scalar_count_equal": left_structure.get(
                "content_compacted_unicode_scalar_count"
            )
            == right_structure.get("content_compacted_unicode_scalar_count"),
            "content_compacted_hash_equal": left_structure.get(
                "content_compacted_sha256_audit_only"
            )
            == right_structure.get("content_compacted_sha256_audit_only"),
            "document_content_compacted_hash_match_count": sum(
                left_document.get("content_compacted_sha256_audit_only")
                == right_document.get("content_compacted_sha256_audit_only")
                for left_document, right_document in zip(left_documents, right_documents)
            ),
            "document_count_equal": left_structure.get("document_count")
            == right_structure.get("document_count"),
            "paragraph_count_equal": left_structure.get("paragraph_count")
            == right_structure.get("paragraph_count"),
            "heading_count_equal": left_structure.get("heading_count")
            == right_structure.get("heading_count"),
            "image_count_equal": left_structure.get("image_count")
            == right_structure.get("image_count"),
            "image_like_count_equal": left_structure.get("image_like_count")
            == right_structure.get("image_like_count"),
            "image_sequence_hash_equal": left_structure.get(
                "image_sequence_sha256_audit_only"
            )
            == right_structure.get("image_sequence_sha256_audit_only"),
            "image_resource_multiset_equal": left_structure.get(
                "image_resource_multiset_sha256_audit_only"
            )
            == right_structure.get("image_resource_multiset_sha256_audit_only"),
            "toc_point_count_equal": left_structure.get("toc_point_count")
            == right_structure.get("toc_point_count"),
            "toc_max_depth_equal": left_structure.get("toc_max_depth")
            == right_structure.get("toc_max_depth"),
            "toc_label_sequence_hash_equal": left_structure.get(
                "toc_label_sequence_sha256_audit_only"
            )
            == right_structure.get("toc_label_sequence_sha256_audit_only"),
            "toc_depth_sequence_hash_equal": left_structure.get(
                "toc_depth_sequence_sha256_audit_only"
            )
            == right_structure.get("toc_depth_sequence_sha256_audit_only"),
        }
    )
    return result


def resolve_sources(book_root: Path, books: list[dict[str, Any]]) -> dict[str, Path]:
    matches: dict[str, list[Path]] = {
        book["source_sha256_audit_only"]: [] for book in books
    }
    for path in book_root.iterdir():
        if path.is_file() and not path.is_symlink() and path.suffix.lower() == ".kfx":
            digest = file_sha256(path)
            if digest in matches:
                matches[digest].append(path)
    if any(len(paths) != 1 for paths in matches.values()):
        raise ValueError("a pinned corpus input did not resolve uniquely")
    return {digest: paths[0] for digest, paths in matches.items()}


def run_converter(command: list[str], output: Path) -> dict[str, Any]:
    try:
        result = subprocess.run(
            command,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
            timeout=900,
        )
    except subprocess.TimeoutExpired:
        return {"status": "timeout"}
    if result.returncode != 0:
        return {"status": "failed", "returncode": result.returncode}
    if not output.is_file():
        return {"status": "missing_output"}
    try:
        summary = epub_stream_summary(output)
    except (OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile):
        return {"status": "invalid_epub"}
    return {"status": "success", "summary": summary}


def baseline_comparison(
    current: dict[str, Any], baseline: dict[str, Any]
) -> dict[str, Any]:
    return {
        "normalized_scalar_count_equal": current["normalized_unicode_scalar_count"]
        == baseline.get("unicode_scalar_count"),
        "normalized_hash_equal": current["normalized_sha256_audit_only"]
        == baseline.get("normalized_sha256_audit_only"),
        "normalized_scalar_delta_current_minus_pinned": current[
            "normalized_unicode_scalar_count"
        ]
        - (baseline.get("unicode_scalar_count") or 0),
    }


def compare_book(
    book_id: str,
    source: Path,
    source_sha256: str,
    folio: Path,
    boko: Path,
    c2_row: dict[str, Any],
    temporary: Path,
) -> dict[str, Any]:
    output_paths = {
        "calibre": temporary / f"{book_id}.calibre.epub",
        "boko": temporary / f"{book_id}.boko.epub",
        "folioforge": temporary / f"{book_id}.folioforge.epub",
    }
    commands = {
        "calibre": ["ebook-convert", str(source), str(output_paths["calibre"])],
        "boko": [str(boko), "convert", str(source), str(output_paths["boko"])],
        "folioforge": [
            str(folio),
            "convert",
            str(source),
            "--to",
            "epub",
            "--output",
            str(output_paths["folioforge"]),
        ],
    }
    outputs = {
        name: run_converter(commands[name], output_paths[name])
        for name in ("calibre", "boko", "folioforge")
    }
    summaries = {
        name: result["summary"]
        for name, result in outputs.items()
        if result.get("status") == "success"
    }
    pairwise_results: dict[str, Any] = {}
    for left, right in (
        ("calibre", "boko"),
        ("calibre", "folioforge"),
        ("boko", "folioforge"),
    ):
        key = f"{left}_epub_vs_{right}_epub"
        if left in summaries and right in summaries:
            pairwise_results[key] = pairwise(summaries[left], summaries[right])
        else:
            pairwise_results[key] = {
                "comparable": False,
                "reason": "one_or_more_converters_did_not_produce_a_valid_epub",
            }
    calibre_json = c2_row["stages"]["calibre_oracle"]
    calibre_baseline = c2_row["stages"]["epub_visible_text"]
    if "calibre" in summaries:
        pairwise_results["calibre_json_content_vs_calibre_epub"] = {
            "comparable": True,
            "normalized_scalar_count_equal": summaries["calibre"][
                "normalized_unicode_scalar_count"
            ]
            == calibre_json["unicode_scalar_count"],
            "normalized_hash_equal": summaries["calibre"][
                "normalized_sha256_audit_only"
            ]
            == calibre_json["normalized_sha256_audit_only"],
        }
    else:
        pairwise_results["calibre_json_content_vs_calibre_epub"] = {
            "comparable": False,
            "reason": "calibre_did_not_produce_a_valid_epub",
        }
    if "folioforge" in summaries:
        pairwise_results["folioforge_epub_vs_pinned_c2_epub"] = baseline_comparison(
            summaries["folioforge"], calibre_baseline
        )
    else:
        pairwise_results["folioforge_epub_vs_pinned_c2_epub"] = {
            "comparable": False,
            "reason": "folioforge_did_not_produce_a_valid_epub",
        }
    for path in output_paths.values():
        path.unlink(missing_ok=True)
    return {
        "input_id": book_id,
        "source_sha256_audit_only": source_sha256,
        "outputs": outputs,
        "pairwise": pairwise_results,
    }


def run(args: argparse.Namespace) -> int:
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    c2_comparison = json.loads(args.comparison.read_text(encoding="utf-8"))
    books = manifest.get("books")
    c2_books = c2_comparison.get("books")
    if not isinstance(books, list) or len(books) != 82:
        raise ValueError("pinned anonymous corpus must contain 82 books")
    if not isinstance(c2_books, list) or len(c2_books) != 82:
        raise ValueError("pinned C2 comparison must contain 82 books")
    by_id = {book["id"]: book for book in books}
    c2_by_id = {book["input_id"]: book for book in c2_books}
    if set(by_id) != set(c2_by_id):
        raise ValueError("manifest and C2 comparison IDs did not reconcile")
    selected = list(by_id)
    if args.only:
        unknown = set(args.only) - set(selected)
        if unknown:
            raise ValueError("requested anonymous ID is not in the pinned corpus")
        selected = [book_id for book_id in selected if book_id in set(args.only)]
    sources = resolve_sources(args.book_root, books)
    if not shutil_which("ebook-convert"):
        raise ValueError("Calibre ebook-convert is not available")
    calibre_version = command_version(["calibre-debug", "--version"])
    boko_version = command_version([str(args.boko), "--version"])
    folio_version = command_version([str(args.folio), "--version"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    results = []
    with tempfile.TemporaryDirectory(
        prefix="folio-c2-three-way-", dir=str(args.temp_root) if args.temp_root else None
    ) as temporary:
        temporary_path = Path(temporary)
        for index, book_id in enumerate(selected, 1):
            print(f"[{index}/{len(selected)}] comparing EPUB outputs for {book_id}", flush=True)
            results.append(
                compare_book(
                    book_id,
                    sources[by_id[book_id]["source_sha256_audit_only"]],
                    by_id[book_id]["source_sha256_audit_only"],
                    args.folio,
                    args.boko,
                    c2_by_id[book_id],
                    temporary_path,
                )
            )
    success_counts = {
        name: sum(row["outputs"][name]["status"] == "success" for row in results)
        for name in ("calibre", "boko", "folioforge")
    }
    all_three = [
        row
        for row in results
        if all(row["outputs"][name]["status"] == "success" for name in ("calibre", "boko", "folioforge"))
    ]
    output = {
        "schema_version": 2,
        "scope": "all selected DRM-free KFX corpus inputs; conversion EPUBs are temporary and deleted",
        "comparison": "Calibre KFX Input / Bokō / FolioForge EPUB visible text in spine order",
        "primary": "raw Unicode scalar streams",
        "secondary": "CRLF/CR to LF, then NFC",
        "extractor": "shared neutral EPUB XHTML body/spine extractor",
        "toolchain": {
            "calibre_version": calibre_version,
            "kfx_input_version": args.kfx_input_version,
            "kfx_input_plugin_archive_sha256": args.kfx_input_plugin_sha256,
            "boko_version": boko_version,
            "boko_license": "GPL-3.0-or-later; external executable only",
            "folioforge_version": folio_version,
            "folioforge_binary_sha256_audit_only": file_sha256(args.folio),
        },
        "selected_book_count": len(results),
        "expected_corpus_count": 82,
        "converter_success_counts": success_counts,
        "books": results,
        "summary": {
            "all_three_success_count": len(all_three),
            "calibre_epub_equals_boko_epub_normalized_count": sum(
                row["pairwise"]["calibre_epub_vs_boko_epub"].get("normalized_hash_equal", False)
                for row in results
            ),
            "calibre_epub_equals_folioforge_epub_normalized_count": sum(
                row["pairwise"]["calibre_epub_vs_folioforge_epub"].get("normalized_hash_equal", False)
                for row in results
            ),
            "boko_epub_equals_folioforge_epub_normalized_count": sum(
                row["pairwise"]["boko_epub_vs_folioforge_epub"].get("normalized_hash_equal", False)
                for row in results
            ),
            "calibre_epub_equals_folioforge_epub_content_compacted_count": sum(
                row["pairwise"]["calibre_epub_vs_folioforge_epub"].get(
                    "content_compacted_hash_equal", False
                )
                for row in results
            ),
            "boko_epub_equals_folioforge_epub_content_compacted_count": sum(
                row["pairwise"]["boko_epub_vs_folioforge_epub"].get(
                    "content_compacted_hash_equal", False
                )
                for row in results
            ),
            "calibre_epub_equals_folioforge_epub_image_sequence_count": sum(
                row["pairwise"]["calibre_epub_vs_folioforge_epub"].get(
                    "image_sequence_hash_equal", False
                )
                for row in results
            ),
            "boko_epub_equals_folioforge_epub_image_sequence_count": sum(
                row["pairwise"]["boko_epub_vs_folioforge_epub"].get(
                    "image_sequence_hash_equal", False
                )
                for row in results
            ),
            "calibre_epub_equals_folioforge_epub_image_resource_multiset_count": sum(
                row["pairwise"]["calibre_epub_vs_folioforge_epub"].get(
                    "image_resource_multiset_equal", False
                )
                for row in results
            ),
            "boko_epub_equals_folioforge_epub_image_resource_multiset_count": sum(
                row["pairwise"]["boko_epub_vs_folioforge_epub"].get(
                    "image_resource_multiset_equal", False
                )
                for row in results
            ),
            "calibre_epub_equals_folioforge_epub_toc_label_sequence_count": sum(
                row["pairwise"]["calibre_epub_vs_folioforge_epub"].get(
                    "toc_label_sequence_hash_equal", False
                )
                for row in results
            ),
            "boko_epub_equals_folioforge_epub_toc_label_sequence_count": sum(
                row["pairwise"]["boko_epub_vs_folioforge_epub"].get(
                    "toc_label_sequence_hash_equal", False
                )
                for row in results
            ),
            "calibre_epub_equals_folioforge_epub_toc_depth_sequence_count": sum(
                row["pairwise"]["calibre_epub_vs_folioforge_epub"].get(
                    "toc_depth_sequence_hash_equal", False
                )
                for row in results
            ),
            "boko_epub_equals_folioforge_epub_toc_depth_sequence_count": sum(
                row["pairwise"]["boko_epub_vs_folioforge_epub"].get(
                    "toc_depth_sequence_hash_equal", False
                )
                for row in results
            ),
            "calibre_epub_equals_folioforge_epub_structure_count": sum(
                all(
                    row["pairwise"]["calibre_epub_vs_folioforge_epub"].get(
                        field, False
                    )
                    for field in (
                        "document_count_equal",
                        "paragraph_count_equal",
                        "heading_count_equal",
                        "image_like_count_equal",
                    )
                )
                for row in results
            ),
            "boko_epub_equals_folioforge_epub_structure_count": sum(
                all(
                    row["pairwise"]["boko_epub_vs_folioforge_epub"].get(
                        field, False
                    )
                    for field in (
                        "document_count_equal",
                        "paragraph_count_equal",
                        "heading_count_equal",
                        "image_like_count_equal",
                    )
                )
                for row in results
            ),
            "calibre_json_equals_calibre_epub_normalized_count": sum(
                row["pairwise"]["calibre_json_content_vs_calibre_epub"].get("normalized_hash_equal", False)
                for row in results
            ),
            "folioforge_epub_equals_pinned_c2_epub_normalized_count": sum(
                row["pairwise"]["folioforge_epub_vs_pinned_c2_epub"].get("normalized_hash_equal", False)
                for row in results
            ),
        },
    }
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote three-way EPUB comparison for {len(results)} inputs", flush=True)
    return 0


def shutil_which(command: str) -> str | None:
    """Small dependency-free equivalent of shutil.which for this runner."""
    import os

    for directory in os.environ.get("PATH", "").split(os.pathsep):
        candidate = Path(directory) / command
        if candidate.is_file() and candidate.stat().st_mode & 0o111:
            return str(candidate)
    return None


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--book-root", type=Path, required=True)
    result.add_argument("--manifest", type=Path, required=True)
    result.add_argument("--comparison", type=Path, required=True)
    result.add_argument("--folio", type=Path, required=True)
    result.add_argument("--boko", type=Path, required=True)
    result.add_argument("--output", type=Path, required=True)
    result.add_argument("--temp-root", type=Path)
    result.add_argument("--only", action="append", default=[])
    result.add_argument("--kfx-input-version", default="2.34.2")
    result.add_argument(
        "--kfx-input-plugin-sha256",
        default="338809c18e5f9bb721dc3570a64cf6f7add4a1711c8183e6179e90fcbadf3c1d",
    )
    return result


if __name__ == "__main__":
    try:
        raise SystemExit(run(parser().parse_args()))
    except (
        OSError,
        KeyError,
        TypeError,
        ValueError,
        RuntimeError,
        subprocess.SubprocessError,
        json.JSONDecodeError,
    ) as error:
        print(
            f"C2-R three-way output comparison failed ({type(error).__name__}); "
            "no source text or filename was emitted.",
            file=sys.stderr,
        )
        raise SystemExit(1)
