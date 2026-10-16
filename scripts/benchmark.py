#!/usr/bin/env python3
"""Measure a release binary against isolated synthetic data, never private KBs.

Each observation launches a new CLI process. Warm samples reuse the OS filesystem
cache; this script does NOT evict caches or claim filesystem-cold measurements.
The index rebuild is measured separately from indexed retrieval. No dependencies.
"""

import argparse
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n")


def digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()).hexdigest()


def instant(sequence=0):
    return (dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc) + dt.timedelta(seconds=sequence)).isoformat().replace("+00:00", "Z")


def manifest(identifier):
    return {
        "schema_version": "1.0", "id": identifier, "title": "Synthetic benchmark " + identifier,
        "summary": "Generated benchmark content; no private knowledge.", "version": "0.1.0",
        "visibility": "private", "owner": "Synthetic fixture", "language": "en",
        "created_at": "2026-01-01", "updated_at": "2026-01-01", "entrypoint": "overview",
        "pages": [], "sources": [{"id": "fixture", "title": "Synthetic benchmark generator",
                                  "type": "observation", "uri": "fixture://benchmark", "checked_at": "2026-01-01"}],
        "assets": [],
    }


def declare(m, identifier, path):
    m["assets"].append({"id": identifier, "path": path, "media_type": "application/json"})


def page_fixture(root, identifier, count):
    base = root / "bases" / identifier
    m = manifest(identifier)
    for index in range(count):
        page_id = "overview" if index == 0 else f"page-{index:05d}"
        path = f"pages/{page_id}.md"
        m["pages"].append({"id": page_id, "title": f"Synthetic topic {index}", "path": path,
                           "kind": "reference", "status": "documented", "sources": ["fixture"], "tags": ["benchmark"]})
        page = base / path
        page.parent.mkdir(parents=True, exist_ok=True)
        page.write_text(f"# Synthetic topic {index}\n\n"
                        "This document explains a synthetic ownership and retrieval example.\n\n"
                        + ("The lookup needle identifies this benchmark passage.\n\n" if index % 7 == 0 else "")
                        + "## Procedure\n\n"
                        + "1. Select one knowledge partition.\n2. Read its provenance and revision.\n"
                        + "3. Keep current observations separate from target requirements.\n\n"
                        + ("Additional synthetic explanatory text keeps the corpus reproducible.\n" * 12))
    write_json(base / "knowledge-base.json", m)
    return {"id": identifier, "pages": count, "assertions": 0}


def assertion_fixture(root, identifier, count):
    record = page_fixture(root, identifier, 1)
    record["assertions"] = count
    base = root / "bases" / identifier
    m = json.loads((base / "knowledge-base.json").read_text())
    profile = {
        "schema_version": "1.0", "profile": "tars-memory/1.0", "sequence": math.ceil(count / 1000) + 1,
        "created_at": instant(), "history_started_at": instant(), "last_recorded_at": instant(math.ceil(count / 1000) + 1),
        "concepts": ["topic"], "assertions": [], "transactions": [], "idempotency": {},
    }
    declare(m, "memory-profile", "data/memory/profile.json")
    declare(m, "memory-concept-topic", "data/memory/concepts/topic.json")
    concept = {
        "schema_version": "1.0", "profile": "tars-memory/1.0", "id": "topic", "title": "Synthetic topic",
        "summary": "Synthetic assertions used only to measure local retrieval.", "aliases": ["benchmark"],
        "parents": [], "related": [], "pages": ["overview"], "created_at": instant(1), "sequence": 1,
    }
    concept["revision"] = digest(concept)
    write_json(base / "data/memory/concepts/topic.json", concept)
    concept_input = {key: concept[key] for key in ("title", "summary", "aliases", "parents", "related", "pages")}
    operations = {1: [{"op": "concept-init", "id": "topic", "input": concept_input}]}
    receipts = {1: [{"id": "topic", "sequence": 1}]}
    for index in range(count):
        assertion_id = f"claim-{index:05d}"
        sequence = index // 1000 + 2
        content = {"id": assertion_id, "concepts": ["topic"], "basis": "definition",
                   "text": f"Synthetic assertion {index} describes ownership and retrieval for group {index % 31}."
                           + (" The lookup needle belongs to this passage." if index % 7 == 0 else ""),
                   "temporal": {"kind": "timeless"}, "sources": ["fixture"], "scope": {"environment": "synthetic"}}
        revision = {"sequence": sequence, "recorded_at": instant(sequence), "mode": "assert", "content": content,
                    "source_snapshots": m["sources"], "retracted": False}
        revision["revision"] = digest(revision)
        operations.setdefault(sequence, []).append({"op": "remember", "input": content})
        receipts.setdefault(sequence, []).append({"id": assertion_id, "revision": revision["revision"], "sequence": sequence, "recorded_at": instant(sequence)})
        path = f"data/memory/assertions/{assertion_id}.json"
        write_json(base / path, {"schema_version": "1.0", "profile": "tars-memory/1.0", "id": assertion_id, "revisions": [revision]})
        declare(m, "memory-assertion-" + assertion_id, path)
        profile["assertions"].append(assertion_id)
    for sequence in range(1, profile["sequence"] + 1):
        profile["transactions"].append({"sequence": sequence, "recorded_at": instant(sequence),
                                        "clock_regressed": False, "previous_recorded_at": instant(sequence - 1),
                                        "operation_digest": digest(operations[sequence]), "results": receipts[sequence]})
    write_json(base / "data/memory/profile.json", profile)
    write_json(base / "knowledge-base.json", m)
    return record


def percentile(values, probability):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * probability) - 1)]


def run_once(command, env, expected=None):
    start = time.perf_counter_ns()
    result = subprocess.run(command, capture_output=True, env=env, timeout=300, check=False)
    elapsed = (time.perf_counter_ns() - start) / 1_000_000
    if result.returncode:
        diagnostic = result.stderr or result.stdout
        raise RuntimeError(diagnostic.decode(errors="replace")[:2000])
    if expected:
        expected(json.loads(result.stdout))
    return elapsed, len(result.stdout)


def measure(name, command, env, runs, warmup=2, expected=None):
    first, first_bytes = run_once(command, env, expected)
    for _ in range(warmup):
        run_once(command, env, expected)
    samples = [run_once(command, env, expected) for _ in range(runs)]
    times = [sample[0] for sample in samples]
    sizes = [sample[1] for sample in samples]
    return {"operation": name, "status": "measured", "samples": runs, "warmup_processes": warmup,
            "first_process_ms": first, "first_response_bytes": first_bytes,
            "warm_process_ms": {"min": min(times), "p50": statistics.median(times), "p95": percentile(times, .95), "p99": percentile(times, .99), "max": max(times)},
            "response_bytes": {"min": min(sizes), "p50": statistics.median(sizes), "max": max(sizes)}}


def nonempty_hits(value):
    assert value["hits"], "a fast empty search is not a successful benchmark"


def nonempty_literal(value):
    assert isinstance(value, list) and value, "expected nonempty legacy literal hits"


def valid_fixture(value):
    assert value.get("valid") is True, "generated fixtures must pass native validation: " + str(value.get("errors"))


def version(command):
    try:
        return subprocess.check_output(command, text=True, stderr=subprocess.DEVNULL, timeout=10).strip()
    except (OSError, subprocess.SubprocessError):
        return "unavailable"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/tars-kb"))
    parser.add_argument("--pages", default="100,1000")
    parser.add_argument("--assertions", default="1000,10000")
    parser.add_argument("--runs", type=int, default=15)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--work-dir", type=Path, default=Path("target/benchmark-work"),
                        help="synthetic staging parent; defaults to the build tree instead of potentially RAM-backed /tmp")
    parser.add_argument("--with-writes", action="store_true", help="also benchmark durable synthetic claim creation")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    assert args.runs >= 3, "use at least three measured repetitions"
    pages = [int(count) for count in args.pages.split(",") if count]
    assertions = [int(count) for count in args.assertions.split(",") if count]
    assert all(1 <= count <= 100000 for count in pages + assertions)
    cpu = "unavailable"
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.exists():
        cpu = next((line.split(":", 1)[1].strip() for line in cpuinfo.read_text().splitlines() if line.startswith("model name")), cpu)
    compiler = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc")
    report = {"schema_version": "1.0", "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "binary_version": version([str(binary), "--version"]), "compiler": version([compiler, "--version"]),
              "system": {"os": platform.system(), "kernel": platform.release(), "architecture": platform.machine(), "cpu": cpu},
              "method": {"each_sample_new_process": True, "filesystem_cache": "warm/uncontrolled; not evicted",
                         "timing": "wall time through complete stdout/stderr capture", "warmup_processes": "2 except index operations (0); recorded per operation",
                         "peak_rss": "not measured", "source_data": "synthetic only; generated directly, outside measured command time",
                         "rebuilds": "each sample explicitly rebuilds the derived index; OS caches remain warm"},
              "datasets": [], "measurements": []}
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="tars-kb-benchmark-", dir=args.work_dir.resolve()) as directory:
        directory = Path(directory)
        pinned_binary = directory / "tars-kb"
        shutil.copyfile(binary, pinned_binary)
        pinned_binary.chmod(0o700)
        binary = pinned_binary
        report["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        report["binary_version"] = version([str(binary), "--version"])
        report["method"]["binary"] = "private executable snapshot pinned for all measured samples"
        root = directory / "library"
        report["system"]["benchmark_filesystem"] = version(["stat", "-f", "-c", "%T", str(directory)])
        report["method"]["durability"] = "native writer fsync behavior retained; backing filesystem reported; no power-loss test"
        start = time.perf_counter()
        datasets = [page_fixture(root, f"pages-{count}", count) for count in pages]
        datasets += [assertion_fixture(root, f"assertions-{count}", count) for count in assertions]
        write_json(root / "catalog.json", {"schema_version": "1.0", "knowledge_bases": [{"id": row["id"], "path": "bases/" + row["id"]} for row in datasets]})
        report["generation_seconds"] = time.perf_counter() - start
        report["datasets"] = datasets
        report["source_bytes"] = sum(path.stat().st_size for path in root.rglob("*") if path.is_file())
        env = {**os.environ, "XDG_CACHE_HOME": str(directory / "cache")}
        prefix = [str(binary), "--root", str(root)]
        run_once([*prefix, "validate", "--json"], env, valid_fixture)
        report["fixture_validation"] = "passed native full validation before measurements"
        report["measurements"].append(measure("startup-help", [str(binary), "--help"], env, args.runs))
        report["measurements"].append(measure("catalog-list", [*prefix, "list", "--json"], env, args.runs))
        for dataset in datasets:
            identifier = dataset["id"]
            operations = [
                ("show-page", ["show", identifier, "overview", "--json"], None),
                ("literal-search", ["search", identifier, "needle", "--json"], nonempty_literal),
                ("index-rebuild", ["index", identifier, "--rebuild"], None),
                ("index-update-unchanged", ["index", identifier], None),
                ("ranked-search", ["search", identifier, "needle", "--mode", "ranked", "--limit", "8", "--max-bytes", "8192", "--json"], nonempty_hits),
                ("ranked-common", ["search", identifier, "synthetic", "--mode", "ranked", "--limit", "8", "--max-bytes", "8192", "--json"], nonempty_hits),
                ("ranked-negative", ["search", identifier, "absentcanaryxyz", "--mode", "ranked", "--limit", "8", "--max-bytes", "8192", "--json"], None),
            ]
            if dataset["assertions"]:
                operations.extend([
                    ("read-one-fact", ["fact", identifier, "claim-00000", "--json"], None),
                    ("read-one-concept", ["concept", "get", identifier, "topic", "--json"], None),
                ])
            for operation, command, expected in operations:
                print(f"Measuring {identifier}: {operation}", file=sys.stderr, flush=True)
                try:
                    result = measure(operation, [*prefix, *command], env,
                                     args.runs if not operation.startswith("index-") else min(3, args.runs),
                                     warmup=0 if operation.startswith("index-") else 2, expected=expected)
                except (RuntimeError, AssertionError, subprocess.TimeoutExpired) as error:
                    result = {"operation": operation, "status": "error", "error": str(error)}
                result["dataset"] = identifier
                report["measurements"].append(result)
            if args.with_writes and dataset["assertions"]:
                samples = []
                for number in range(min(5, args.runs)):
                    content = {"id": f"benchmark-write-{number}", "concepts": ["topic"], "basis": "definition",
                               "text": "A synthetic benchmark mutation with a durable receipt.", "temporal": {"kind": "timeless"}, "sources": ["fixture"]}
                    input_path = directory / "claim.json"
                    write_json(input_path, content)
                    try:
                        samples.append(run_once([*prefix, "remember", identifier, "topic", "--input", str(input_path)], env))
                    except (RuntimeError, subprocess.TimeoutExpired) as error:
                        report["measurements"].append({"operation": "durable-claim-create", "dataset": identifier, "status": "error", "error": str(error)})
                        break
                else:
                    times = [sample[0] for sample in samples]
                    report["measurements"].append({"operation": "durable-claim-create", "dataset": identifier, "status": "measured", "samples": len(samples), "warmup_processes": 0,
                                                   "warm_process_ms": {"p50": statistics.median(times), "p95": percentile(times, .95), "p99": percentile(times, .99)},
                                                   "note": "new assertion and manifest/history growth; no fsync shortcuts"})
    encoded = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded)
    else:
        print(encoded, end="")


if __name__ == "__main__":
    main()
