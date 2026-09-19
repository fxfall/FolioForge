#!/usr/bin/env python3
"""Run the release conversion benchmark without writing into the repository."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import resource
import statistics
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def child_usage() -> tuple[float, int]:
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    rss = int(usage.ru_maxrss)
    if sys.platform != "darwin":
        rss *= 1024
    return usage.ru_utime + usage.ru_stime, rss


def time_command() -> list[str]:
    """Return a platform time wrapper that reports one child process's RSS."""
    time_binary = Path("/usr/bin/time")
    if not time_binary.is_file():
        return []
    return [str(time_binary), "-l" if sys.platform == "darwin" else "-v"]


def peak_rss_from_time(stderr: str) -> int | None:
    if sys.platform == "darwin":
        match = re.search(r"^\s*(\d+)\s+maximum resident set size", stderr, re.MULTILINE)
        return int(match.group(1)) if match else None
    match = re.search(r"Maximum resident set size \(kbytes\):\s+(\d+)", stderr)
    return int(match.group(1)) * 1024 if match else None


def read_cases(manifest: Path) -> list[tuple[str, Path]]:
    cases = []
    for line in manifest.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        name, input_path = line.split("\t", 1)
        cases.append((name.strip(), (ROOT / input_path.strip()).resolve()))
    return cases


def target_extension(target: str) -> str:
    return {"epub": "epub", "kf7": "mobi", "kf8": "azw3", "kfx": "kfx"}.get(
        target, target
    )


def run_one(binary: Path, input_path: Path, target: str, output: Path, env: dict[str, str]) -> dict:
    before_cpu, before_rss = child_usage()
    started = time.perf_counter()
    command = time_command() + [
        str(binary),
        "convert",
        str(input_path),
        "--to",
        target,
        "--metrics",
        "--output",
        str(output),
    ]
    result = subprocess.run(
        command,
        cwd=ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    wall_ms = (time.perf_counter() - started) * 1000.0
    after_cpu, after_rss = child_usage()
    if result.returncode != 0:
        raise RuntimeError(
            f"benchmark conversion failed for {input_path.name} -> {target}: "
            f"{result.stderr[-400:]}"
        )
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"benchmark report was not JSON: {error}") from error
    output_size = output.stat().st_size if output.exists() else 0
    metrics = report.get("metrics") or {}
    return {
        "wall_ms": wall_ms,
        "cpu_ms": max(0.0, (after_cpu - before_cpu) * 1000.0),
        "peak_rss_bytes": peak_rss_from_time(result.stderr)
        or max(0, after_rss - before_rss),
        "input_bytes": input_path.stat().st_size,
        "output_bytes": output_size,
        "resource_bytes": metrics.get("resource_bytes", 0),
        "stage_ms": metrics.get("stage_ms", {}),
    }


def median_measurements(measurements: list[dict]) -> dict:
    keys = ("wall_ms", "cpu_ms", "peak_rss_bytes", "input_bytes", "output_bytes", "resource_bytes")
    result = {key: statistics.median(item[key] for item in measurements) for key in keys}
    stage_names = sorted({name for item in measurements for name in item["stage_ms"]})
    result["stage_ms"] = {
        name: statistics.median(item["stage_ms"].get(name, 0) for item in measurements)
        for name in stage_names
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, default=ROOT / "bench/canonical/manifest.tsv")
    parser.add_argument("--targets", nargs="+", default=["epub"])
    parser.add_argument("--runs", type=int, default=5)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")

    binary = args.binary.resolve()
    output_root = args.output_root.resolve()
    output_root.mkdir(parents=True, exist_ok=True)
    output_dir = output_root / "outputs"
    output_dir.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env["TMPDIR"] = str(output_root / "tmp")
    env["FOLIOFORGE_TEMP_ROOT"] = str(output_root / "runtime")
    Path(env["TMPDIR"]).mkdir(parents=True, exist_ok=True)
    Path(env["FOLIOFORGE_TEMP_ROOT"]).mkdir(parents=True, exist_ok=True)

    cases = read_cases(args.manifest)
    results = []
    for name, input_path in cases:
        if not input_path.is_file():
            raise SystemExit(f"benchmark input is missing: {input_path}")
        digest = hashlib.sha256(input_path.read_bytes()).hexdigest()
        for target in args.targets:
            case_dir = output_dir / name / target
            case_dir.mkdir(parents=True, exist_ok=True)
            extension = target_extension(target)
            warmup_output = case_dir / f"warmup.{extension}"
            run_one(binary, input_path, target, warmup_output, env)
            measurements = []
            for index in range(args.runs):
                output = case_dir / f"run-{index + 1}.{extension}"
                measurements.append(run_one(binary, input_path, target, output, env))
            results.append(
                {
                    "name": name,
                    "target": target,
                    "input": str(input_path.relative_to(ROOT)),
                    "input_sha256": digest,
                    "runs": args.runs,
                    "median": median_measurements(measurements),
                }
            )

    report = {
        "phase": "7",
        "profile": "release",
        "machine": {
            "platform": platform.platform(),
            "processor": platform.processor(),
            "python": platform.python_version(),
        },
        "binary": str(binary),
        "manifest": str(args.manifest.resolve()),
        "warmup_runs": 1,
        "measured_runs": args.runs,
        "results": results,
    }
    (output_root / "benchmark-report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(output_root / "benchmark-report.json")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
