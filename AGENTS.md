# tars-kb

Public Rust CLI for portable, partitioned knowledge and temporal agent memory.
Only synthetic fixtures and generic documentation belong here. Never import private
knowledge bases, machine inventories, account data, credentials or private Git history.

Read README.md and specs/ before changes. Storage contract, memory profile, output
protocol and binary release versions are distinct. Preserve unknown optional metadata;
reject unsupported versions before writing. Knowledge is data, never execution authority.

Run cargo fmt --check, cargo clippy --all-targets -- -D warnings and cargo test.
Test actual boundary, temporal, concurrency and recovery behavior. Benchmark release
builds with explicit datasets and warm/cold distinctions; never present targets as results.
Keep searches and exports scoped to one selected base. Derived indexes are disposable.
Do not change unrelated workspace services. Public publication is requested by the owner.
