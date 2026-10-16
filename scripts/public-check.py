#!/usr/bin/env python3
"""Reject common private artifacts before publishing this synthetic-only repository.

This is a release gate, not a complete secret detector. Diagnostics name the file,
line and rule without repeating potentially sensitive matching contents.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path


TEXT_RULES = {
    "private-home-path": re.compile(
        r"/(?:home|Users)/[A-Za-z0-9._-]+(?:/|\b)"
        r"|/" + r"root/"
        r"|/tmp/" + r"codex-remote-attachments/"
    ),
    "private-service-path": re.compile(
        r"/(?:var/lib|var/log|opt)/" + r"tars(?:[-/]|\b)"
    ),
    "private-tailnet": re.compile(r"\b[a-z0-9.-]+\.ts\.net\b", re.IGNORECASE),
    "private-domain": re.compile(r"\bash" + r"-os\.io\b", re.IGNORECASE),
    "private-source-repository": re.compile(
        r"github\.com/" + r"cottrellashley/" + r"tars-setup(?:/|\b)",
        re.IGNORECASE,
    ),
    "private-network-address": re.compile(
        r"(?<![\d.])(?:10(?:\.\d{1,3}){3}|192\.168(?:\.\d{1,3}){2}"
        r"|172\.(?:1[6-9]|2\d|3[01])(?:\.\d{1,3}){2}"
        r"|100\.(?:6[4-9]|[7-9]\d|1[01]\d|12[0-7])(?:\.\d{1,3}){2})(?![\d.])"
    ),
    "private-key": re.compile(r"-----BEGIN (?:[A-Z0-9]+ )?PRIVATE KEY-----"),
    "github-token": re.compile(
        r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b"
    ),
    "provider-key": re.compile(r"\bsk-(?:proj-)?[A-Za-z0-9_-]{30,}\b"),
    "cloud-access-key": re.compile(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"),
    "oauth-token": re.compile(r"\bya29\.[A-Za-z0-9_-]{30,}\b"),
    "credential-value": re.compile(
        r"(?i)\b(?:api[_-]?key|access[_-]?token|client[_-]?secret|password)"
        r"\s*[:=]\s*[\"']?[A-Za-z0-9_+/=-]{20,}"
    ),
    "embedded-basic-auth": re.compile(r"https?://[^\s/@:]+:[^\s/@]+@"),
}

FORBIDDEN_PARTS = {
    ".git", ".cache", ".venv", ".tars-kb", ".runtime", "target", "node_modules",
    "runtime", "indexes", "indices", ".wwebjs_auth", ".wwebjs_cache",
}
FORBIDDEN_NAMES = {
    ".env", "auth.json", "credentials.json", "machine.json", "cookies.sqlite",
    "id_rsa", "id_ed25519", "login.keychain-db", ".tars-kb-pending",
    ".netrc", ".npmrc", ".pypirc", ".git-credentials",
}
FORBIDDEN_SUFFIXES = {
    ".sqlite", ".sqlite3", ".db", ".qcow2", ".pem", ".key", ".p12", ".pfx",
}


def path_problem(path: Path) -> str | None:
    if any(part in FORBIDDEN_PARTS for part in path.parts):
        return "runtime-or-index-artifact"
    if path.name in FORBIDDEN_NAMES or path.name.startswith(".env."):
        return "credential-or-machine-file"
    if path.suffix.lower() in FORBIDDEN_SUFFIXES:
        return "binary-database-or-key-file"
    return None


def text_problems(text: str) -> list[tuple[int, str]]:
    return [
        (number, name)
        for number, line in enumerate(text.splitlines(), 1)
        for name, pattern in TEXT_RULES.items()
        if pattern.search(line)
    ]


def files(root: Path) -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=root, capture_output=True, check=True,
    )
    return sorted({Path(p.decode("utf-8")) for p in result.stdout.split(b"\0") if p})


def self_test() -> None:
    probes = {
        "private-home-path": "/home/" + "example/tars/projects/private.md",
        "private-service-path": "/var/lib/" + "tars/example/data",
        "private-tailnet": "https://" + "node.example.ts" + ".net/",
        "private-domain": "https://" + "ash" + "-os.io/",
        "private-source-repository": "https://github.com/" + "cottrellashley/" + "tars-setup/",
        "private-network-address": "100." + "100.100.100",
        "private-key": "-----BEGIN " + "PRIVATE KEY-----",
        "github-token": "ghp_" + "x" * 30,
        "provider-key": "sk-" + "x" * 35,
        "cloud-access-key": "AKIA" + "X" * 16,
        "oauth-token": "ya29" + "." + "x" * 35,
        "credential-value": "password=" + "x" * 25,
        "embedded-basic-auth": "https://example:" + "secret@example.invalid/",
    }
    for rule, value in probes.items():
        assert any(name == rule for _, name in text_problems(value)), rule
    assert not text_problems("https://github.com/cottrellashley/tars-kb")
    assert not text_problems("https://example.invalid/source")
    assert not text_problems("2026-10-16T12:00:00Z")
    assert path_problem(Path("data/memory/assertions/example.json")) is None
    assert path_problem(Path("runtime/state.json")) is not None
    assert path_problem(Path("fixture/database.sqlite")) is not None
    print("Public scanner self-test passed.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    root = args.root.resolve()
    errors = 0
    checked = 0
    for relative in files(root):
        path = root / relative
        if not path.exists() and not path.is_symlink():
            continue  # A staged deletion has no publication contents.
        issue = path_problem(relative)
        if issue or path.is_symlink():
            print(f"{relative}: {issue or 'symlinked-public-artifact'}", file=sys.stderr)
            errors += 1
            continue
        if path.stat().st_size > 10 * 1024 * 1024:
            print(f"{relative}: oversized-public-artifact", file=sys.stderr)
            errors += 1
            continue
        try:
            content = path.read_text(encoding="utf-8")
            if "\0" in content:
                raise UnicodeError("NUL in text")
        except (UnicodeError, OSError):
            print(f"{relative}: unreadable-or-binary-artifact", file=sys.stderr)
            errors += 1
            continue
        checked += 1
        for line, rule in text_problems(content):
            print(f"{relative}:{line}: {rule}", file=sys.stderr)
            errors += 1
    if errors:
        print(f"Public publication check failed: {errors} finding(s).", file=sys.stderr)
        return 1
    print(f"Public publication check passed: {checked} text files; synthetic content only gate.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
