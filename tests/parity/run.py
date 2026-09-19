#!/usr/bin/env python3
import ctypes
import hashlib
import json
import os
import platform
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
TARGET_DIR = ROOT / "target" / "debug"
CONFIG = json.loads((ROOT / "tests/parity/conversion.json").read_text())
EDIT = json.loads((ROOT / "tests/parity/edit-plan.json").read_text())
REPORT_FIELDS = (
    "source_format",
    "target_format",
    "degradation_mode",
    "compatibility",
    "metadata",
    "features",
    "resource_summary",
    "warnings",
    "degradation",
    "round_trip",
    "input_report",
    "semantic_report",
    "compatibility_report",
    "output_report",
)


class FolioResult(ctypes.Structure):
    _fields_ = [("code", ctypes.c_int32), ("json", ctypes.c_void_p)]


def run_cli(binary, *args):
    result = subprocess.run(
        [str(binary), *map(str, args)],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return json.loads(result.stdout)


def convert_through_ffi(input_path, output_path, library_path):
    library = ctypes.CDLL(str(library_path))
    library.folio_convert.argtypes = [ctypes.c_char_p]
    library.folio_convert.restype = ctypes.POINTER(FolioResult)
    library.folio_result_free.argtypes = [ctypes.POINTER(FolioResult)]
    library.folio_result_free.restype = None

    request = {
        "input": str(input_path),
        "output": str(output_path),
        "target": CONFIG["target"],
        "options": {
            "deterministic": CONFIG["deterministic"],
            "compression": CONFIG["compression"],
            "degradation_mode": CONFIG["mode"],
            "degradation": CONFIG["degradation"],
        },
        "edit": EDIT,
    }
    pointer = library.folio_convert(json.dumps(request).encode())
    if not pointer:
        raise RuntimeError("macOS/Core C ABI returned a null result")
    try:
        result = pointer.contents
        payload = json.loads(ctypes.string_at(result.json).decode())
        if result.code != 0:
            raise RuntimeError(f"macOS/Core C ABI conversion failed: {payload}")
        return payload
    finally:
        library.folio_result_free(pointer)


def multipart(fields, input_path):
    boundary = "----FolioForgeParityBoundary"
    body = bytearray()
    for name, value in fields.items():
        body.extend(f"--{boundary}\r\n".encode())
        body.extend(f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode())
        body.extend(value.encode())
        body.extend(b"\r\n")
    body.extend(f"--{boundary}\r\n".encode())
    body.extend(
        b'Content-Disposition: form-data; name="files[]"; filename="input.epub"\r\n'
        b"Content-Type: application/epub+zip\r\n\r\n"
    )
    body.extend(input_path.read_bytes())
    body.extend(f"\r\n--{boundary}--\r\n".encode())
    return boundary, bytes(body)


def convert_through_service(service_binary, input_path, output_path, work_dir):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    address = f"http://127.0.0.1:{port}"
    env = os.environ.copy()
    env.update(
        {
            "FOLIOFORGE_BIND": f"127.0.0.1:{port}",
            "FOLIOFORGE_WORK_DIR": str(work_dir),
            "FOLIOFORGE_ONLINE_ENABLED": "0",
        }
    )
    log_path = work_dir.parent / "service.log"
    log = log_path.open("wb")
    process = subprocess.Popen(
        [str(service_binary)], env=env, stdout=log, stderr=subprocess.STDOUT
    )
    try:
        for _ in range(100):
            if process.poll() is not None:
                raise RuntimeError(f"service exited; see {log_path}")
            try:
                with urllib.request.urlopen(address + "/health", timeout=0.5):
                    break
            except (urllib.error.URLError, TimeoutError):
                time.sleep(0.1)
        else:
            raise RuntimeError(f"service did not become healthy; see {log_path}")

        fields = {
            "target": CONFIG["target"].lower(),
            "mode": CONFIG["mode"].lower(),
            "batch_mode": "best_effort",
            "edit": json.dumps(EDIT, separators=(",", ":")),
        }
        boundary, body = multipart(fields, input_path)
        request = urllib.request.Request(
            address + "/convert",
            data=body,
            headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            job_id = json.loads(response.read())["job_id"]

        for _ in range(600):
            with urllib.request.urlopen(address + f"/jobs/{job_id}", timeout=2) as response:
                state = json.loads(response.read())
            if state["status"] not in {"queued", "running"}:
                break
            time.sleep(0.1)
        else:
            raise RuntimeError(f"service conversion timed out: {job_id}")
        if state["status"] != "completed":
            raise RuntimeError(f"service conversion ended as {state['status']}: {state}")

        with urllib.request.urlopen(
            address + f"/jobs/{job_id}/download", timeout=30
        ) as response:
            output_path.write_bytes(response.read())
        return state["report"]["items"][0]["report"]
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        log.close()


def semantic_output(binary, path):
    return run_cli(binary, "inspect", path, "--semantic")


def stable_report(report):
    value = {field: report[field] for field in REPORT_FIELDS}
    value["output_report"] = dict(value["output_report"])
    value["output_report"].pop("path", None)
    return value


def main():
    cli = TARGET_DIR / "folio"
    service = TARGET_DIR / "folio-service"
    library_name = {
        "Darwin": "libfolio_ffi.dylib",
        "Linux": "libfolio_ffi.so",
    }.get(platform.system())
    if not library_name:
        raise RuntimeError("the parity C ABI harness supports macOS and Linux hosts")
    library = TARGET_DIR / library_name
    for path in (cli, service, library):
        if not path.exists():
            raise RuntimeError(f"missing parity prerequisite: {path}")

    with tempfile.TemporaryDirectory(prefix="folioforge-parity-") as directory:
        work = Path(directory)
        input_path = ROOT / "tests/parity/input.epub"
        edit_path = ROOT / "tests/parity/edit-plan.json"

        cli_output = work / "cli.azw3"
        cli_report = run_cli(
            cli,
            "convert",
            input_path,
            "--to",
            CONFIG["target"].lower(),
            "--mode",
            CONFIG["mode"].lower(),
            "--output",
            cli_output,
            "--edit-plan",
            edit_path,
        )

        ffi_output = work / "ffi.azw3"
        ffi_report = convert_through_ffi(input_path, ffi_output, library)

        service_work = work / "service-work"
        service_work.mkdir()
        service_output = work / "service.azw3"
        service_report = convert_through_service(
            service, input_path, service_output, service_work
        )

        reports = [cli_report, ffi_report, service_report]
        normalized = [stable_report(report) for report in reports]
        if normalized[1:] != normalized[:-1]:
            names = ("cli", "native-c-abi", "web-service")
            for field in REPORT_FIELDS:
                values = [report[field] for report in reports]
                if not all(value == values[0] for value in values[1:]):
                    print(f"parity report mismatch in {field}:")
                    for name, value in zip(names, values):
                        print(f"  {name}: {json.dumps(value, ensure_ascii=False)[:1600]}")
            raise AssertionError("CLI, native C ABI, and Web/service Core reports differ")

        outputs = [cli_output, ffi_output, service_output]
        hashes = [hashlib.sha256(path.read_bytes()).hexdigest() for path in outputs]
        if len(set(hashes)) != 1:
            raise AssertionError(f"deterministic client outputs differ: {hashes}")

        semantic = [semantic_output(cli, path) for path in outputs]
        if semantic[1:] != semantic[:-1]:
            raise AssertionError("re-imported Semantic IR differs between clients")
        if cli_report["metadata"]["title"] != EDIT["metadata"]["title"]:
            raise AssertionError("the shared BookEditPlan was not applied")

        print(
            "cross-client parity passed: CLI = native C ABI = Web/service; "
            f"sha256={hashes[0]}"
        )


if __name__ == "__main__":
    main()
