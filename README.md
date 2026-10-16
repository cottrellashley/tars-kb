# tars-kb

A local Rust CLI for portable knowledge bases and temporal agent memory. Keep
readable explanations in Markdown, precise assertions in JSON and each knowledge
base in its own explicitly selected namespace.

Progressive reads let an agent start with base metadata, select a concept, retrieve
bounded assertions and open evidence only when needed. The executable runs no model,
requires no server and does not treat retrieved text as permission to act.

## Install

From a clone of this repository, with the Rust/native build toolchain installed:

```bash
cargo install --path . --locked
tars-kb --help
```

The initial release targets Linux. Native Linux x86_64/aarch64 CI and a macOS
compatibility job are configured; inspect the actual workflow results for the chosen
revision. Release archives and checksum verification are described in
[releases](docs/releases.md). Windows is not currently supported.

## Start with a small, explicit read

The checked-in library is entirely synthetic:

```bash
tars-kb --root examples/library list --json
tars-kb --root examples/library read sample overview --json
tars-kb --root examples/library search sample 'duration' --limit 4 --json
```

Create a separate working copy before trying writes. The
[example walkthrough](examples/README.md) initializes two concepts, remembers dated
and timeless assertions, corrects a fact and queries historical validity.

```bash
tars-kb --root ./my-library init research --title 'Research notes' --owner 'Example author'
tars-kb --root ./my-library memory init research
tars-kb --root ./my-library concept init research measurement \
  --title 'Measurement' --summary 'Definitions and evidence about measurement.'
tars-kb --root ./my-library source add research \
  --input examples/inputs/source-notes.json
```

Supply a complete assertion as JSON; see
[the timeless input fixture](examples/inputs/claim-timeless.json) and
[memory specification](specs/memory.md). Register its source in the selected base
before remembering it.

```bash
tars-kb --root ./my-library remember research measurement \
  --input examples/inputs/claim-timeless.json --idempotency-key first-definition
tars-kb --root ./my-library concepts research --json
tars-kb --root ./my-library facts research measurement --json
tars-kb --root ./my-library fact research duration-definition --evidence --json
```

The bare `research` argument always selects one base. A link to another base does
not read or import it. Unknown concepts and sources produce errors rather than
silently creating a new destination or inventing provenance.

## Time and revisions

An assertion's recording time is different from when its subject was observed or
valid. Temporal kinds are `timeless`, `observed`, `event`, `interval` and `unknown`.
Calendar dates retain their precision and optional time zone; UTC instants remain
instants. Unknown time is not converted into “now.”

Facts can be queried by valid time and by what the base knew at an accepted sequence
or recording time. Point observations are evidence at a point, not an automatic
claim that the same value remains true indefinitely. Corrections, real-world
changes, reconfirmations and retractions have different revision behavior.

Revisions and source snapshots are stored in canonical files. An optional search
index is derived data; deleting it does not delete memory or its history. See
[memory semantics](specs/memory.md) and [search](docs/search.md).

## What ships

| Area | Operations |
|---|---|
| Library | Initialize, list, validate and export one selected base |
| Pages | Read full pages or bounded sections; create, replace, patch and edit with revision checks |
| Sources | Register named provenance without fetching it |
| Memory | Initialize concepts, remember assertions, query facts, revise, inspect history and batch writes |
| Retrieval | Literal/ranked scoped search, a rebuildable index and bounded context output |
| Integration | JSON output, capability/spec/schema discovery and explicit version contracts |

The storage contract, memory profile, output protocol and executable version are
separate. Existing v1 base manifests/pages remain usable; the memory profile is an
optional declared JSON asset set. See [CLI usage](docs/usage.md),
[content contract](specs/contract.md), [storage/recovery](specs/storage.md) and
[output protocol](specs/output.md).

## Local ownership and safety

This is a trusted-owner filesystem tool, not a multi-user authorization server.
Partition selection avoids accidental context mixing; operating system/service
permissions enforce who can actually read a base. No source URL is fetched and no
page example is executed as a side effect of retrieval.

Writes use advisory locks, revision checks and a recovery journal. Back up at a
quiescent transaction boundary; a journal is not an independent backup. Do not put
credentials, private originals or live application databases into versioned knowledge.
Read [security boundaries](docs/security.md) before exposing the tool to another actor.

## Development

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
python3 scripts/public-check.py --self-test
python3 scripts/public-check.py
```

Python is used by the repository's publication check, not by the installed executable.
Performance claims need measured release-build evidence; the
[benchmark methodology](docs/performance.md) distinguishes process startup, cache
state, index creation and bounded retrieval. No latency target is presented as a
measured result.

MIT licensed. All committed example content is fictional and suitable for public use.
