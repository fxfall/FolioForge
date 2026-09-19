#!/usr/bin/env python3
"""Fail the public repository clean gate before a release or push."""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MAX_TRACKED_BYTES = 50 * 1024 * 1024
AUDIT_PATH = Path("tools/release/repository-audit.py")


def tracked_paths() -> list[Path]:
    result = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "-z"],
        check=True,
        stdout=subprocess.PIPE,
    )
    return [Path(value) for value in result.stdout.decode().split("\0") if value]


def working_tree_errors() -> list[str]:
    result = subprocess.run(
        ["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=normal"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return [line for line in result.stdout.splitlines() if line]


def text_bytes(path: Path) -> bytes | None:
    try:
        data = path.read_bytes()
    except OSError:
        return None
    if b"\0" in data[:8192]:
        return None
    return data


def main() -> int:
    try:
        paths = tracked_paths()
    except subprocess.CalledProcessError as error:
        print(f"repository-audit: git ls-files failed: {error}", file=sys.stderr)
        return 2

    failures: list[str] = []
    private_path_markers = (
        "tests-private/",
        "private-corpus/",
        "/target/",
        "/.build/",
        "/DerivedData/",
        "/.swiftpm/",
        "/dist/",
        ".folioforge-",
    )
    local_prefixes = (
        "/" + "Volumes/Repositories/",
        "/" + "Users/",
        "/" + "private/var/",
        "/" + "var/folders/",
    )
    secret_tokens = (
        "gh" + "p_",
        "github_pat" + "_",
        "xoxb" + "-",
        "AKIA",
        "sk" + "-",
    )
    credential_assignment = re.compile(
        r"(?i)(api[_-]?key|access[_-]?token|client[_-]?secret)\s*[:=]\s*['\"][A-Za-z0-9]"
    )

    for relative in paths:
        normalized = relative.as_posix()
        path = ROOT / relative
        if any(marker in f"/{normalized}" for marker in private_path_markers):
            failures.append(f"private/generated path is tracked: {normalized}")
        try:
            size = path.stat().st_size
        except OSError as error:
            failures.append(f"tracked file is unreadable: {normalized}: {error}")
            continue
        if size > MAX_TRACKED_BYTES:
            failures.append(f"tracked file exceeds 50 MiB: {normalized} ({size} bytes)")
        data = text_bytes(path)
        if data is None or relative == AUDIT_PATH:
            continue
        text = data.decode("utf-8", errors="replace")
        if any(prefix in text for prefix in local_prefixes):
            failures.append(f"machine-specific absolute path: {normalized}")
        if any(token in text for token in secret_tokens) or credential_assignment.search(text):
            failures.append(f"possible credential/token material: {normalized}")

    try:
        status = working_tree_errors()
    except subprocess.CalledProcessError as error:
        print(f"repository-audit: git status failed: {error}", file=sys.stderr)
        return 2
    if status:
        failures.extend(f"working tree is not clean: {line}" for line in status)

    if failures:
        print("repository-audit: FAIL", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(f"repository-audit: PASS ({len(paths)} tracked paths)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
