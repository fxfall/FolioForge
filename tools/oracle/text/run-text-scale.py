#!/usr/bin/env python3
"""Run TXT size checks without leaving fixtures in the repository.

The harness measures the public CLI's analysis and EPUB conversion paths.  It
generates deterministic UTF-8 novel-like input in a temporary directory, so a
run does not depend on copyrighted books or on a checked-in large artifact.
Preview is an API/FFI operation in FolioForge; its client-facing smoke remains
covered by the Rust/service tests rather than serializing a 100 MiB HTML bundle
to this report.
"""

from __future__ import annotations

import argparse
import json
import resource
import subprocess
import sys
import tempfile
import time
from pathlib import Path


NOVEL_PARAGRAPH = (
    "这是用于性能回归的确定性中文、English and العربية paragraph. "
    "它包含足够长的连续正文，帮助检测换行、章节分析与转换过程的内存行为。 "
    "The same semantic paragraph is repeated inside each chapter so the input "
    "remains a novel-like plain-text book instead of a binary filler。\n\n"
)


def command_prefix(binary: str | None) -> list[str]:
    if binary:
        return [str(Path(binary).expanduser())]
    return ["cargo", "run", "--quiet", "-p", "folio-cli", "--"]


def write_fixture(book_path: Path, size_bytes: int) -> int:
    written = 0
    chapter = 1
    with book_path.open("wb") as stream:
        while written < size_bytes:
            chapter_text = (
                f"第{chapter}章 风从海上来\n\n" + NOVEL_PARAGRAPH * 1_200
            ).encode("utf-8")
            remaining = size_bytes - written
            if remaining >= len(chapter_text):
                value = chapter_text
                stream.write(value)
                written += len(value)
            else:
                value = chapter_text[:remaining]
                while value:
                    try:
                        value.decode("utf-8")
                        break
                    except UnicodeDecodeError:
                        value = value[:-1]
                stream.write(value)
                stream.write(b" " * (remaining - len(value)))
                written += remaining
            chapter += 1
    return written


def run_command(
    command: list[str], repo_root: Path, output_path: Path | None = None
) -> dict[str, object]:
    started = time.perf_counter()
    result = subprocess.run(
        command,
        stdout=subprocess.DEVNULL if output_path is not None else subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=repo_root,
        check=False,
    )
    maxrss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    if sys.platform == "darwin":
        maxrss //= 1024
    return {
        "exit_code": result.returncode,
        "seconds": round(time.perf_counter() - started, 3),
        "stderr_bytes": len(result.stderr),
        "maxrss_kib": int(maxrss),
        "stderr_tail": result.stderr.decode("utf-8", errors="replace")[-500:],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--repo",
        type=Path,
        default=Path(__file__).resolve().parents[3],
        help="FolioForge workspace root",
    )
    parser.add_argument("--binary", help="built folio binary; defaults to cargo run")
    parser.add_argument(
        "--sizes",
        default="10,50,100",
        help="comma-separated sizes in MiB (default: 10,50,100)",
    )
    parser.add_argument(
        "--skip-convert", action="store_true", help="only measure analysis"
    )
    parser.add_argument(
        "--targets",
        default="epub,kf7,kf8",
        help="comma-separated active non-KFX targets (default: epub,kf7,kf8)",
    )
    args = parser.parse_args()
    sizes = [int(value) for value in args.sizes.split(",") if value.strip()]
    targets = [value.strip().lower() for value in args.targets.split(",") if value.strip()]
    extensions = {"epub": "epub", "kf7": "mobi", "kf8": "azw3"}
    unsupported_targets = [value for value in targets if value not in extensions]
    if unsupported_targets:
        parser.error(f"unsupported active scale target(s): {', '.join(unsupported_targets)}")
    prefix = command_prefix(args.binary)
    results: list[dict[str, object]] = []

    with tempfile.TemporaryDirectory(prefix="folioforge-text-scale-") as temp_dir:
        temp_root = Path(temp_dir)
        for size_mib in sizes:
            book_path = temp_root / f"scale-{size_mib}m.txt"
            written = write_fixture(book_path, size_mib * 1024 * 1024)
            analysis = run_command(
                prefix
                + [
                    "analyze",
                    str(book_path),
                    "--to",
                    "epub",
                    "--mode",
                    "compatible",
                    "--text-mode",
                    "novel",
                ],
                args.repo,
            )
            conversion = None
            if not args.skip_convert:
                conversion = {}
                for target in targets:
                    output_path = temp_root / f"scale-{size_mib}m.{extensions[target]}"
                    result = run_command(
                        prefix
                        + [
                            "convert",
                            str(book_path),
                            "--to",
                            target,
                            "--mode",
                            "compatible",
                            "--text-mode",
                            "novel",
                            "--output",
                            str(output_path),
                        ],
                        args.repo,
                        output_path=output_path,
                    )
                    result["output_bytes"] = output_path.stat().st_size if output_path.exists() else 0
                    conversion[target] = result
            results.append(
                {
                    "size_mib": size_mib,
                    "input_bytes": written,
                    "analysis": analysis,
                    "conversion": conversion,
                }
            )

    print(json.dumps({"tool": "folioforge-text-scale", "results": results}, ensure_ascii=False, indent=2))
    return 0 if all(
        item["analysis"]["exit_code"] == 0
        and (
            item["conversion"] is None
            or all(result["exit_code"] == 0 for result in item["conversion"].values())
        )
        for item in results
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
