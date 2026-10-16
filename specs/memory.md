# Tars memory profile 1.0

This profile is `tars-memory/1.0`, independently versioned from storage contract
`1.0`, the CLI output protocol and the executable. It adds curated concepts and
sourced assertions as declared JSON assets. It does not own tasks, account messages
or original documents, and never authorizes executing knowledge text. All examples
in this repository are synthetic.

The reference shapes are in [memory.schema.json](../schemas/memory.schema.json).
Runtime validation additionally checks references, date/time semantics, transaction
order and accepted revision hashes. Unsupported profile versions or required features
are rejected. Unknown optional metadata is preserved; a correction merges the supplied
fields into the previous content instead of discarding unknown fields.

## Canonical files and identity

```text
bases/<base>/knowledge-base.json
bases/<base>/data/memory/profile.json
bases/<base>/data/memory/concepts/<concept-id>.json
bases/<base>/data/memory/assertions/<assertion-id>.json
```

Every file is declared in the v1 manifest. Profile initialization is explicit and
does not overwrite an existing unsupported or undeclared profile. IDs are local to
the selected base. Reading a link to another base never retrieves its content.

The profile contains an inventory of concepts and assertions, a strictly increasing
transaction `sequence`, local acceptance timestamps, transaction results/digests and
idempotency receipts. Each assertion stores its complete immutable accepted revision
history inside its own JSON file. Current state is its last revision. Retraction
creates a retained revision; it never deletes the earlier assertion.

This v1 layout deliberately rewrites the profile/inventory and, for new files, the
manifest. It does not claim constant-time creation or unbounded history capacity.
Large libraries require measured limits and a separately versioned storage evolution.
The derived search index can be deleted without losing any of this history.

## Concepts and assertions

A concept requires a title and short authored summary. Optional aliases, parents,
related concepts and page IDs default to empty arrays. Referenced concepts/pages must
already exist or have been created earlier in the same memory batch. Concept creation
does not infer an ontology or merge aliases across bases.

An assertion requires:

```json
{
  "id": "synthetic-port",
  "concepts": ["routing"],
  "text": "The synthetic service is configured for port 4100.",
  "basis": "reported",
  "subject": "service:example",
  "predicate": "port",
  "value": 4100,
  "cardinality": "single",
  "scope": {"environment": "synthetic"},
  "temporal": {
    "kind": "interval",
    "from": {"type": "instant", "value": "2026-01-01T00:00:00Z"},
    "until": null
  },
  "sources": ["synthetic-report"]
}
```

`basis` is `definition`, `observed`, `reported`, `decision`, `requirement` or
`inferred`. It describes the claim's basis, independently of timing. Definitions
and requirements need sources too. Scope may identify a specification and exact
version, environment or other assumptions. A specification requirement and observed
noncompliance coexist; neither overwrites the other.

The structured subject/predicate/value fields are optional. Free text remains useful;
the tool does not claim to detect contradictions in arbitrary prose. Optional
`assessment` metadata can describe uncertainty and its reason. It is returned without
inventing numeric probabilities or ranking truth by recency. `review_after` is an
explicit instant for evidence review, not automatic expiry or permission to rescan.

Every accepted revision captures the full source metadata it cited. Updating or
removing a current source description does not rewrite its historical snapshot.
Sources are references, not automatic retrieval authority. No URI is fetched.

## Five temporal forms

```json
{"kind":"timeless"}
{"kind":"observed","at":{"type":"instant","value":"2026-01-16T12:00:00Z"}}
{"kind":"event","at":{"type":"date","value":"2026-01-16","time_zone":"Europe/London"}}
{"kind":"interval","from":{"type":"date","value":"2026-01-20","time_zone":"Europe/London"},"until":{"type":"date","value":"2026-01-23","time_zone":"Europe/London"}}
{"kind":"unknown","description":"The original record did not give a date."}
```

`timeless` is deliberately not tied to calendar validity, such as a definition
under named assumptions. It does not mean a service address remains verified forever.
Missing temporal information is an error, never an implicit timeless or current fact.

An observation is evidence at a point; an event is an occurrence. Neither asserts
continued state. An observation dated January 16 cannot prove present health. Exact
future observation instants are rejected relative to local acceptance. Future planned
rules/events can be represented with their appropriate basis and scope.

Intervals are inclusive at `from`, exclusive at `until`: `[from, until)`. Both endpoints
must use the same precision and timezone basis. A required `until: null` means an
explicitly asserted open end. It does not mean an unknown historical end date. Partial
or approximately known bounds use `unknown` plus their original description in 1.0.

An instant is offset-qualified RFC 3339 with seconds and is stored in UTC; normalized
inputs retain a differing original spelling in `source_value`. Leap-second `:60`
input is rejected explicitly. A date remains a Gregorian calendar date. A named IANA
zone is optional for a date label, but required to compare a calendar interval with
an instant. Such comparisons convert the query instant to the interval's local date;
they do not fabricate a midnight or assume every day has 24 hours. Missing zones or
mixed precision can make comparison indeterminate. No ambient machine timezone is
silently substituted. Relative language and local datetimes without offsets are not
parsed by this profile; callers must resolve them deliberately before writing.

## Valid time and knowledge time

`recorded_at` is generated by the local accepting writer and cannot be supplied in
normal assertion input. Publication time and observation time remain separate.
Multiple revisions in a batch share one acceptance sequence and timestamp.

`--valid-at` selects applicable intervals and timeless statements. Point observations
and events are excluded **even at exactly their point**: they are evidence, not
continuous applicability. Unknown/indeterminate timing is excluded and counted
separately. `--view last-observed` requests observations explicitly; with a valid-time
cutoff it excludes observations after that point. It retains equal-time conflicting
observations and incomparable date/instant records rather than inventing an order.

Without a valid-time filter, `all` returns current accepted, nonretracted assertions
with their temporal labels; it does not claim that every returned statement is current
real-world state. `last-observed` groups only records with comparable structured
subject/predicate/scope/basis. Unstructured observations remain separate.

`--known-at` reconstructs accepted knowledge by local recording time. `--known-revision`
uses the per-base sequence instead. They are mutually exclusive. Dates before the
profile's `history_started_at`, or future revision numbers, return
`history_unavailable`. Recording time is not inferred from filesystem timestamps or
optional Git history.

Clock regressions are retained in transaction metadata. A wall-time query within a
regressed range returns `clock_regression`; it must use an explicit known revision
instead. A sequence orders commits but cannot manufacture trustworthy UTC precision.

Freshness is explicitly evaluated at the response's `freshness_evaluated_at` time.
`review_due` means review is due, not that a past observation was false. These labels
remain attached to historical retrieval and do not redefine validity.

## Revision operations

Every revision requires the exact assertion revision hash previously read.

| Mode | Input | Meaning |
|---|---|---|
| `correct` | `{"reason":"…","content":{"value":4200}}` | Earlier account was wrong; merge corrected fields into a new revision under the same ID. |
| `change` | `{"effective_at":<point>,"replacement":<new assertion>,"reason":"…"}` | Real change: atomically close the old interval and create a new assertion whose interval starts at the effective point. |
| `reconfirm` | `{"replacement":<new observed assertion>}` | Add a later observation with a new ID, preserving the old observation's timestamp/history. |
| `retract` | `{"reason":"…"}` | Withdraw support while retaining the original value and history; never assert its opposite. |

A correction may explicitly list optional keys in `remove_fields`, for example
`{"reason":"Obsolete review policy","remove_fields":["review_after"]}`. Required
identity/content fields cannot be removed. Other optional metadata remains preserved.

Change requires an old interval and a strictly later effective point within it. The
replacement uses a new ID, preserves subject/predicate/scope, and declares its own
end. Both revisions are accepted in one transaction. Transaction results retain the
old/replacement relationship. An effective point equal to the old start is a
correction case, not an empty historical interval. Recording a January 12 change on
January 16 leaves a January 15 known-at query using the former open interval; today's
knowledge uses the closed old interval plus its replacement.

Reconfirmation requires unchanged subject/predicate/scope/value and a later comparable
observation time when the old assertion was an observation. A changed measurement is
new evidence rather than a reconfirmation of the same value. Retraction cannot be
silently undone by revision; reassert explicitly under a new ID with its evidence.
Reconfirmation lineage must be acyclic and use existing same-base targets. General
`related`, `supports` and `disputes` links can be reciprocal; they are not lifecycle
supersession and do not choose a winning assertion.

Competing assertions are not silently superseded. Mechanical conflict candidates
require `cardinality: single`, matching structured keys and comparable overlapping
times, with different values. Definition, requirement and descriptive evidence bases
are kept distinct. Multiple-valued facts coexist. No general prose theorem prover
or automatic conflict-resolution claim is made.

## Atomic batches, idempotency and preservation

Memory batches contain 1–1000 ordered `concept-init`, `remember` or `revise`
operations for one base. They may refer to earlier creations in the same batch.
Validation completes in memory before the cooperating durable commit writes any
files. A failed operation leaves the entire batch uncommitted. Source/page operations
have their separate writers; 1.0 memory batches do not pretend to include them.

An optional idempotency key binds the exact structured operation digest. Repeating
the same request returns its original durable receipt. A different payload with that
key fails. The profile, changed records and any required manifest registration are
committed together using the core filesystem transaction protocol. No derived index
is an alternative authority, and revision hashes establish accepted-byte consistency,
not independent factual correctness or protection from a fully compromised owner.

A faithful export/restore carries all profile metadata and assertion revisions,
including retractions, source snapshots, clock warnings and history coverage. It
preserves acceptance timestamps and sequences, so historical answers remain portable.
A foreign-claim ingest is a different operation: normal `remember` creates new local
acceptance metadata while upstream dates belong in provenance. This release has no
general merge/import operation that silently reconciles two histories.

Full validation checks inventory declarations, source snapshots, revision hashes,
transaction sequences and accepted clocks. Cooperative read/write locks and CAS
protect ordinary local agents. They are not authorization against an arbitrary
process that already controls the owner account. See the storage specification for
crash recovery and consistent export boundaries.
