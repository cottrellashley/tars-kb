# Local storage, revision checks and recovery

This document describes the Rust implementation's local transaction protocol. It is
separate from the portable content contract and temporal memory profile. Internal
journal formats are implementation details; portable exports do not depend on them.

## Canonical and derived material

```text
library/
├── catalog.json
├── bases/
│   └── <base-id>/
│       ├── knowledge-base.json
│       ├── pages/
│       ├── data/
│       └── assets/
└── .runtime/
    ├── locks/
    └── transactions/
```

The catalog, manifests and declared content are canonical. Per-base locks and
transaction journals are under the library's `.runtime/` directory. A journal can
contain old and proposed content, so it is protected state even though it is not
authored knowledge. Search indexes and caches are disposable derived data; their
location and invalidation rules are documented separately.

Lock identity is tied to the canonical library and base, not a caller-selected cache
directory. Two cooperating invocations with different cache settings must still use
the same lock. The implementation uses advisory Unix file locking and a bounded wait;
it returns a busy error when it cannot acquire the required lock in time.

## Read and write lifecycle

Opening a base takes a shared lock and checks for pending work. Only pending recovery
releases that lock and takes exclusive ownership before rechecking/recovering. Normal
readers keep their shared lock for the operation and can run concurrently. A mutation
releases its shared lock, takes exclusive ownership and checks expected revisions
before changing files.

Revision checks are compare-and-swap conditions. A matching fact/page identifier does
not imply the caller saw the current revision. A conflict leaves current owner edits
intact; callers reload and decide whether to retry. Do not turn a conflict into an
unconditional overwrite automatically.

Each successful base write also advances the additive manifest field
`x-tars-kb-revision` in the same transaction, including writes that only revise an
existing memory record. This generation marker lets readers detect known changes
without scanning all assertion bodies. It does not auto-increment the author's
semantic content version or turn a manifest digest into a fact's revision hash.
Manual out-of-band edits do not participate in this protocol; use verified retrieval
when current byte-level freshness is required.

JSON parsing rejects duplicate keys and nonfinite numbers. The implementation limits
individual file size and checks relative path boundaries. Unknown optional metadata
is preserved during supported edits. Unsupported storage/profile versions are
rejected before writing.

## Journaled transaction

The writer validates every intended path and expected current byte digest. It writes
and syncs a prepared journal carrying the old/new content, then a durable
`.tars-kb-pending` marker inside the base. It applies the file replacements, writes a
committed journal state, and finally removes the marker and journal. File replacements
use private temporary files, atomic rename and directory syncing.

Recovery under the base lock follows the journal phase:

- A prepared transaction restores its old content and removes newly created empty
  directories where safe.
- A committed transaction completes the new content.
- A file that matches neither recorded version is an external edit conflict;
  recovery stops and preserves evidence rather than overwriting it.
- A pending marker with no matching journal fails with `recovery_required`.

The temporary pending marker is a transaction sentinel, not an authored asset. A
stable validated/exported partition cannot silently carry an unresolved marker.
Copying a base during a write can lose the external journal; the marker makes that
case visible rather than allowing the copied partial state to appear clean.

Atomic rename is a per-file primitive. The journal, locks and recovery checks make a
multi-file operation recoverable for cooperating clients; they do not turn arbitrary
filesystem readers into transactional clients. Durability depends on the actual OS,
filesystem and storage device honoring the relevant operations.

## Backups and copies

Quiesce writers and let pending operations recover/complete before copying a library
or taking a portable export. For a consistent stopped library backup, include its
canonical files and any recovery state that still exists. Prefer a clean completed
transaction boundary. Do not discard `.runtime/transactions/` while a pending marker
exists in a base.

A normal portable export contains only the clean selected partition and integrity
metadata, not locks, journals or indexes. A file-manager copy, Git checkout or sync
tool is not a replacement for transaction-aware export during active writes.

Journals recover interrupted operations. Independent versioned backups recover lost
or unwanted operations. Checksums establish byte integrity, not factual accuracy,
access authorization or absence of malicious content.

## Platform boundary

The initial implementation targets local Unix filesystems on Linux; macOS has a
compatibility CI job. Release artifacts are Linux-first. A green compile alone is
not evidence of correct concurrent writes, crash recovery or filesystem durability
on another platform. Windows support and distributed writers are outside this
protocol's current scope.

On macOS, directory synchronization calls `libc::fsync` explicitly and propagates
errors, retrying interruption. Regular file writes retain `File::sync_all`.
The [Rust 1.99.1 implementation](https://github.com/rust-lang/rust/blob/1.99.1/library/std/src/sys/fs/unix.rs)
uses Apple's `F_FULLFSYNC` for `File::sync_all` without a fallback; the directory path
is kept separate rather than ignoring a failed full-flush request. This source-based
portability adjustment is not a claim of a power-loss durability test.
