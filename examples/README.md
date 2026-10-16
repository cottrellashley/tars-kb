# Try the synthetic library

The sample is fictional. It contains two prose concepts, measurement and a
demonstration garden, plus an overview. Input fixtures live outside the canonical
partition so they are not mistaken for accepted memory.

Run from a clone with `tars-kb` installed, or build a release executable and put its
directory on `PATH`. The following commands make a disposable authoring copy:

```bash
DEMO_LIBRARY="$(mktemp -d)"
cp -R examples/library/. "$DEMO_LIBRARY"
tars-kb --root "$DEMO_LIBRARY" list --json
tars-kb --root "$DEMO_LIBRARY" validate sample --json
tars-kb --root "$DEMO_LIBRARY" read sample measurement --json
```

## Concepts and assertions

Initialize the optional memory profile, then register concepts that point to the
existing explanatory pages:

```bash
tars-kb --root "$DEMO_LIBRARY" memory init sample
tars-kb --root "$DEMO_LIBRARY" concept init sample measurement \
  --input examples/inputs/concept-measurement.json
tars-kb --root "$DEMO_LIBRARY" concept init sample garden \
  --input examples/inputs/concept-garden.json
tars-kb --root "$DEMO_LIBRARY" concepts sample --json
```

The base already declares two synthetic sources. Their URIs identify evidence; no
provider or network access is involved. Add four different temporal examples:

```bash
tars-kb --root "$DEMO_LIBRARY" remember sample measurement \
  --input examples/inputs/claim-timeless.json --idempotency-key demo-definition
tars-kb --root "$DEMO_LIBRARY" remember sample garden \
  --input examples/inputs/claim-observed.json --idempotency-key demo-observation
tars-kb --root "$DEMO_LIBRARY" remember sample garden \
  --input examples/inputs/claim-temporary.json --idempotency-key demo-interval
tars-kb --root "$DEMO_LIBRARY" remember sample garden \
  --input examples/inputs/claim-unknown.json --idempotency-key demo-unknown
tars-kb --root "$DEMO_LIBRARY" facts sample garden --json
```

Repeating an identical operation with its original idempotency key returns the
accepted receipt. Reusing the key for a different payload fails. This does not make
two different keys for the same assertion ID acceptable duplicates.

## Ask a precise question

```bash
tars-kb --root "$DEMO_LIBRARY" facts sample garden \
  --valid-at 2026-10-18 --view applicable --json
tars-kb --root "$DEMO_LIBRARY" facts sample garden \
  --valid-at 2026-10-24 --view applicable --json
tars-kb --root "$DEMO_LIBRARY" facts sample garden --view last-observed --json
tars-kb --root "$DEMO_LIBRARY" fact sample plot-count --evidence --json
tars-kb --root "$DEMO_LIBRARY" search sample 'duration' --mode ranked --verified --json
tars-kb --root "$DEMO_LIBRARY" context sample --query 'watering trial' --concept garden --json
```

An interval is half-open: its `from` boundary is included and its `until` boundary is
excluded. The observed plot count is point evidence and the unknown-time assertion
is not silently promoted to a current valid-time answer. Consult response metadata
for excluded or indeterminate records. Read the full
[memory specification](../specs/memory.md) before relying on historical queries.

## Correct an assertion

First read `plot-count` and copy its returned `revision` into `REVISION` below. The
correction fixture changes a transcription error concerning the same observation;
it does not describe a new observation or a change in the fictional world.

```bash
tars-kb --root "$DEMO_LIBRARY" fact sample plot-count --evidence --json
REVISION='<revision returned by fact>'
tars-kb --root "$DEMO_LIBRARY" revise sample plot-count --mode correct \
  --expect-revision "$REVISION" --input examples/inputs/claim-correction.json \
  --idempotency-key demo-correction
tars-kb --root "$DEMO_LIBRARY" history sample plot-count --json
```

The stable assertion retains its accepted revisions and source snapshots. A stale
expected revision fails without overwriting a concurrent correction. Use `change`,
`reconfirm` or `retract` only with the corresponding input shape and semantics.

## Validate and export

```bash
tars-kb --root "$DEMO_LIBRARY" validate sample --json
```

Choose a new export directory outside the library:

```bash
DEMO_EXPORT_PARENT="$(mktemp -d)"
DEMO_EXPORT_PARENT="$(cd "$DEMO_EXPORT_PARENT" && pwd -P)"
tars-kb --root "$DEMO_LIBRARY" export sample \
  --output "$DEMO_EXPORT_PARENT/reading" --format markdown
```

Export refuses an existing destination. Review the selected partition's complete
manifest, pages, assets and checksums. An export is a derived reading artifact; edit
the canonical files through the authoring workflow.

Do not run mutations directly against the checked-in example when using it as a
fixture. Keep generated `.runtime/` state, indexes and export directories out of the
public source tree.
