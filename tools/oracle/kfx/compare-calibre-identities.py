#!/usr/bin/env python3
"""Compare placement candidates with Calibre without retaining book content."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from collections import Counter, deque
from pathlib import Path
from typing import Any


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def exact_identity_names(resource_audit: dict[str, Any]) -> list[str]:
    return sorted(
        {
            row["fragment_key"]["fid"]
            for row in resource_audit.get("external_resources", [])
            if row.get("identity_status") == "ResolvedByExactIdentity"
            and isinstance(row.get("fragment_key", {}).get("fid"), str)
        }
    )


def ordered_stage_comparison(
    occurrences: list[dict[str, Any]],
    oracle_names: list[str],
    alias_to_name: dict[str, str],
) -> dict[str, Any]:
    stage_names = [
        alias_to_name.get(row.get("resource"))
        if row.get("identity_status") == "exact_image_identity"
        else None
        for row in occurrences
    ]
    comparison_count = min(len(stage_names), len(oracle_names))
    matched = sum(
        stage_names[index] == oracle_names[index]
        for index in range(comparison_count)
    )
    frequencies = Counter(name for name in stage_names if name is not None)
    return {
        "occurrence_count": len(stage_names),
        "ordered_identity_matches": matched,
        "ordered_comparison_count": comparison_count,
        "resource_frequencies_equal_oracle": frequencies == Counter(oracle_names),
    }


def capacity_matching_analysis(
    candidate_rows: list[set[str]], capacities: Counter[str]
) -> tuple[int, list[int] | None]:
    """Match source occurrences to oracle resource capacities in one flow pass.

    When every source occurrence is matched, the residual graph identifies all
    candidate edges that can participate in any full matching. This avoids
    re-running a whole matching once for every occurrence.
    """
    names = sorted(name for name, capacity in capacities.items() if capacity > 0)
    source = 0
    row_start = 1
    name_start = row_start + len(candidate_rows)
    sink = name_start + len(names)
    graph: list[list[list[int]]] = [[] for _ in range(sink + 1)]

    def add_edge(start: int, end: int, capacity: int) -> int:
        forward_index = len(graph[start])
        reverse_index = len(graph[end])
        graph[start].append([end, reverse_index, capacity])
        graph[end].append([start, forward_index, 0])
        return forward_index

    name_nodes = {name: name_start + index for index, name in enumerate(names)}
    row_edges: list[list[tuple[str, int]]] = []
    for row_index, candidates in enumerate(candidate_rows):
        row_node = row_start + row_index
        add_edge(source, row_node, 1)
        edges = []
        for name in sorted(candidates):
            name_node = name_nodes.get(name)
            if name_node is not None:
                edge_index = add_edge(row_node, name_node, 1)
                edges.append((name, edge_index))
        row_edges.append(edges)
    for name, name_node in name_nodes.items():
        add_edge(name_node, sink, capacities[name])

    sys.setrecursionlimit(max(sys.getrecursionlimit(), len(graph) + 100))
    level = [-1] * len(graph)
    cursor = [0] * len(graph)

    def build_levels() -> bool:
        level[:] = [-1] * len(graph)
        level[source] = 0
        queue = deque([source])
        while queue:
            node = queue.popleft()
            for neighbor, _, residual in graph[node]:
                if residual > 0 and level[neighbor] < 0:
                    level[neighbor] = level[node] + 1
                    queue.append(neighbor)
        return level[sink] >= 0

    def send_flow(node: int, amount: int) -> int:
        if node == sink:
            return amount
        while cursor[node] < len(graph[node]):
            edge = graph[node][cursor[node]]
            neighbor, reverse_index, residual = edge
            if residual > 0 and level[neighbor] == level[node] + 1:
                pushed = send_flow(neighbor, min(amount, residual))
                if pushed:
                    edge[2] -= pushed
                    graph[neighbor][reverse_index][2] += pushed
                    return pushed
            cursor[node] += 1
        return 0

    matched = 0
    while build_levels():
        cursor[:] = [0] * len(graph)
        while pushed := send_flow(source, len(candidate_rows) - matched):
            matched += pushed

    if matched != len(candidate_rows):
        return matched, None

    # Compute strongly connected components of the residual network. An
    # unused occurrence→resource edge is feasible in another full matching
    # exactly when it lies on a residual cycle.
    residual_reverse: list[list[int]] = [[] for _ in graph]
    for node, edges in enumerate(graph):
        for neighbor, _, residual in edges:
            if residual > 0:
                residual_reverse[neighbor].append(node)

    visited = [False] * len(graph)
    finish_order: list[int] = []
    for root in range(len(graph)):
        if visited[root]:
            continue
        visited[root] = True
        stack = [(root, 0)]
        while stack:
            node, edge_index = stack[-1]
            while edge_index < len(graph[node]) and graph[node][edge_index][2] <= 0:
                edge_index += 1
            if edge_index == len(graph[node]):
                stack.pop()
                finish_order.append(node)
                continue
            stack[-1] = (node, edge_index + 1)
            neighbor = graph[node][edge_index][0]
            if not visited[neighbor]:
                visited[neighbor] = True
                stack.append((neighbor, 0))

    component = [-1] * len(graph)
    component_id = 0
    for root in reversed(finish_order):
        if component[root] >= 0:
            continue
        component[root] = component_id
        stack = [root]
        while stack:
            node = stack.pop()
            for neighbor in residual_reverse[node]:
                if component[neighbor] < 0:
                    component[neighbor] = component_id
                    stack.append(neighbor)
        component_id += 1

    feasible_counts = []
    for row_index, edges in enumerate(row_edges):
        row_node = row_start + row_index
        feasible = 0
        for name, edge_index in edges:
            name_node = name_nodes[name]
            if (
                graph[row_node][edge_index][2] == 0
                or component[row_node] == component[name_node]
            ):
                feasible += 1
        feasible_counts.append(feasible)
    return matched, feasible_counts


def run(args: argparse.Namespace) -> int:
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    entries = manifest.get("inputs")
    if not isinstance(entries, list) or not entries:
        raise ValueError("anonymous placement input manifest is empty or invalid")
    matches: dict[str, list[Path]] = {item["sha256"]: [] for item in entries}
    for path in args.book_root.iterdir():
        if (
            path.is_file()
            and not path.is_symlink()
            and path.suffix.lower() == ".kfx"
        ):
            digest = file_sha256(path)
            if digest in matches:
                matches[digest].append(path)
    for item in entries:
        if len(matches[item["sha256"]]) != 1:
            raise ValueError("pinned local input did not resolve uniquely")

    calibre_version = subprocess.run(
        ["calibre-debug", "--version"], check=True, capture_output=True, text=True
    ).stdout.strip()

    summaries: list[dict[str, Any]] = []
    for item in entries:
        book_path = matches[item["sha256"]][0]
        resource_audit = json.loads(
            subprocess.run(
                [str(args.folio), "inspect", str(book_path), "--kfx-resource-audit"],
                check=True,
                capture_output=True,
                text=True,
                timeout=180,
            ).stdout
        )
        placement_audit = json.loads(
            subprocess.run(
                [str(args.folio), "inspect", str(book_path), "--kfx-placement-audit"],
                check=True,
                capture_output=True,
                text=True,
                timeout=180,
            ).stdout
        )
        identity_names = exact_identity_names(resource_audit)
        alias_catalog = placement_audit["resource_identity"]["resource_alias_catalog"]
        if len(identity_names) != len(alias_catalog):
            raise ValueError("anonymous resource alias catalog did not match exact identities")
        alias_to_name = dict(zip(alias_catalog, identity_names))

        raw_path = args.temp_dir / (item["id"] + ".raw.json")
        result = subprocess.run(
            [
                "calibre-debug",
                "-r",
                "KFX Input",
                "--",
                "--json-content",
                str(book_path),
                str(raw_path),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=180,
        )
        try:
            if result.returncode != 0 or not raw_path.is_file():
                raise RuntimeError("KFX Input oracle failed")
            raw = json.loads(raw_path.read_text(encoding="utf-8"))
            rows = raw.get("data")
            if not isinstance(rows, list):
                raise ValueError("unexpected KFX Input JSON structure")
            oracle_occurrences = [
                (row["position"], ordinal, row["content"])
                for ordinal, row in enumerate(rows)
                if isinstance(row, dict)
                and row.get("type") == 2
                and isinstance(row.get("position"), int)
                and isinstance(row.get("content"), str)
            ]
        finally:
            raw_path.unlink(missing_ok=True)

        oracle_occurrences.sort(key=lambda row: (row[0], row[1]))
        oracle_names = [row[2] for row in oracle_occurrences]
        oracle_frequencies = Counter(oracle_names)
        source_rows = placement_audit["raw_reference"]["occurrences"]
        direct_rows = [
            {alias_to_name[row["resource"]]} if row.get("resource") in alias_to_name else set()
            for row in source_rows
        ]
        alternate_rows = [
            {
                alias_to_name[alias]
                for alias in row["alternate_symbol_resources_audit_only"]
                if alias in alias_to_name
            }
            for row in source_rows
        ]
        candidate_rows = [
            direct | alternate for direct, alternate in zip(direct_rows, alternate_rows)
        ]
        direct_frequencies = Counter(name for names in direct_rows for name in names)
        matching_count, feasible_candidate_counts = capacity_matching_analysis(
            candidate_rows, oracle_frequencies
        )
        oracle_name_set = set(oracle_names)
        uniquely_forced = (
            sum(count == 1 for count in feasible_candidate_counts)
            if feasible_candidate_counts is not None
            else None
        )
        multiply_feasible = (
            sum(count > 1 for count in feasible_candidate_counts)
            if feasible_candidate_counts is not None
            else None
        )
        no_feasible = (
            sum(count == 0 for count in feasible_candidate_counts)
            if feasible_candidate_counts is not None
            else None
        )
        ordered_stages = {
            name: ordered_stage_comparison(
                placement_audit[name]["occurrences"], oracle_names, alias_to_name
            )
            for name in ("native_placement", "semantic_placement", "ir_image_nodes")
        }
        summaries.append(
            {
                "input_id": item["id"],
                "oracle_occurrence_count": len(oracle_occurrences),
                "raw_reference_occurrence_count": len(source_rows),
                "native_occurrence_count": len(placement_audit["native_placement"]["occurrences"]),
                "semantic_occurrence_count": len(placement_audit["semantic_placement"]["occurrences"]),
                "ir_image_node_count": len(placement_audit["ir_image_nodes"]["occurrences"]),
                "kfx_exact_identity_count": len(identity_names),
                "oracle_distinct_resources": len(oracle_name_set),
                "oracle_names_without_exact_kfx_identity_count": len(
                    oracle_name_set - set(identity_names)
                ),
                "exact_kfx_identities_without_oracle_use_count": len(
                    set(identity_names) - oracle_name_set
                ),
                "oracle_resource_names_equal_exact_kfx_fids": oracle_name_set
                == set(identity_names),
                "ordered_reading_order_stage_comparisons": ordered_stages,
                "raw_direct_sid_resource_frequencies_equal_oracle": direct_frequencies
                == oracle_frequencies,
                "raw_reference_maximum_unordered_candidate_matching": matching_count,
                "raw_reference_frequency_constrained_full_matching_exists": (
                    feasible_candidate_counts is not None
                ),
                "raw_reference_uniquely_forced_occurrences": uniquely_forced,
                "raw_reference_ambiguous_occurrences": multiply_feasible,
                "raw_reference_unmatched_occurrences": no_feasible,
                "unresolved_raw_occurrences_after_candidate_matching": max(
                    len(oracle_names), len(candidate_rows)
                )
                - matching_count,
            }
        )
        print(f"Compared {item['id']}.", flush=True)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(
            {
                "schema_version": 2,
                "calibre_version": calibre_version,
                "kfx_input_version": "2.34.2",
                "comparison": "anonymous-calibre-position-order-vs-kfx-native-semantic-ir-stages-and-raw-resource-frequencies",
                "books": summaries,
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
    parser.add_argument("--book-root", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--folio", required=True, type=Path)
    parser.add_argument("--temp-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        return run(args)
    except (
        OSError,
        KeyError,
        TypeError,
        ValueError,
        RuntimeError,
        subprocess.CalledProcessError,
        subprocess.TimeoutExpired,
        json.JSONDecodeError,
    ) as error:
        parser.error(
            "identity comparison failed ({}); source names and book metadata were not emitted".format(
                type(error).__name__
            )
        )
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
