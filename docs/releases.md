# Building and release artifacts

Build from source using the checked-in dependency lockfile:

```bash
cargo install --path . --locked
```

The crate declares its minimum supported Rust version in `Cargo.toml`. Native SQLite
is bundled by the Rust dependency, so an independently installed SQLite executable
is not needed for search. Building still requires the normal Rust/native compiler
toolchain.

CI runs formatting, warnings-as-errors Clippy, tests, interrupted-write tests using
the separate fault-injection feature, a plain release build, synthetic example
validation and the public-content scanner. Release binaries do not enable the
fault-injection feature. Native Linux x86_64 and aarch64
jobs avoid treating cross-compilation as execution evidence. A macOS compatibility
job is configured; verify its actual result for the revision you intend to use.

The Linux release workflow runs on version tags and builds:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

It packages the executable, license, documentation, specifications and synthetic
examples, then publishes the archives and `SHA256SUMS` only after their jobs pass.
The archives use the Ubuntu build environment's GNU libc; they are not promised as
musl/static binaries or compatible with every older Linux distribution. No Windows
binary, macOS notarization or package-registry publication is implied.

Download the archive matching the OS/architecture and its checksum file from the
same [GitHub release](https://github.com/cottrellashley/tars-kb/releases). From that
download directory:

```bash
sha256sum --check --ignore-missing SHA256SUMS
```

Inspect the verified archive and install `tars-kb` into an owner-controlled directory
on `PATH`. A checksum detects accidental or mismatched bytes; it does not replace
trust in the source/release account. The workflow does not claim signed provenance
or reproducible binaries.

Before cutting a release, maintainers verify the package version matches the tag,
run the public scanner over the complete candidate tree, review the staged diff
and history, and check that examples contain synthetic material only. Checksums,
content schema versions, memory-profile versions and output-protocol versions are
distinct from the executable's package version.

## Updating the tool or a contract

Install a chosen release, then inspect `capabilities --json`, `spec list` and
`schema list` before using it against authored data. The current binary reads and
writes storage major 1 and the optional `tars-memory/1.0` profile. Unknown required
features and unsupported profiles fail before a write; upgrading an executable
does not implicitly migrate a library. Keep a clean, verified export and independent
backup before any future contract migration. This release has no migration command
or automatic updater. A future incompatible profile needs an explicit documented
migration and a CLI release that advertises support.
