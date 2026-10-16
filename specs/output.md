# CLI output protocol 1

The machine protocol version is `1.0`. It is independent of the executable version,
portable storage contract and optional memory profile. Use the installed
`capabilities`, `spec` and `schema` commands to discover support.

## Selection and compatibility

`--json` emits machine-readable JSON on standard output. Human diagnostics belong on
standard error, and a failed operation has a nonzero exit status. Global `--root` and
`--json` can appear anywhere in the command. A content operation selects exactly one
base; a response does not silently broaden that scope.

The default legacy `list`, `show` and `search` JSON forms retain their compatibility
shapes where the compatibility path applies. New bounded/progressive responses carry
`output_version: "1.0"` alongside their operation-specific fields. There is no
universal `items` envelope imposed on all operations: consumers must respect the
documented payload (`items`, concept inventory, history, a page or one fact).

Do not parse human tables or prose as a stable API. Preserve unknown additive output
fields when forwarding data; reject an unsupported major protocol before relying on
its meaning.

## Bounded results

Progressive collection operations accept `--limit`, `--max-bytes` and `--cursor` where
shown by their help. Results state whether they are truncated and supply
`next_cursor` when continuation is available. A cursor is opaque and binds the
operation's scope, selection and revision; do not modify it or reuse it after a
different query/base/accepted revision.

Byte limits constrain serialized JSON payload bytes, not just the text excerpts;
the terminating output newline is separate.
Bounded responses use compact JSON even without `--json`, so pretty-printing does
not silently exceed the selected budget.
Exact page/section and single-fact reads do not return an arbitrary partial record
when the budget is too small. They fail with `output_limit` and required-size
information so the caller can deliberately increase the budget or narrow selection.
An oversized collection item can instead produce an empty truncated collection with
`required_item_bytes`; increase the budget before retrying rather than interpreting
the empty page as no matching knowledge.

Keep the selected revision and consistency metadata with retrieved material. A page
digest identifies bytes; a memory sequence identifies an accepted transaction;
an assertion revision identifies one accepted record version. They are not
interchangeable.

## Errors

For a parsed command, machine operation errors retain an error message and category,
for example:

```json
{"error":"conflict: revision changed","code":"conflict"}
```

Use `code` for control flow and the message for explanation. Error text may become
more precise in later releases. Typical categories include invalid input, unknown
identity, unsafe path, unsupported version/profile, conflict, busy, output limit,
unavailable history and recovery required. Inspect actual capabilities/help for the
operation rather than assuming every category applies to every command.
Command-line grammar errors, help and version output are produced by the argument
parser and can be plain text even when `--json` was supplied; they are not successful
operation payloads.

An error is not a successful empty result. A recovery or revision error must not be
handled by silently retrying an unconditional write. No response can establish that
an assertion is true merely because its JSON structure is valid.
