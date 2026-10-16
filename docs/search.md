# Scoped search and derived indexes

Literal search remains the compatibility default. It searches declared Markdown
page lines using full Unicode case folding and substring matching, and returns the
existing hit-array shape. For example, `STRASSE` matches `Straße`. It does not read
sibling bases, memory records, opaque attachments or evidence documents.

The bounded retrieval API additionally supports current nonretracted memory
assertions. It preserves each assertion's concept, basis, temporal shape, scope,
sources, revision, observation/freshness label and conflicts. An observation hit is
historical evidence, not proof that the state remains true. Search does not fetch
source URIs or execute content. A concept filter selects explicit concept membership.

Ranked search is explicit. Ordinary query words are quoted as data, combined with
AND, and passed through bound SQL parameters. SQLite FTS5 ranks IDs/titles/headings
and the authored titles/aliases of an assertion's concepts
ahead of body text using weighted BM25. This is lexical matching, not the literal
substring contract. No advanced query language, remote model or embedding service
is enabled implicitly.

## Index lifecycle

Each canonical library location and selected base has a separate SQLite cache:

```text
$XDG_CACHE_HOME/tars-kb/<canonical-library-hash>/<base-id>/search-v1.sqlite3
# Without XDG_CACHE_HOME:
~/.cache/tars-kb/<canonical-library-hash>/<base-id>/search-v1.sqlite3
```

Cache namespace directories are mode 0700 and index files are 0600. The cache must
be outside the canonical library. A per-index lock coordinates search/update/rebuild
processes. Indexes use a fixed versioned schema/parser identity; incompatible or
corrupt indexes fail visibly and can be explicitly rebuilt. Rebuild constructs a
replacement privately and publishes it under the index lock. No canonical file is
changed by indexing. Losing a cache never loses a knowledge record.

Cache-lock contention currently waits for the active index operation to finish;
there is no separate cache-lock timeout. This can add latency while a rebuild runs.

Index update reads declared search inputs and hashes their bytes. Changed document
rows are replaced in one SQLite transaction, unchanged rows remain, and documents
that have been removed, undeclared or retracted are deleted. The source signature
covers the manifest, declared pages, supported memory profile, concepts and assertions;
it does not claim to cover unsearched opaque attachments. Updates are incremental
at the index-row level; the input scan itself remains proportional to selected
source content. There is no watcher or assertion that file timestamps prove freshness.

## Consistency and budgets

An ordinary ranked query requires an existing index matching the accepted manifest
revision. Native writes advance that manifest revision; a known old index returns
`index_stale` until explicit update or `--verified` refresh. An accepted matching
index reports
`indexed_snapshot_unverified`, its source signature, manifest revision and indexing
time. It does not claim the working files or time-dependent review labels are fresh.
`--verified` scans the selected sources and updates the index before querying, then
reports `verified_source_snapshot`. This verifies the captured input bytes, not that
an uncooperative editor cannot change a file immediately afterward. The base lock
coordinates participating native writers; it does not lock ordinary editors.

Missing declared files or invalid memory cause errors rather than a complete empty
answer. A corrupt index is an error until explicitly rebuilt. A removed file must
also be removed from its canonical inventory; deleting only the file leaves an
invalid source and cannot be silently treated as an accepted deletion.

Bounded retrieval defaults to eight hits and an 8192-byte compact JSON budget.
Complete hit records retain qualifiers together. If a record cannot fit, a reference
and required item size are returned where possible and `truncated` is true; the tool does not return its claim without time or
conflict qualifiers. Budgets below the envelope size fail explicitly. Context packs
the same existing evidence and references; it never invents an answer. Stable search
cursors are not implemented yet; the response says to increase the limit to expand
the snapshot. Existing legacy search output remains unchanged.

No performance numbers are claimed here. Measure process startup, direct reads,
input scanning/index construction, incremental row updates, ranked queries and JSON
serialization separately using synthetic data, pinned builds and explicit cache states.
