# Portable knowledge base contract 1

## Purpose and authority

A knowledge base is a named, versioned collection of readable pages and declared
assets. The contract makes it discoverable and usable by future readers, agents,
search tools, documentation viewers and export tools without depending on a chat
history or vendor database. It describes content, not execution permission.

The normative version is `schema_version: "1.0"`. This is the Tars KB contract,
not an externally standardized OKF implementation. Implementations MUST reject an
unsupported major version. Additive metadata MAY be ignored/preserved by v1 readers;
a breaking meaning or path/identity change requires a new major version. A base's
`version` is its own semantic content version, distinct from the contract version.

The words MUST, MUST NOT, SHOULD and MAY express requirements in this contract.
The original v1 reference CLI is a standard-library implementation for trusted local
authoring; it is not a multi-user authorization server or a guarantee against a hostile
concurrent filesystem writer changing paths during a read. Future services must bind
authorized immutable revisions or use descriptor-safe reads. JSON Schemas define
the interchange shapes. Structural validation is not verification of factual claims.

## Library discovery and partitions

A library root MUST contain `catalog.json` with `schema_version` and an ordered
`knowledge_bases` array. Each registration contains a unique base `id` and relative
`path`. The v1 canonical path is `bases/<id>`. The ID starts with a lowercase letter
and continues with lowercase ASCII letters, digits or hyphens.

Each base MUST contain `knowledge-base.json`. Its `id` MUST match the catalog.
Each base is a partition: IDs of pages, assets and sources are local to it. A tool
MUST select a base explicitly for content reads/search/exports, or require a separately
authorized explicit multi-base operation. Content search always requires a selected
base. Listing catalog metadata does not read all page content.

Content MUST NOT be inherited from sibling bases by default. A cross-base reference
is explicit and does not authorize retrieval or disclosure of its target. Filesystem
permissions and any future authenticated service enforce access; paths and visibility
fields alone are not an isolation boundary. Workers that cannot read a base must not
receive its source mounts, cached index or an API capable of bypassing that limit.

## Manifest

Required fields:

| Field | Meaning |
|---|---|
| `schema_version` | Contract version, currently `1.0` |
| `id` | Stable base namespace |
| `title`, `summary` | Human purpose and scope |
| `version` | Base content semantic version |
| `visibility` | `private` or `shareable`; classification, not an access grant |
| `owner` | Responsible author/maintainer |
| `language` | Content language, for example `en-GB` |
| `created_at`, `updated_at` | UTC calendar dates `YYYY-MM-DD` |
| `entrypoint` | ID of an existing page |
| `pages` | Ordered declared pages; this is the reading order |
| `sources` | Named attribution records |
| `assets` | Declared files carried with the base |

A page has `id`, `title`, base-relative `path`, `kind`, `status`, `sources` (source IDs)
and `tags` (strings). Its canonical location is `pages/<name>.md`. IDs are stable even
when titles change. Markdown is UTF-8 without executable frontmatter requirements.
Metadata is centralized in the manifest to avoid two competing inventories.

Allowed page kinds are `overview`, `concept`, `reference`, `procedure`, `decision`,
`status`. Allowed status values are `documented`, `implemented`, `observed`, `proposed`,
`pending`. A page with mixed component maturity states MUST state those distinctions
in its text and structured records rather than imply that its page status passes
all underlying capabilities. `documented` means an authored explanation exists.

Sources have `id`, `title`, `type`, `uri`, `checked_at`. Types are `workspace`,
`repository`, `official`, `conversation`, `observation`. `uri` is provenance; it may
be an HTTPS link, a `workspace://` record pointer or a conversation/observation
identifier. Tools MUST NOT fetch source URIs as a side effect of ordinary validation,
search or export. A source date establishes when that source was checked, not when
all described behavior was tested. Credentials MUST NOT appear in source URIs.

Assets have `id`, `path`, `media_type`. JSON datasets use `data/<name>.json` with
`application/json`; other declared opaque assets use `assets/<name>` with a syntactically
valid media type. The tool copies their bytes and never executes or auto-fetches them. A base
MUST declare every content/asset file except the manifest itself. An implementation
may temporarily write a transaction sentinel such as `.tars-kb-pending`; it MUST
recover or reject that pending state before validating/exporting a clean partition.
The sentinel is not authored knowledge. No undeclared
attachment may be silently excluded from a portable export. A personal original may instead
be represented by a service/file/version reference appropriate to the audience.

## References and file boundaries

Stable references use `kb://<base-id>/<page-id>`. Local Markdown file links are
permitted inside the same base. Explicit cross-base links use the stable URI instead
of relative sibling-directory traversal. Local references MUST resolve to declared
content; same-base Markdown fragments are checked by the reference implementation. Cross-base
fragments are retained without reading sibling page text and are not fully validated.

A tool MUST reject absolute content paths, `..` components, symlinked content,
unknown IDs, duplicate IDs and canonical paths escaping the selected base. It MUST
reject a catalog path that escapes the library. It MUST validate each actual file
boundary rather than trust a string prefix. Shared library-root symlinks used as a
workspace entry may resolve to a trusted root before partition checks; symlinks
inside a content partition are prohibited.

Page text, code fences and imported material are untrusted content. Reading them
MUST NOT execute shell, evaluate embedded code, follow tool instructions, enable a
connection or change policy. A code example is not an instruction to run it. HTML
exports MUST escape raw HTML and disallow executable link schemes.

## Structured datasets

Each dataset MUST identify its `schema_version` and purpose. It SHOULD use stable
record IDs, explicit status labels, provenance and observation dates where relevant.
Directory records distinguish logical roles, actual templates and installation state.
An observed machine snapshot MUST identify its environment and timestamp without
secret values. Unknown or unavailable health is not zero/healthy.

New domain-specific datasets can be introduced as declared assets with their own
schemas. A generic v1 reader checks JSON syntax/declaration and content boundaries; it does
not pretend to validate an unknown domain schema. Consumers must declare supported
dataset versions and refuse unsupported meanings rather than silently reinterpret.

## Creation and changes

Creation MUST produce a new independent base, a minimal entry page and a catalog
registration. It MUST refuse to overwrite an existing partition. Registration MUST
preserve unrelated bases. Mutating tools SHOULD serialize catalog updates, write
atomically and leave failures visible; they MUST NOT present a partially created
base as successfully registered.

Authors edit the canonical pages/manifest/assets and validate before publishing.
Stable IDs should survive title/path edits; changed references must be updated.
`updated_at` and content version should advance for meaningful released changes.
The tool does not auto-increment or invent author decisions. Concurrent content edits
need ordinary Git/worktree or service concurrency control; a directory name alone
cannot coordinate multiple writers.

## Tool responses and exports

CLI JSON results MUST remain JSON on stdout; diagnostics go to stderr with a nonzero
exit code for failure. Human output may be concise prose. A caller binds the selected
base ID and revision to each operation. No tool may silently widen that selection.

Exports MUST carry only the selected partition's manifest, reading-order content,
declared assets, provenance/status labels and checksums. A combined reading edition
is derived; edits to it do not replace the canonical pages automatically. Export
MUST refuse an existing output directory in v1 and reject output inside the library.
A cross-base link is retained as a reference instead of copying unrelated content.
Checksums establish artifact integrity, not factual correctness or security approval.

The standard profiles are Markdown, HTML and JSON. A PDF renderer may consume the
HTML/Markdown export as an optional build step without becoming a required dependency
of the reference CLI. It must not fetch private source URIs or silently include other
bases. Exporting is not publishing, changing visibility or granting access.

## Future services and indexes

A viewer/search/MCP/API implementation uses this contract as its storage input.
Its authentication, authorization, write concurrency, operation audit, indexing,
revision checks and retention MUST be separately implemented. Index each base with
its ID and revision; filter authorization before retrieval/ranking and before output.
Do not embed all bases into a single unrestricted context merely for convenience.
Rebuildable indexes belong outside canonical authored content. Deletion/update must
invalidate affected index/cache rows; remote model disclosure remains separately
controlled. No automatic model jobs, remote calls or schedules are created by this contract.
