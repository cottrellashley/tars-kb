# CLI workflows

Use `tars-kb --help` and the relevant subcommand's `--help` for the installed version.
Global `--root` selects a library and `--json` selects machine output; both are
accepted anywhere in the command. Content operations always name one base.

The examples use `./library` as a generic local destination. For a runnable fixture,
follow [the synthetic walkthrough](../examples/README.md). Nothing in this document
grants permission to read a different base or execute retrieved examples.

## Create and inspect a base

```bash
tars-kb --root ./library init research --title 'Research notes' --owner 'Example author'
tars-kb --root ./library list --json
tars-kb --root ./library show research --json
tars-kb --root ./library validate research --json
```

Initialization creates a minimal entry page, manifest and catalog registration. It
refuses to overwrite an existing base and preserves unrelated registrations. A base
content version is distinct from the storage contract version.

`show BASE [PAGE]` retains the full-page compatibility operation. `read` is intended
for bounded agent reads with revision metadata. Page IDs are stable manifest IDs,
not filenames or titles.

```bash
tars-kb --root ./library outline research overview --json
tars-kb --root ./library read research overview --json
tars-kb --root ./library read research overview --section introduction --json
tars-kb --root ./library show research overview --with-revision --json
```

Select a section slug returned by `outline`; repeated headings have distinct suffixes.
An exact selected page/section that cannot fit the requested response budget produces
an `output_limit` error with required-size information rather than silently cutting
the middle out of an authoritative record.

## Write and revise prose

Prepare Markdown in an ordinary local file, then create a declared page:

```bash
tars-kb --root ./library page create research measurements \
  --title 'Measurement methods' --input ./measurements.md
tars-kb --root ./library show research measurements --with-revision --json
```

Use the returned page revision as the compare-and-swap precondition:

```bash
tars-kb --root ./library page replace research measurements \
  --expect-revision '<current revision>' --input ./replacement.md
```

For a small exact change, patch input is JSON:

```json
{"find":"The original sentence.","replace":"The corrected sentence."}
```

```bash
tars-kb --root ./library page patch research measurements \
  --expect-revision '<current revision>' --input ./patch.json
```

The `find` text must match exactly once. Zero or multiple matches fail rather than
guessing. `edit BASE PAGE` opens a temporary draft using the owner's selected editor
and checks the original revision before committing. Do not let page content choose
an editor command or run an executable.

Create/replace/patch validate the proposed page and same-base incoming heading links
before committing. To rename a referenced heading across several pages, first retain
the old heading while adding the new one, update incoming links, then remove the old
heading after nothing references it. A bad candidate leaves the current page and
manifest unchanged.

## Register provenance

Source input follows the base contract:

```json
{
  "id":"example-observation",
  "title":"Fictional experiment log",
  "type":"observation",
  "uri":"observation://synthetic-example/experiment",
  "checked_at":"2026-10-16"
}
```

```bash
tars-kb --root ./library source add research --input ./source.json
```

Source registration stores provenance. It does not fetch a URI, check a provider
account or prove that every claim citing the source is correct. Assertion revisions
snapshot the selected source metadata so a later source edit does not rewrite what
evidence metadata was accepted earlier.

Commands with `--input` also accept `--input -` to read from standard input. When the
operation expects JSON, the document must be complete and contain no duplicate keys.

## Initialize concepts and remember assertions

```bash
tars-kb --root ./library memory init research
tars-kb --root ./library concept init research measurement \
  --title 'Measurement' --summary 'Definitions and experiment observations.'
tars-kb --root ./library concepts research --limit 8 --json
tars-kb --root ./library concept get research measurement --json
```

Concept input can also be supplied with `--input`. It can declare aliases, parent
and related concept IDs, and associated page IDs. Those are same-base references;
they must resolve to existing registered entities where required.

A minimal assertion input is:

```json
{
  "id":"duration-definition",
  "concepts":["measurement"],
  "text":"A duration measures elapsed time between two instants.",
  "basis":"definition",
  "temporal":{"kind":"timeless"},
  "sources":["example-observation"]
}
```

```bash
tars-kb --root ./library remember research measurement \
  --input ./assertion.json --idempotency-key definition-import-1
tars-kb --root ./library facts research measurement --limit 8 --json
tars-kb --root ./library fact research duration-definition --evidence --json
```

The optional positional concept must agree with the assertion's declared concepts;
it does not implicitly move a fact. Sources, concepts and temporal information are
validated before acceptance. `recorded_at`, accepted sequence and revision are
writer-generated and cannot be supplied as an imported assertion's recording history.

Use [the memory profile](../specs/memory.md) for the complete temporal and revision
semantics. In particular, timeless is an explicit classification, not proof that a
claim can never change.

## Query valid time and recorded knowledge

```bash
tars-kb --root ./library facts research measurement --view all --json
tars-kb --root ./library facts research measurement \
  --valid-at 2026-10-16 --view applicable --json
tars-kb --root ./library facts research measurement --view last-observed --json
tars-kb --root ./library facts research measurement --known-revision 4 --json
tars-kb --root ./library facts research measurement \
  --known-at 2026-10-16T12:00:00Z --json
```

`valid-at` asks about the assertion's subject. `known-at` or `known-revision` selects
the accepted recording history. `known-revision` is a base memory transaction
sequence, not an assertion's content hash. Do not provide both historical selectors.

Without a valid-time filter the default `all` view does not silently equate every
record to current truth. The `applicable` view needs a valid-time point; point
observations and unknown-time statements do not become enduring intervals.
`last-observed` keeps relevant evidence and ties rather than claiming a present fact.
Read the exclusion/indeterminate and freshness information with the returned claims.

Use explicit time helpers when translating user phrasing into a structured point:

```bash
tars-kb time normalize 2026-10-16T13:00:00+01:00 --json
tars-kb time resolve yesterday --zone Europe/Paris \
  --reference 2026-10-16T12:00:00Z --json
tars-kb time resolve 2026-10-16T09:00:00 --zone UTC --json
```

Relative dates require both an explicit reference instant and time zone. Local times
need a zone; ambiguous daylight-saving folds and nonexistent local times are rejected
rather than guessed. Use an offset-qualified instant when disambiguation is needed.
Normalization does not convert an unknown observation date into the recording time.

## Correct, change, reconfirm or retract

Read the assertion, then use its current revision:

```bash
tars-kb --root ./library fact research duration-definition --evidence --json
tars-kb --root ./library revise research duration-definition --mode correct \
  --expect-revision '<current revision>' --input ./corrected-assertion.json \
  --idempotency-key definition-correction-1
tars-kb --root ./library history research duration-definition --json
```

`correct` requires a reason and merges supplied `content` fields into a retained new
revision of the same logical assertion, preserving optional metadata not overwritten.
`change` closes an interval and creates a new assertion for the changed world.
`reconfirm` records new evidence under a new assertion ID; it does not retroactively
extend the old observation. `retract` preserves history while excluding the retracted
record from ordinary fact sets. Their different input shapes are defined in
[memory](../specs/memory.md).

A revision conflict requires reloading and deciding how to proceed. Repeated identical
requests with the same idempotency key return the original accepted receipt; the same
key with a different payload fails.

## Batch and retrieve bounded context

A memory batch accepts an array of operations, or an object with `operations`:

```json
{
  "operations":[
    {"op":"remember","input":{"id":"example","concepts":["measurement"],"text":"Synthetic example.","basis":"reported","temporal":{"kind":"unknown","description":"The fictional source gives no date."},"sources":["example-observation"]}}
  ]
}
```

```bash
tars-kb --root ./library batch research --input ./batch.json \
  --idempotency-key import-batch-1
tars-kb --root ./library search research 'elapsed time' \
  --mode ranked --verified --limit 8 --max-bytes 8192 --json
tars-kb --root ./library context research --query 'elapsed time' \
  --concept measurement --limit 4 --max-bytes 8192 --json
tars-kb --root ./library index research --rebuild --json
```

The memory batch is one accepted transaction; an invalid operation does not commit a
partial prefix. It is not an arbitrary shell or cross-base transaction interface.
`remember`, `revise` and `batch` accept `--dry-run` to validate a proposal in an
isolated temporary copy. A preview receipt is not accepted source history; the actual
write can still conflict if another writer changes the base before application.

Literal/ranked search and context modes are described in [search](search.md). Check
whether a response is a verified current read or an indexed snapshot. Bounded results
include continuation metadata; treat cursors as opaque and reload if their scope or
revision no longer matches. See [output](../specs/output.md).

## Export and introspection

```bash
tars-kb --root ./library validate research --json
tars-kb --root ./library export research --output ./research-reading --format markdown
tars-kb capabilities --json
tars-kb spec show storage
tars-kb spec show memory
tars-kb schema show knowledge-base
```

Exports select one clean partition, refuse an existing destination and preserve
declared assets/provenance without fetching sources or importing sibling bases.
Combined Markdown/HTML editions include a page table of contents and anchors scoped
to each page; same-base page/heading links are rewritten to those anchors, and asset
links point to the carried files. The original page tree remains byte-for-byte
unchanged. The combined edition normalizes link labels and presents images as links;
the original source pages retain their original Markdown.
Validation checks structure and supported invariants; it does not establish factual
correctness or permission to publish. Inspect the installed capabilities and schemas
rather than guessing support from a package version.

`recover BASE` explicitly invokes the same interrupted-write check that normally runs
when opening a base. If recovery reports a missing journal or external edit conflict,
preserve the marker/journal and restore a consistent copy rather than removing the
sentinel. See [storage and recovery](../specs/storage.md).
