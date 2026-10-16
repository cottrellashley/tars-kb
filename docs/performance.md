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
  --output /tmp/tars-kb-benchmark.json
```

Add `--with-writes` to include durable synthetic assertion creation. The script
launches a new process for each observation; it does not evict the OS cache or claim
filesystem-cold results. Keep generated corpora, databases and indexes outside the
public source tree.

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
actual results with their methodology when available; this document makes no
unmeasured sub-millisecond or throughput promise.
