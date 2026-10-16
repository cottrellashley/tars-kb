# Performance and measurement

The fast path is a short-lived local executable, bounded output and lazy reads of a
selected partition. Listing base metadata should not load page bodies; reading one
fact should not parse every page or every assertion. Full validation and export
deliberately traverse more content.

No model call, embedding service or background daemon is needed for ordinary use.
An optional local search index accelerates retrieval; it remains rebuildable from
canonical files. See [search](search.md) for the current retrieval implementation.

The repository's standard-library benchmark script creates isolated synthetic data:

```bash
cargo build --release --locked
python3 scripts/benchmark.py --binary target/release/tars-kb \
  --pages 100,1000 --assertions 1000,10000 --runs 15 \
  --index-runs 3 --write-runs 5 --with-writes \
  --output /tmp/tars-kb-benchmark.json
```

The script launches a new process for each observation; it does not evict the OS
cache or claim filesystem-cold results. Synthetic working data defaults to
`target/benchmark-work`, not potentially RAM-backed temporary storage. Check the
reported filesystem before interpreting durable-write results. Keep generated
corpora, databases and indexes outside the public source tree.

## Measured v0.1.0 results

The [complete October 16 report](benchmarks/linux-x86_64-2026-10-16.json) records
36 successful measurement groups after native full fixture validation. This run used
Linux x86_64, an Intel Core i7-7500U at 2.70 GHz and btrfs. Its four synthetic bases
contained 100/1,000 pages and 1,000/10,000 assertions, totaling 16,554,443 source bytes.

The tested executable came from [GitHub release v0.1.0](https://github.com/cottrellashley/tars-kb/releases/tag/v0.1.0),
built from [source ea67e357703b3cc769fbd4bd78ef83ab9671b917](https://github.com/cottrellashley/tars-kb/commit/ea67e357703b3cc769fbd4bd78ef83ab9671b917)
with `rustc 1.99.1 (58cf8f9c3 2026-10-13)`. The executable SHA-256 is
`7af808df449b82697110eb35a7c84ff07ba2ef6c664356e335d5dacc9995211e`.
This is the CI-built Linux executable, not a timing claim about every supported
platform or a locally rebuilt debug binary.

Each sample starts a new process and measures wall time through captured stdout and
stderr. OS caches were warm/uncontrolled. Ordinary operations have 15 repeated
samples; index operations have three, and durable creates have five. The raw report
also retains first-process timings and declared warmup counts. File/directory sync
operations remained enabled. No power-loss test or peak-memory measurement was run.

Selected measurements, in milliseconds:

| Operation | Dataset | Samples | Median | Observed maximum |
|---|---|---:|---:|---:|
| Process startup/help | No base opened | 15 | 10.728 | 12.039 |
| Catalog metadata list | All four bases | 15 | 224.224 | 385.651 |
| Read one page | 100 pages | 15 | 14.684 | 17.929 |
| Read one page | 1,000 pages | 15 | 40.859 | 42.267 |
| Ranked search | 1,000 assertions | 15 | 50.328 | 69.370 |
| Ranked search | 10,000 assertions | 15 | 286.905 | 542.656 |
| Read one fact | 1,000 assertions | 15 | 43.845 | 55.527 |
| Read one fact | 10,000 assertions | 15 | 308.387 | 498.694 |
| Read one concept | 10,000 assertions | 15 | 321.610 | 414.604 |
| Rebuild index | 10,000 assertions | 3 | 8,188.622 | 9,273.344 |
| Update unchanged index | 10,000 assertions | 3 | 6,727.855 | 7,522.817 |
| Create durable assertion | 1,000 assertions | 5 | 449.232 | 485.618 |
| Create durable assertion | 10,000 assertions | 5 | 3,325.971 | 3,648.347 |

These are exploratory observations. In particular, three index samples and five
write samples cannot establish reliable p95/p99 latency; their reported tails are
sample maxima. Fifteen ordinary samples are also a small observational sample. The
table therefore labels maxima explicitly rather than presenting them as service
levels. The full report includes common-word/negative queries, literal searches,
other dataset sizes and response-byte counts.

At 10,000 assertions this release does **not** demonstrate an ultra-fast memory
workflow on this machine: a single fact takes about 308 ms at the median and a
durable create about 3.33 seconds. Even an unchanged index update takes about
6.73 seconds. These costs remain visible rather than being omitted from read-only
startup or small-base results.

The v1 format keeps a manifest entry per declared assertion and an inventory/history
profile; native writes also serialize a recoverable transaction journal. Opening a
selected record still incurs size-dependent metadata work, and creating records
rewrites growing metadata. This run measures end-to-end behavior rather than
attributing time with a profiler. Reducing those costs needs a separately measured,
compatible optimization while preserving revision, temporal and durability behavior.

## Measuring subsequent changes

Do not infer benchmark results from the use of Rust. Measure release builds on
datasets representative of the intended library, including startup and JSON output.

Record at least:

- Executable revision, release profile, compiler, operating system, CPU, storage and
  dataset definition.
- Number of bases, pages, concepts, assertions/revisions, bytes and query mix.
- Wall-clock median and tail latency over a stated number of repetitions.
- Cold-index build time separately from warm indexed retrieval.
- Process-cold runs separately from filesystem-cache-cold runs. Restarting a process
  does not flush the filesystem cache.
- Output limits, returned count, response bytes and peak memory where measured.
- Write/revision latency separately from read/search latency and concurrent-writer
  contention.

A useful matrix measures metadata listing, one concept, one fact, bounded search,
repeated multi-query retrieval, a durable write and full validation at increasing
dataset sizes. Include a negative query and a common-word query. Recheck correctness
after retrieval or indexing changes; a fast stale result is not a valid improvement.

Benchmark targets and unexecuted CI configurations are not measured results. Publish
new results with the binary identity and methodology; do not replace the measured
limitations above with an unmeasured sub-millisecond or throughput promise.
