#!/usr/bin/env python3
"""Build and audit the controlled KFX semantic probe corpus.

This is a development tool. It deliberately keeps Kindle Previewer, Calibre,
and Bōkō outside FolioForge runtime crates. All generated artifacts must be
placed under an explicit external artifact root supplied by the caller.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import zipfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable
from xml.etree import ElementTree as ET


REPO_ROOT = Path(__file__).resolve().parents[2]
PROBE_ROOT = REPO_ROOT / "tests" / "semantic-probes" / "kfx"
DEFAULT_REVIEW_FILE = PROBE_ROOT / "semantic-review.json"
DEFAULT_ARTIFACT_ROOT = Path(os.environ.get("FOLIOFORGE_KFX_PROBE_ROOT", ".folioforge-kfx-probe"))
DEFAULT_FOLIO = REPO_ROOT / "target" / "debug" / "folio"
DEFAULT_PREVIEWER = Path(os.environ.get("KINDLE_PREVIEWER", "Kindle Previewer 3"))
DEFAULT_CALIBRE_CONFIG = Path(os.environ.get("CALIBRE_CONFIG_DIRECTORY", ".folioforge-calibre-config"))
DEFAULT_CALIBRE_DEBUG = Path(shutil.which("calibre-debug") or "calibre-debug")

ET.register_namespace("", "http://www.w3.org/1999/xhtml")
ET.register_namespace("epub", "http://www.idpf.org/2007/ops")
ET.register_namespace("dc", "http://purl.org/dc/elements/1.1/")

FEATURE_PREFIXES = {
    "footnote": "FN-",
    "math": "MATH-",
    "ruby": "RUBY-",
    "figure": "FIG-",
    "conditional": "COND-",
    "illustrated": "ILL-",
    "render-inline": "INLINE-",
}


class ProbeError(RuntimeError):
    pass


COVERAGE_STATES = (
    "Encountered",
    "Recognized",
    "Mapped",
    "Approximated",
    "Unsupported",
    "Unknown",
    "NotExercised",
)


@dataclass(frozen=True)
class ProbeExpectation:
    """Source-level semantic contract; never a KFX field-ID contract."""

    probe_id: str
    feature: str
    source_semantics: dict[str, Any]
    acceptance: dict[str, Any]
    forbidden_inference: tuple[str, ...]

    @classmethod
    def from_manifest(cls, manifest: dict[str, Any]) -> "ProbeExpectation":
        return cls(
            probe_id=str(manifest["id"]),
            feature=str(manifest["feature"]),
            source_semantics=dict(manifest["source_semantics"]),
            acceptance=dict(manifest.get("acceptance", {})),
            forbidden_inference=tuple(
                str(value) for value in manifest.get("forbidden_inference", [])
            ),
        )

    def as_dict(self) -> dict[str, Any]:
        return {
            "probe_id": self.probe_id,
            "feature": self.feature,
            "source_semantics": self.source_semantics,
            "acceptance": self.acceptance,
            "forbidden_inference": list(self.forbidden_inference),
        }


def now_utc() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_manifest(fixture: Path) -> dict[str, Any]:
    path = fixture / "manifest.json"
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ProbeError(f"cannot read {path}: {exc}") from exc
    if not isinstance(manifest, dict):
        raise ProbeError(f"{path} must contain a JSON object")
    required = ("schema_version", "id", "feature", "source_file", "source_semantics")
    missing = [key for key in required if key not in manifest]
    if missing:
        raise ProbeError(f"{path} is missing: {', '.join(missing)}")
    probe_id = manifest["id"]
    feature = manifest["feature"]
    if not isinstance(probe_id, str) or not re.fullmatch(r"[A-Z]+-[0-9]{2,3}", probe_id):
        raise ProbeError(f"{path}: id must look like FN-01 or MATH-01")
    if feature not in FEATURE_PREFIXES:
        raise ProbeError(f"{path}: unsupported feature {feature!r}")
    if not probe_id.startswith(FEATURE_PREFIXES[feature]):
        raise ProbeError(f"{path}: id {probe_id} does not match feature {feature}")
    if not isinstance(manifest["source_semantics"], dict):
        raise ProbeError(f"{path}: source_semantics must be an object")
    # Numeric KFX identifiers in a source manifest are almost always a sign
    # that the observation layer has leaked into the source assertion.
    raw = json.dumps(manifest, ensure_ascii=False)
    if re.search(r"\$\d{2,4}|fragment[_ -]?type|symbol[_ -]?id|kfx[_ -]?(field|symbol)", raw, re.I):
        raise ProbeError(
            f"{path}: source semantics must not contain KFX field/fragment/symbol identifiers"
        )
    source = fixture / str(manifest["source_file"])
    if not source.is_file():
        raise ProbeError(f"{path}: source file does not exist: {source.name}")
    readme = fixture / "README.md"
    if not readme.is_file():
        raise ProbeError(f"{path}: fixture README.md is missing")
    return manifest


def fixtures() -> list[tuple[Path, dict[str, Any]]]:
    found: list[tuple[Path, dict[str, Any]]] = []
    for manifest_path in sorted(PROBE_ROOT.glob("*/manifest.json")):
        fixture = manifest_path.parent
        found.append((fixture, load_manifest(fixture)))
    if not found:
        raise ProbeError(f"no semantic probes found under {PROBE_ROOT}")
    ids = [manifest["id"] for _, manifest in found]
    if len(ids) != len(set(ids)):
        raise ProbeError("semantic probe IDs must be unique")
    return found


def load_semantic_review(path: Path) -> dict[str, dict[str, Any]]:
    if not path.is_file():
        return {}
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ProbeError(f"cannot read semantic review {path}: {exc}") from exc
    if not isinstance(raw, dict) or not isinstance(raw.get("probes"), dict):
        raise ProbeError(f"{path} must contain a top-level probes object")
    review: dict[str, dict[str, Any]] = {}
    valid_ids = {manifest["id"] for _, manifest in fixtures()}
    for probe_id, entry in raw["probes"].items():
        if probe_id not in valid_ids:
            raise ProbeError(f"{path}: review contains unknown probe {probe_id}")
        if not isinstance(entry, dict):
            raise ProbeError(f"{path}: review for {probe_id} must be an object")
        state = entry.get("state")
        if state not in COVERAGE_STATES:
            raise ProbeError(f"{path}: {probe_id} has invalid state {state!r}")
        if not isinstance(entry.get("reason"), str) or not entry["reason"].strip():
            raise ProbeError(f"{path}: {probe_id} requires a non-empty reason")
        review[probe_id] = entry
    return review


def fixture_for(probe_id: str) -> tuple[Path, dict[str, Any]]:
    for fixture, manifest in fixtures():
        if manifest["id"] == probe_id:
            return fixture, manifest
    raise ProbeError(f"unknown semantic probe {probe_id}")


def media_type(path: Path) -> str:
    return {
        ".svg": "image/svg+xml",
        ".png": "image/png",
        ".jpg": "image/jpeg",
        ".jpeg": "image/jpeg",
        ".gif": "image/gif",
        ".css": "text/css",
        ".xhtml": "application/xhtml+xml",
    }.get(path.suffix.lower(), "application/octet-stream")


def safe_id(path: str) -> str:
    value = re.sub(r"[^A-Za-z0-9]+", "-", path).strip("-").lower()
    return "asset-" + (value or "resource")


def source_files(fixture: Path, manifest: dict[str, Any]) -> list[tuple[str, bytes]]:
    source_name = str(manifest["source_file"])
    source = fixture / source_name
    xhtml = source.read_bytes()
    try:
        root = ET.fromstring(xhtml)
    except ET.ParseError as exc:
        raise ProbeError(f"{manifest['id']}: source XHTML is not well-formed XML: {exc}") from exc
    if root.tag.rsplit("}", 1)[-1] != "html":
        raise ProbeError(f"{manifest['id']}: source root must be html")
    title = str(manifest.get("title", manifest["id"]))
    language = str(manifest.get("language", "en"))
    uid = "urn:folioforge:semantic-probe:" + str(manifest["id"]).lower()
    container = (
        '<?xml version="1.0" encoding="UTF-8"?>'
        '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">'
        '<rootfiles><rootfile full-path="OEBPS/content.opf" '
        'media-type="application/oebps-package+xml"/></rootfiles></container>'
    ).encode("utf-8")
    nav = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<html xmlns="http://www.w3.org/1999/xhtml" '
        'xmlns:epub="http://www.idpf.org/2007/ops" lang="%s">'
        '<head><title>%s</title></head><body><nav epub:type="toc" id="toc">'
        '<h1>Contents</h1><ol><li><a href="chapter.xhtml">%s</a></li></ol>'
        '</nav></body></html>'
    ) % (language, title, title)
    ncx = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">'
        '<head><meta name="dtb:uid" content="%s"/></head>'
        '<docTitle><text>%s</text></docTitle><navMap>'
        '<navPoint id="navpoint-1" playOrder="1"><navLabel><text>%s</text></navLabel>'
        '<content src="chapter.xhtml"/></navPoint></navMap></ncx>'
    ) % (uid, title, title)
    opf_items = [
        '<item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>',
        '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>',
        '<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>',
        '<item id="css" href="styles.css" media-type="text/css"/>',
    ]
    asset_pairs: list[tuple[str, bytes]] = []
    assets = fixture / "assets"
    if assets.is_dir():
        for path in sorted(item for item in assets.rglob("*") if item.is_file()):
            relative = path.relative_to(assets).as_posix()
            href = "assets/" + relative
            asset_pairs.append((href, path.read_bytes()))
            opf_items.append(
                '<item id="%s" href="%s" media-type="%s"/>'
                % (safe_id(relative), href, media_type(path))
            )
    opf = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<package xmlns="http://www.idpf.org/2007/opf" version="3.0" '
        'unique-identifier="book-id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">'
        '<dc:identifier id="book-id">%s</dc:identifier><dc:title>%s</dc:title>'
        '<dc:language>%s</dc:language></metadata><manifest>%s</manifest>'
        '<spine toc="ncx"><itemref idref="chapter"/></spine></package>'
    ) % (uid, title, language, "".join(opf_items))
    css = (
        "html, body { margin: 0; padding: 0; }\n"
        "body { font-family: serif; line-height: 1.45; }\n"
        "figure { break-inside: avoid; }\n"
        ".probe-label { font-weight: 700; }\n"
    ).encode("utf-8")
    return [
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container),
        ("OEBPS/content.opf", opf.encode("utf-8")),
        ("OEBPS/nav.xhtml", nav.encode("utf-8")),
        ("OEBPS/toc.ncx", ncx.encode("utf-8")),
        ("OEBPS/chapter.xhtml", xhtml),
        ("OEBPS/styles.css", css),
        *[("OEBPS/" + href, data) for href, data in asset_pairs],
    ]


def package_source(fixture: Path, manifest: dict[str, Any], output: Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_STORED) as archive:
        for name, data in source_files(fixture, manifest):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            info.external_attr = 0o644 << 16
            archive.writestr(info, data)


def validate_source_semantics(fixture: Path, manifest: dict[str, Any]) -> dict[str, Any]:
    expectation = ProbeExpectation.from_manifest(manifest)
    source = fixture / str(manifest["source_file"])
    try:
        root = ET.fromstring(source.read_bytes())
    except ET.ParseError as exc:
        raise ProbeError(f"{manifest['id']}: cannot inspect source semantics: {exc}") from exc
    epub_type = "{http://www.idpf.org/2007/ops}type"
    elements = list(root.iter())
    semantic_markers: dict[str, int] = {}
    for element in elements:
        value = element.attrib.get(epub_type, "")
        for marker in value.split():
            semantic_markers[marker] = semantic_markers.get(marker, 0) + 1
    required_markers = manifest.get("required_epub_types", [])
    missing = [marker for marker in required_markers if semantic_markers.get(marker, 0) == 0]
    if missing:
        raise ProbeError(f"{manifest['id']}: missing required EPUB semantics: {', '.join(missing)}")
    return {
        "id": manifest["id"],
        "feature": manifest["feature"],
        "source_file": str(manifest["source_file"]),
        "source_sha256": sha256(source),
        "source_semantic_markers": dict(sorted(semantic_markers.items())),
        "probe_expectation": expectation.as_dict(),
        "coverage": {
            "feature": expectation.feature,
            "state": "NotExercised",
            "reason": "source semantics validated; no generated KFX has been reviewed",
        },
    }


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def command_output(path: Path, command: list[str], env: dict[str, str], cwd: Path | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as log:
        log.write("$ " + " ".join(repr(part) for part in command) + "\n")
        log.flush()
        process = subprocess.run(
            command,
            cwd=cwd,
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            check=False,
        )
        log.write(f"\n[exit_code={process.returncode}]\n")
    if process.returncode != 0:
        raise ProbeError(f"command failed ({process.returncode}); see {path}")


def executable(path: Path, name: str) -> Path:
    candidate = path if path else Path(shutil.which(name) or "")
    if not candidate.is_file():
        raise ProbeError(f"{name} was not found at {candidate}")
    return candidate


def kp3_command(previewer: Path, epub: Path, output: Path) -> list[str]:
    command = [str(previewer), str(epub), "-convert", "-output", str(output), "-locale", "en"]
    file_result = subprocess.run(["file", str(previewer)], capture_output=True, text=True, check=False)
    if platform.machine() in {"arm64", "aarch64"} and "x86_64" in file_result.stdout:
        return ["arch", "-x86_64", *command]
    return command


def one_kpf(directory: Path) -> Path:
    paths = sorted(directory.rglob("*.kpf"))
    if not paths:
        raise ProbeError(f"Kindle Previewer produced no .kpf under {directory}")
    if len(paths) > 1:
        # KP3 may keep a diagnostic copy. Prefer the top-level deterministic
        # result, but preserve every file in the artifact directory.
        top_level = [path for path in paths if path.parent == directory]
        if len(top_level) == 1:
            return top_level[0]
    return paths[0]


def build_probe(
    fixture: Path,
    manifest: dict[str, Any],
    artifact_root: Path,
    folio: Path,
    previewer: Path,
    calibre_config: Path,
    calibre_debug: Path,
) -> Path:
    probe_id = str(manifest["id"])
    run_root = artifact_root / "semantic-probes" / probe_id
    run_root.mkdir(parents=True, exist_ok=True)
    source_epub = run_root / "source.epub"
    source_audit = validate_source_semantics(fixture, manifest)
    package_source(fixture, manifest, source_epub)
    source_audit["packaged_epub_sha256"] = sha256(source_epub)
    write_json(run_root / "source-semantics.json", source_audit)

    env = os.environ.copy()
    env["CALIBRE_CONFIG_DIRECTORY"] = str(calibre_config)
    temp_root = run_root / "tmp"
    temp_root.mkdir(parents=True, exist_ok=True)
    # Calibre's persistent temporary directories do not consistently honor
    # TMPDIR. Keep its own temp root beside the probe artifacts as well, so a
    # probe never silently writes transient files to the system volume.
    calibre_temp_root = run_root / "calibre-tmp"
    calibre_temp_root.mkdir(parents=True, exist_ok=True)
    env["TMPDIR"] = str(temp_root)
    env["TEMP"] = str(temp_root)
    env["TMP"] = str(temp_root)
    env["CALIBRE_TEMP_DIR"] = str(calibre_temp_root)

    command_output(
        run_root / "folio-validate.log",
        [str(folio), "validate", str(source_epub)],
        env,
    )
    preview_root = run_root / "kp3"
    preview_root.mkdir(parents=True, exist_ok=True)
    command_output(
        run_root / "kp3.log",
        kp3_command(previewer, source_epub, preview_root),
        env,
    )
    try:
        kpf = one_kpf(preview_root)
    except ProbeError as exc:
        # KP3 can return exit code 0 while reporting "Not Supported" in its
        # Summary_Log.csv. Preserve that distinction as a machine-readable
        # blocked result instead of making a later run look like an absent
        # fixture or a semantic conclusion.
        blocked = {
            "schema_version": 1,
            "probe_id": probe_id,
            "feature": manifest["feature"],
            "created_utc": now_utc(),
            "status": "blocked_toolchain",
            "blocker": "kindle_previewer_no_kpf",
            "error": str(exc),
            "paths": {
                "source_epub": str(source_epub),
                "source_semantics": str(run_root / "source-semantics.json"),
                "kp3_log": str(run_root / "kp3.log"),
                "kp3_summary": str(preview_root / "Summary_Log.csv"),
            },
            "sha256_audit_only": {"source_epub": sha256(source_epub)},
            "source_semantics": manifest["source_semantics"],
            "probe_expectation": source_audit["probe_expectation"],
            "coverage": {
                "feature": manifest["feature"],
                "state": "NotExercised",
                "reason": "KP3 did not produce KPF/KFX",
            },
        }
        write_json(run_root / "result.json", blocked)
        raise ProbeError(f"{exc}; wrote blocked result {run_root / 'result.json'}") from exc
    kfx = run_root / "generated.kfx"
    command_output(
        run_root / "kfx-output.log",
        [str(calibre_debug), "-r", "KFX Output", "--", str(kpf), str(kfx)],
        env,
    )
    if not kfx.is_file():
        raise ProbeError(f"KFX Output reported success but did not create {kfx}")

    for label, args in (
        ("folio-fidelity-audit", ["inspect", str(kfx), "--kfx-fidelity-audit"]),
        ("folio-text-event-audit", ["inspect", str(kfx), "--kfx-text-event-audit"]),
        ("folio-semantic-evidence", ["inspect", str(kfx), "--kfx-semantic-evidence"]),
        ("folio-native-report", ["inspect", str(kfx), "--ion"]),
    ):
        output = run_root / f"{label}.json"
        command_output(run_root / f"{label}.log", [str(folio), *args], env)
        # The CLI output is already JSON; copying it through a parser makes
        # malformed diagnostics fail the probe run rather than hide in logs.
        try:
            output.write_text(
                (run_root / f"{label}.log").read_text(encoding="utf-8").split("\n", 1)[1]
                .rsplit("\n[exit_code=0]\n", 1)[0]
                .strip()
                + "\n",
                encoding="utf-8",
            )
            json.loads(output.read_text(encoding="utf-8"))
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            raise ProbeError(f"{probe_id}: {label} did not produce valid JSON: {exc}") from exc

    calibre_epub = run_root / "calibre-input.epub"
    calibre_json = run_root / "calibre-input.json"
    command_output(
        run_root / "calibre-input-epub.log",
        [str(calibre_debug), "-r", "KFX Input", "--", "-e", str(kfx), str(calibre_epub)],
        env,
    )
    command_output(
        run_root / "calibre-input-json.log",
        [str(calibre_debug), "-r", "KFX Input", "--", "-j", str(kfx), str(calibre_json)],
        env,
    )
    if not calibre_epub.is_file() or not calibre_json.is_file():
        raise ProbeError(f"{probe_id}: Calibre KFX Input did not produce EPUB and JSON outputs")

    ff_epub = run_root / "folioforge.epub"
    ff_kf8 = run_root / "folioforge.kf8"
    command_output(
        run_root / "folioforge-epub.log",
        [str(folio), "convert", str(kfx), "--to", "epub", "--mode", "compatible", "--output", str(ff_epub)],
        env,
    )
    command_output(
        run_root / "folioforge-kf8.log",
        [str(folio), "convert", str(kfx), "--to", "kf8", "--mode", "compatible", "--output", str(ff_kf8)],
        env,
    )
    for path in (ff_epub, ff_kf8):
        if not path.is_file():
            raise ProbeError(f"{probe_id}: FolioForge did not produce {path.name}")

    result = {
        "schema_version": 1,
        "probe_id": probe_id,
        "feature": manifest["feature"],
        "created_utc": now_utc(),
        "evidence_order": [
            "source_epub",
            "kindle_previewer_kpf",
            "original_kfx_structural_audit",
            "calibre_kfx_input_reference",
            "folioforge_outputs",
        ],
        "paths": {
            "source_epub": str(source_epub),
            "kpf": str(kpf),
            "kfx": str(kfx),
            "source_semantics": str(run_root / "source-semantics.json"),
            "folio_semantic_evidence": str(run_root / "folio-semantic-evidence.json"),
            "calibre_epub": str(calibre_epub),
            "calibre_json": str(calibre_json),
            "folioforge_epub": str(ff_epub),
            "folioforge_kf8": str(ff_kf8),
        },
        "sha256_audit_only": {
            "source_epub": sha256(source_epub),
            "kpf": sha256(kpf),
            "kfx": sha256(kfx),
            "calibre_epub": sha256(calibre_epub),
            "calibre_json": sha256(calibre_json),
            "folioforge_epub": sha256(ff_epub),
            "folioforge_kf8": sha256(ff_kf8),
        },
        "source_semantics": manifest["source_semantics"],
        "probe_expectation": source_audit["probe_expectation"],
        "coverage": {
            "feature": manifest["feature"],
            "state": "Encountered",
            "reason": "KFX generated; raw structural evidence still requires semantic review",
            "allowed_states": list(COVERAGE_STATES),
        },
        "acceptance": manifest.get("acceptance", {}),
        "status": "generated_requires_raw_kfx_review",
    }
    write_json(run_root / "result.json", result)
    return run_root


def list_command() -> int:
    rows = []
    for fixture, manifest in fixtures():
        rows.append(
            {
                "id": manifest["id"],
                "feature": manifest["feature"],
                "fixture": fixture.name,
                "title": manifest.get("title", manifest["id"]),
            }
        )
    print(json.dumps(rows, ensure_ascii=False, indent=2))
    return 0


def evidence_command(args: argparse.Namespace) -> int:
    folio = executable(args.folio, "folio")
    source = args.input
    if source.is_file():
        paths = [source]
    elif source.is_dir():
        paths = sorted(path for path in source.rglob("*.kfx") if path.is_file())
    else:
        raise ProbeError(f"evidence input does not exist: {source}")
    if not paths:
        raise ProbeError(f"evidence input contains no .kfx files: {source}")
    reports = []
    aggregate_fields: dict[str, int] = {}
    aggregate_fragments: dict[str, int] = {}
    aggregate_annotations: dict[str, int] = {}
    aggregate_conditional_fields: dict[str, int] = {}
    aggregate_render_inline_fields: dict[str, int] = {}
    for index, path in enumerate(paths):
        process = subprocess.run(
            [str(folio), "inspect", str(path), "--kfx-semantic-evidence"],
            capture_output=True,
            text=True,
            check=False,
        )
        if process.returncode != 0:
            raise ProbeError(f"semantic evidence failed for input {index}: {process.stderr.strip()}")
        try:
            report = json.loads(process.stdout)
        except json.JSONDecodeError as exc:
            raise ProbeError(f"semantic evidence returned invalid JSON for input {index}: {exc}") from exc
        for key, value in report.get("candidate_field_occurrence_counts", {}).items():
            aggregate_fields[key] = aggregate_fields.get(key, 0) + int(value)
        for key, value in report.get("native", {}).get("fragment_type_counts", {}).items():
            aggregate_fragments[key] = aggregate_fragments.get(key, 0) + int(value)
        for key, value in report.get("native", {}).get("annotation_symbol_counts", {}).items():
            aggregate_annotations[key] = aggregate_annotations.get(key, 0) + int(value)
        for key, value in report.get("conditional", {}).get(
            "candidate_field_occurrence_counts", {}
        ).items():
            aggregate_conditional_fields[key] = aggregate_conditional_fields.get(key, 0) + int(
                value
            )
        for key, value in report.get("render_inline", {}).get(
            "candidate_field_occurrence_counts", {}
        ).items():
            aggregate_render_inline_fields[key] = aggregate_render_inline_fields.get(key, 0) + int(
                value
            )
        reports.append(
            {
                "file_index": index,
                "input_sha256_audit_only": report.get("input_sha256_audit_only"),
                "status": report.get("status"),
                "error_code": report.get("error_code"),
                "fragment_count": report.get("native", {}).get("fragment_count", 0),
                "fragment_type_counts": report.get("native", {}).get("fragment_type_counts", {}),
                "annotation_symbol_counts": report.get("native", {}).get(
                    "annotation_symbol_counts", {}
                ),
                "field_occurrence_counts_by_fragment": report.get("native", {}).get(
                    "field_occurrence_counts_by_fragment", {}
                ),
                "field_value_kind_counts_by_fragment": report.get("native", {}).get(
                    "field_value_kind_counts_by_fragment", {}
                ),
                "value_kind_counts_by_fragment": report.get("native", {}).get(
                    "value_kind_counts_by_fragment", {}
                ),
                "scalar_string_count": report.get("native", {}).get("scalar_string_count", 0),
                "scalar_string_max_utf8_bytes": report.get("native", {}).get(
                    "scalar_string_max_utf8_bytes", 0
                ),
                "candidate_field_occurrence_counts": report.get(
                    "candidate_field_occurrence_counts", {}
                ),
                "candidate_samples": report.get("candidate_samples", []),
                "conditional": report.get("conditional", {}),
                "render_inline": report.get("render_inline", {}),
                "diagnostics": report.get("diagnostics", {}),
            }
        )
    result = {
        "schema_version": 1,
        "scope": "content_free_original_kfx_semantic_evidence",
        "created_utc": now_utc(),
        "input_count": len(paths),
        "aggregate_candidate_field_occurrence_counts": dict(sorted(aggregate_fields.items())),
        "aggregate_fragment_type_counts": dict(sorted(aggregate_fragments.items())),
        "aggregate_annotation_symbol_counts": dict(sorted(aggregate_annotations.items())),
        "aggregate_conditional_candidate_field_occurrence_counts": dict(
            sorted(aggregate_conditional_fields.items())
        ),
        "aggregate_render_inline_candidate_field_occurrence_counts": dict(
            sorted(aggregate_render_inline_fields.items())
        ),
        "reports": reports,
    }
    write_json(args.output, result)
    print(f"audited {len(paths)} KFX input(s) -> {args.output}")
    return 0


def coverage_command(args: argparse.Namespace) -> int:
    review = load_semantic_review(args.review_file)
    rows = []
    state_counts: dict[str, int] = {}
    for fixture, manifest in fixtures():
        probe_id = str(manifest["id"])
        run_root = args.artifact_root / "semantic-probes" / probe_id
        source_report = run_root / "source-semantics.json"
        result_report = run_root / "result.json"
        state = "NotExercised"
        reason = "no source validation artifact"
        report_path = None
        if result_report.is_file():
            try:
                result = json.loads(result_report.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as exc:
                raise ProbeError(f"invalid {result_report}: {exc}") from exc
            coverage = result.get("coverage", {})
            state = str(coverage.get("state", state))
            reason = str(coverage.get("reason", "result artifact has no coverage reason"))
            report_path = str(result_report)
        elif source_report.is_file():
            try:
                source_result = json.loads(source_report.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as exc:
                raise ProbeError(f"invalid {source_report}: {exc}") from exc
            coverage = source_result.get("coverage", {})
            state = str(coverage.get("state", state))
            reason = str(coverage.get("reason", reason))
            report_path = str(source_report)
        review_entry = review.get(probe_id)
        review_applied = False
        if review_entry is not None and result_report.is_file():
            state = str(review_entry["state"])
            reason = str(review_entry["reason"])
            review_applied = True
        if state not in COVERAGE_STATES:
            raise ProbeError(f"{probe_id}: invalid coverage state {state!r}")
        state_counts[state] = state_counts.get(state, 0) + 1
        rows.append(
            {
                "probe_id": probe_id,
                "feature": manifest["feature"],
                "state": state,
                "reason": reason,
                "report": report_path,
                "review_applied": review_applied,
                "review": review_entry,
            }
        )
    unreviewed_generated = any(
        row["state"] == "Encountered" and not row["review_applied"] for row in rows
    )
    unknown_visible_semantic_count = None
    if not unreviewed_generated:
        unknown_visible_semantic_count = sum(
            int((row.get("review") or {}).get("visible_impact", False))
            for row in rows
            if row["state"] == "Unknown"
        )
    result = {
        "schema_version": 1,
        "scope": "phase_3_5_s_semantic_probe_coverage",
        "created_utc": now_utc(),
        "allowed_states": list(COVERAGE_STATES),
        "probe_count": len(rows),
        "state_counts": dict(sorted(state_counts.items())),
        "review_file": str(args.review_file),
        "unknown_visible_semantic_count": unknown_visible_semantic_count,
        "unknown_visible_semantic_note": (
            "All generated probes have an explicit review classification"
            if unknown_visible_semantic_count is not None
            else "Not computable until every generated KPF/KFX probe is explicitly reviewed"
        ),
        "probes": rows,
    }
    write_json(args.output, result)
    print(f"reported {len(rows)} probe coverage rows -> {args.output}")
    return 0


def selected(args: argparse.Namespace) -> Iterable[tuple[Path, dict[str, Any]]]:
    all_fixtures = fixtures()
    if args.all:
        return all_fixtures
    if not args.probe_id:
        raise ProbeError("provide a probe ID or --all")
    return [fixture_for(args.probe_id)]


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("list", help="list controlled semantic probes")
    evidence = subparsers.add_parser("evidence", help="audit original KFX structure without body text")
    evidence.add_argument("input", type=Path)
    evidence.add_argument("--output", type=Path, required=True)
    evidence.add_argument("--folio", type=Path, default=DEFAULT_FOLIO)
    coverage = subparsers.add_parser("coverage", help="report semantic probe coverage states")
    coverage.add_argument("--artifact-root", type=Path, default=DEFAULT_ARTIFACT_ROOT)
    coverage.add_argument("--review-file", type=Path, default=DEFAULT_REVIEW_FILE)
    coverage.add_argument("--output", type=Path, required=True)
    for name in ("validate", "build"):
        sub = subparsers.add_parser(name, help=f"{name} one probe or all probes")
        sub.add_argument("probe_id", nargs="?")
        sub.add_argument("--all", action="store_true")
        sub.add_argument("--artifact-root", type=Path, default=DEFAULT_ARTIFACT_ROOT)
        sub.add_argument("--folio", type=Path, default=DEFAULT_FOLIO)
        if name == "build":
            sub.add_argument("--previewer", type=Path, default=DEFAULT_PREVIEWER)
            sub.add_argument("--calibre-config", type=Path, default=DEFAULT_CALIBRE_CONFIG)
            sub.add_argument("--calibre-debug", type=Path, default=DEFAULT_CALIBRE_DEBUG)
    args = parser.parse_args(argv)
    try:
        if args.command == "list":
            return list_command()
        if args.command == "evidence":
            return evidence_command(args)
        if args.command == "coverage":
            return coverage_command(args)
        selected_fixtures = list(selected(args))
        if args.command == "validate":
            for fixture, manifest in selected_fixtures:
                report = validate_source_semantics(fixture, manifest)
                output = args.artifact_root / "semantic-probes" / manifest["id"] / "source-semantics.json"
                write_json(output, report)
                print(f"validated {manifest['id']} -> {output}")
            return 0
        folio = executable(args.folio, "folio")
        previewer = executable(args.previewer, "Kindle Previewer 3")
        calibre_debug = executable(args.calibre_debug, "calibre-debug")
        args.calibre_config.mkdir(parents=True, exist_ok=True)
        failures = []
        for fixture, manifest in selected_fixtures:
            try:
                run_root = build_probe(
                    fixture,
                    manifest,
                    args.artifact_root,
                    folio,
                    previewer,
                    args.calibre_config,
                    calibre_debug,
                )
            except ProbeError as exc:
                failures.append(str(manifest["id"]))
                print(f"kfx-probe: {manifest['id']}: {exc}", file=sys.stderr)
            else:
                print(f"generated {manifest['id']} -> {run_root}")
        if failures:
            print(
                "kfx-probe: build completed with failures: " + ", ".join(failures),
                file=sys.stderr,
            )
            return 2
        return 0
    except ProbeError as exc:
        print(f"kfx-probe: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
