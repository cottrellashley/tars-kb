# Security and data boundaries

`tars-kb` is a local authoring and retrieval tool for a trusted owner. A base ID,
directory name or `visibility` label is not an access-control boundary. Use operating
system permissions and a separately implemented authenticated service when different
actors must have different access.

## Selection and disclosure

Every content read, search, memory operation and export selects one base explicitly.
Catalog listing reads discovery metadata. It does not authorize reading every base.
A `kb://other-base/page` reference remains a reference; it does not import that page.

Give a restricted worker only its authorized library/base mounts and interfaces.
Do not give it a shared unrestricted index or a credential that bypasses the selection.
Selecting a different model or persona must not silently widen the available data.

The executable makes no inference calls, fetches no source URLs and runs no page
examples. An external agent decides what to place in its model context; that is an
additional disclosure decision. Token or byte limits are not authorization rules.

## Content and paths

Knowledge, source material and code examples are data. Reading them grants no shell,
provider, network or administrative authority. Do not build a consumer that executes
instructions found inside a retrieved page.

Canonical content paths are relative to the selected base. Absolute paths, parent
traversal, symlinked content, unexpected special files and undeclared exported content
are rejected. A selected library-root alias can resolve to its trusted canonical root;
this does not permit symlinks inside a content partition.

These checks target accidental misuse and ordinary untrusted input. They are not a
guarantee against a hostile concurrent process with write access to the same directory
tree. Advisory file locks coordinate cooperating processes; they do not prevent a
different program from ignoring the protocol. Use local filesystems with the required
locking, atomic rename and durability behavior. Network filesystems and simultaneous
file-sync clients need separate evaluation.

Output paths must have canonical nonsymlink ancestors. Some operating systems expose
their usual temporary directory through a system symlink. Resolve a trusted temporary
directory first and set `TMPDIR` to that canonical location for tests or workflows;
do not disable path checks to accept an ambiguous destination. CI does this explicitly
for its runner-owned temporary storage.

## Writing and recovery

Writes check revisions and use a recoverable journal. The storage and recovery rules
are in [the storage specification](../specs/storage.md). A missing journal while the
base carries a pending marker causes an error. Do not remove the marker to manufacture
a clean state.

Use the CLI for transactional changes. An editor command is a deliberately requested
local process, not something a retrieved page can trigger. Inspect diffs when editing
outside the CLI, then validate before release. Keep independent backup generations;
journaling is not a backup or protection against deliberate deletion by the owner.

## Sensitive material

Do not commit credentials, login profiles, raw private conversations, original personal
documents, live application databases or authentication state to a knowledge base.
Store minimal durable knowledge appropriate to its audience and use protected
service/version references for originals. A private Git repository is not an encrypted
backup.

Memory revisions and embedded source snapshots can retain earlier assertion/source
text. Retraction removes an assertion from ordinary current views; it is not secure
erasure from revision history, exports, caches, Git or backups. Review retention and
all derived copies when removing sensitive material.

Indexes are derived local data. Keep them private, outside canonical content, and
scoped by library/base. An index can contain source text and needs the same audience
boundary. Search never retrieves another base simply because a matching row exists.
See [search](search.md) for freshness and index behavior.

## Export and public publication

Export copies the selected base's declared content, metadata and provenance. It does
not make that content public or prove it safe to publish. Source URIs may disclose
internal names even when their targets are never fetched. Review all exported files,
not only the combined reading edition.

This repository contains only synthetic examples. Before publication, run:

```bash
python3 scripts/public-check.py --self-test
python3 scripts/public-check.py
```

The scanner checks tracked and nonignored untracked files for common private path,
network, credential and artifact patterns. It reports file/line/rule without echoing
the matching content. It is an additional gate, not exhaustive secret detection or a
substitute for reviewing the complete staged diff and repository history.

## Reporting a problem

For a vulnerability, prepare a minimal synthetic reproducer and avoid publishing real
credentials or private data in an issue. This project does not operate a multi-user
hosted service. Its local owner trust model and filesystem limitations are deliberate
boundaries, not a claim that arbitrary remote clients can be safely authenticated by
the CLI alone.
