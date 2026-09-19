#!/usr/bin/env python3
"""FolioForge 0.1 architecture boundary checks.

This is intentionally small and dependency-free. Cargo metadata checks actual
workspace edges; source checks catch a client or semantic crate becoming a
second capability authority. It is a CI gate, not a replacement for review.
"""

from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]


def metadata() -> dict:
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        check=True,
        text=True,
        capture_output=True,
    )
    return json.loads(result.stdout)


def workspace_edges(data: dict) -> dict[str, set[str]]:
    workspace = set(data["workspace_members"])
    packages = {package["id"]: package for package in data["packages"]}
    workspace_names = {packages[package_id]["name"] for package_id in workspace}
    edges: dict[str, set[str]] = {}
    for package_id in workspace:
        package = packages[package_id]
        edges[package["name"]] = {
            dependency["name"]
            for dependency in package["dependencies"]
            if dependency.get("kind") in (None, "normal")
            and (
                dependency.get("package") in workspace
                or dependency["name"] in workspace_names
            )
        }
    return edges


def check_edges(edges: dict[str, set[str]], failures: list[str]) -> None:
    forbidden = {
        "folio-model": {name for name in edges if name.startswith("folio-") and name not in {"folio-model"}},
        "folio-normalize": {
            name for name in edges if name in {"folio-epub", "folio-kfx", "folio-kf7", "folio-kf8", "folio-docx", "folio-fb2", "folio-html", "folio-markdown"}
        },
        "folio-compat": {"folio-input", "folio-epub", "folio-kfx", "folio-kf7", "folio-kf8", "folio-docx"},
        "folio-epub": {"folio-kfx", "folio-docx"},
        "folio-kfx": {"folio-epub"},
        "folio-core": {"folio-library"},
        "folio-library": {
            "folio-epub",
            "folio-kfx",
            "folio-kf7",
            "folio-kf8",
            "folio-docx",
            "folio-fb2",
            "folio-html",
            "folio-markdown",
        },
    }
    for package, disallowed in forbidden.items():
        overlap = sorted(edges.get(package, set()) & disallowed)
        if overlap:
            failures.append(f"{package} has forbidden dependencies: {', '.join(overlap)}")


def source_text(relative: str) -> str:
    path = ROOT / relative
    return "\n".join(
        item.read_text(encoding="utf-8")
        for item in sorted(path.rglob("*.rs"))
        if item.is_file()
    )


def check_source_authority(failures: list[str]) -> None:
    for package in ("folio-model", "folio-normalize", "folio-compat"):
        text = source_text(f"crates/{package}/src")
        if re.search(r"(?:folio_kfx|folio_docx|folio_epub|DetectedFormat::)", text):
            failures.append(f"{package} contains source-format authority tokens")

    core_text = source_text("crates/folio-core/src")
    if re.search(r"folio_library|rusqlite", core_text):
        failures.append("folio-core contains Library/SQLite authority tokens")

    library_text = source_text("crates/folio-library/src")
    if re.search(r"(?:folio_kfx|folio_docx|folio_epub|folio_kf7|folio_kf8|folio_fb2|folio_html)", library_text):
        failures.append("folio-library contains concrete format-crate authority tokens")

    for client in ("crates/folio-ffi/src", "crates/folio-service/src", "macos/FolioForge"):
        path = ROOT / client
        if not path.exists():
            continue
        text = "\n".join(
            item.read_text(encoding="utf-8")
            for item in sorted(path.rglob("*.rs")) + sorted(path.rglob("*.swift"))
            if item.is_file()
        )
        if re.search(r"CapabilityProfile\s*\{", text):
            failures.append(f"{client} defines its own CapabilityProfile")
        if re.search(r"Feature::(?:Footnote|Ruby|Math|Conditional|FixedLayout)", text):
            failures.append(f"{client} contains target feature authority")


def main() -> int:
    failures: list[str] = []
    check_edges(workspace_edges(metadata()), failures)
    check_source_authority(failures)
    if failures:
        print("FolioForge 0.1 architecture check: FAIL")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print("FolioForge 0.1 architecture check: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
