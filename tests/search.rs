//! Synthetic only: search must never need Ashley's knowledge or account data.
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use tars_kb::{
    library::{self, Base},
    memory,
    search::{self, SearchOptions},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    cache: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let cache = temp.path().join("cache");
        library::initialize(&root, "alpha", "Synthetic alpha", "Fixture").unwrap();
        Self {
            _temp: temp,
            root,
            cache,
        }
    }

    fn open(&self, id: &str) -> Base {
        Base::open(&self.root, id).unwrap()
    }
    fn page(&self, id: &str, text: &str) {
        fs::write(
            self.root.join(format!("bases/{id}/pages/overview.md")),
            text,
        )
        .unwrap();
    }
    fn ranked(&self) -> SearchOptions {
        SearchOptions {
            ranked: true,
            cache_root: Some(self.cache.clone()),
            ..Default::default()
        }
    }
    fn index_path(&self) -> PathBuf {
        let directories: Vec<_> = fs::read_dir(&self.cache)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        directories[0].join("alpha/search-v1.sqlite3")
    }
    fn memory(&self) -> Base {
        let mut base = self.open("alpha");
        library::source_add(&mut base,json!({"id":"fixture","title":"Synthetic observation","type":"observation","uri":"fixture://synthetic","checked_at":"2026-10-16"})).unwrap();
        memory::memory_init(&mut base).unwrap();
        memory::concept_init(
            &mut base,
            "routing",
            json!({"title":"Service routing","summary":"Synthetic routing facts","aliases":["VM proxy"]}),
        )
        .unwrap();
        memory::remember(&mut base,json!({"id":"route-observed","concepts":["routing"],"text":"The synthetic service was listening on port 9876.","basis":"observed","temporal":{"kind":"observed","at":{"type":"instant","value":"2026-10-16T12:00:00Z"}},"scope":{"environment":"synthetic"},"sources":["fixture"]}),None).unwrap();
        base
    }
}

#[test]
fn legacy_literal_preserves_full_casefold_and_original_hit_shape() {
    let fixture = Fixture::new();
    fixture.page(
        "alpha",
        "# Alpha\n\nStraße fixture\nSTRASSE second fixture\n",
    );
    let result = search::literal(&fixture.open("alpha"), "strasse").unwrap();
    assert_eq!(
        result,
        json!([
            {"knowledge_base":"alpha","page":"overview","path":"pages/overview.md","line":3,"text":"Straße fixture"},
            {"knowledge_base":"alpha","page":"overview","path":"pages/overview.md","line":4,"text":"STRASSE second fixture"}
        ])
    );
    assert!(search::literal(&fixture.open("alpha"), " ").is_err());
}

#[test]
fn selected_base_and_library_indexes_are_separate() {
    let fixture = Fixture::new();
    library::initialize(&fixture.root, "beta", "Synthetic beta", "Fixture").unwrap();
    fixture.page("alpha", "# Alpha\n\nalpha-only-canary\n");
    fixture.page("beta", "# Beta\n\nprivate-beta-canary\n");
    let alpha = fixture.open("alpha");
    let beta = fixture.open("beta");
    search::index(&alpha, false, Some(&fixture.cache)).unwrap();
    search::index(&beta, false, Some(&fixture.cache)).unwrap();
    assert!(
        search::literal(&alpha, "private-beta")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let result = search::search(&alpha, "private-beta-canary", &fixture.ranked()).unwrap();
    assert!(result["hits"].as_array().unwrap().is_empty());
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("private-beta-canary\"}")
    );
    // A second root can reuse the same base ID without reusing alpha's index.
    let second = fixture._temp.path().join("second");
    library::initialize(&second, "alpha", "Other alpha", "Fixture").unwrap();
    let other = Base::open(&second, "alpha").unwrap();
    assert!(search::search(&other, "canary", &fixture.ranked()).is_err());
}

#[test]
fn ranked_requires_explicit_index_and_reports_snapshot_consistency() {
    let fixture = Fixture::new();
    fixture.page("alpha", "# Alpha\n\nsynthetic indexing example\n");
    let base = fixture.open("alpha");
    assert!(search::search(&base, "indexing", &fixture.ranked()).is_err());
    let first = search::index(&base, false, Some(&fixture.cache)).unwrap();
    assert_eq!(first["documents"], 1);
    assert_eq!(first["updated"], 1);
    let second = search::index(&base, false, Some(&fixture.cache)).unwrap();
    assert_eq!(second["updated"], 0);
    let result = search::search(&base, "indexing", &fixture.ranked()).unwrap();
    assert_eq!(result["consistency"], "indexed_snapshot_unverified");
    assert_eq!(result["hits"][0]["reference"], "kb://alpha/overview");
    assert_eq!(result["hits"][0]["status"], "documented");
    assert!(!result["source_signature"].as_str().unwrap().is_empty());
}

#[test]
fn verified_search_reconciles_changed_content_without_claiming_unchecked_freshness() {
    let fixture = Fixture::new();
    fixture.page("alpha", "# Alpha\n\nformer-canary\n");
    let base = fixture.open("alpha");
    search::index(&base, false, Some(&fixture.cache)).unwrap();
    let before = search::search(&base, "former", &fixture.ranked()).unwrap();
    assert_eq!(before["hits"].as_array().unwrap().len(), 1);
    fixture.page("alpha", "# Alpha\n\nreplacement-canary\n");
    let stale = search::search(&base, "former", &fixture.ranked()).unwrap();
    assert_eq!(stale["hits"].as_array().unwrap().len(), 1);
    assert_eq!(stale["consistency"], "indexed_snapshot_unverified");
    let options = SearchOptions {
        verified: true,
        ..fixture.ranked()
    };
    let after = search::search(&base, "former", &options).unwrap();
    assert!(after["hits"].as_array().unwrap().is_empty());
    assert_ne!(after["source_signature"], before["source_signature"]);
    assert_eq!(after["consistency"], "verified_source_snapshot");
    assert_eq!(
        search::search(&base, "replacement", &options).unwrap()["hits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn undeclaring_a_page_removes_its_old_index_rows() {
    let fixture = Fixture::new();
    let path = fixture.root.join("bases/alpha/knowledge-base.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["pages"].as_array_mut().unwrap().push(json!({"id":"removable","title":"Removable","path":"pages/removable.md","kind":"reference","status":"documented","sources":[],"tags":[]}));
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    fs::write(
        fixture.root.join("bases/alpha/pages/removable.md"),
        "# Removable\n\nremoved-canary\n",
    )
    .unwrap();
    {
        let base = fixture.open("alpha");
        search::index(&base, false, Some(&fixture.cache)).unwrap();
        assert_eq!(
            search::search(&base, "removed", &fixture.ranked()).unwrap()["hits"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    manifest["pages"].as_array_mut().unwrap().pop();
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    fs::remove_file(fixture.root.join("bases/alpha/pages/removable.md")).unwrap();
    let base = fixture.open("alpha");
    let result = search::index(&base, false, Some(&fixture.cache)).unwrap();
    assert_eq!(result["removed"], 1);
    assert!(
        search::search(&base, "removed", &fixture.ranked()).unwrap()["hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn corrupt_derived_index_fails_cleanly_and_explicit_rebuild_recovers() {
    let fixture = Fixture::new();
    fixture.page("alpha", "# Alpha\n\ncorruption-test-canary\n");
    let base = fixture.open("alpha");
    let source_before = base.read("pages/overview.md").unwrap();
    search::index(&base, false, Some(&fixture.cache)).unwrap();
    fs::write(fixture.index_path(), b"synthetic broken SQLite bytes").unwrap();
    assert!(search::search(&base, "canary", &fixture.ranked()).is_err());
    assert!(search::index(&base, false, Some(&fixture.cache)).is_err());
    search::index(&base, true, Some(&fixture.cache)).unwrap();
    assert_eq!(
        search::search(&base, "canary", &fixture.ranked()).unwrap()["hits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(base.read("pages/overview.md").unwrap(), source_before);
}

#[test]
fn ranked_memory_hits_keep_scope_point_time_and_evidence_labels() {
    let fixture = Fixture::new();
    let base = fixture.memory();
    search::index(&base, false, Some(&fixture.cache)).unwrap();
    let options = SearchOptions {
        concept: Some("routing".into()),
        ..fixture.ranked()
    };
    let result = search::search(&base, "9876", &options).unwrap();
    assert_eq!(
        search::search(&base, "routing", &options).unwrap()["hits"][0]["id"],
        "route-observed"
    );
    assert_eq!(
        search::search(&base, "VM proxy", &options).unwrap()["hits"][0]["id"],
        "route-observed"
    );
    let hit = &result["hits"][0];
    assert_eq!(hit["kind"], "assertion");
    assert_eq!(hit["basis"], "observed");
    assert_eq!(hit["temporal"]["kind"], "observed");
    assert_eq!(hit["scope"]["environment"], "synthetic");
    assert_eq!(hit["sources"], json!(["fixture"]));
    assert!(hit["temporal_status"].is_string());
    assert!(hit["conflicts"].is_array());
    assert_eq!(
        hit["reference"],
        "kb-memory://alpha/assertion/route-observed"
    );
    assert!(
        search::literal(&base, "9876")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let options = SearchOptions {
        concept: Some("unrelated".into()),
        ..fixture.ranked()
    };
    assert!(
        search::search(&base, "9876", &options).unwrap()["hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn retracted_assertions_are_removed_on_verified_refresh() {
    let fixture = Fixture::new();
    let mut base = fixture.memory();
    search::index(&base, false, Some(&fixture.cache)).unwrap();
    let record = memory::fact(&base, "route-observed").unwrap();
    let revision = record["revision"].as_str().unwrap();
    memory::revise(
        &mut base,
        "route-observed",
        "retract",
        json!({"reason":"synthetic withdrawal"}),
        Some(revision),
        None,
    )
    .unwrap();
    assert!(
        search::search(&base, "9876", &fixture.ranked())
            .unwrap_err()
            .to_string()
            .contains("index_stale")
    );
    let options = SearchOptions {
        verified: true,
        ..fixture.ranked()
    };
    assert!(
        search::search(&base, "9876", &options).unwrap()["hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn bounded_context_never_splits_qualifiers_or_exceeds_json_budget() {
    let fixture = Fixture::new();
    fixture.page(
        "alpha",
        &(0..30)
            .map(|number| format!("fixture result {number}\n"))
            .collect::<String>(),
    );
    let base = fixture.open("alpha");
    let options = SearchOptions {
        limit: 2,
        max_bytes: 2400,
        ..Default::default()
    };
    let result = search::context(&base, "fixture", &options).unwrap();
    assert_eq!(result["hits"].as_array().unwrap().len(), 2);
    assert_eq!(result["truncated"], true);
    assert!(serde_json::to_vec(&result).unwrap().len() <= 2400);
    assert!(
        search::search(
            &base,
            "fixture",
            &SearchOptions {
                max_bytes: 8,
                ..options
            }
        )
        .is_err()
    );
}

#[test]
fn queries_are_bound_as_data_and_cache_cannot_live_in_a_base() {
    let fixture = Fixture::new();
    fixture.page("alpha", "# Alpha\n\nordinary fixture\n");
    let base = fixture.open("alpha");
    search::index(&base, false, Some(&fixture.cache)).unwrap();
    for query in [
        "fixture OR unknown",
        "\" OR \"",
        "fixture'; DROP TABLE docs; --",
    ] {
        let result = search::search(&base, query, &fixture.ranked());
        assert!(
            result.is_ok(),
            "ordinary query should remain data: {query}: {result:?}"
        );
    }
    assert_eq!(
        search::search(&base, "fixture", &fixture.ranked()).unwrap()["hits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(search::index(&base, false, Some(&base.root.join("derived"))).is_err());
    assert!(
        search::index(
            &base,
            false,
            Some(&fixture.root.join("../library/injected-cache"))
        )
        .is_err()
    );
}

#[test]
fn cache_and_index_permissions_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let base = fixture.open("alpha");
    search::index(&base, false, Some(&fixture.cache)).unwrap();
    let path = fixture.index_path();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn concurrent_index_updates_and_reads_publish_complete_snapshots() {
    use std::sync::{Arc, Barrier};
    let fixture = Fixture::new();
    fixture.page("alpha", "# Alpha\n\nconcurrency-canary\n");
    search::index(&fixture.open("alpha"), false, Some(&fixture.cache)).unwrap();
    let barrier = Arc::new(Barrier::new(4));
    let mut workers = Vec::new();
    for worker in 0..4 {
        let root = fixture.root.clone();
        let cache = fixture.cache.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            let base = Base::open(&root, "alpha").unwrap();
            let options = SearchOptions {
                ranked: true,
                cache_root: Some(cache.clone()),
                ..Default::default()
            };
            for _ in 0..3 {
                if worker % 2 == 0 {
                    search::index(&base, false, Some(&cache)).unwrap();
                }
                let result = search::search(&base, "concurrency", &options).unwrap();
                assert_eq!(result["hits"].as_array().unwrap().len(), 1);
                assert_eq!(result["hits"][0]["reference"], "kb://alpha/overview");
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn native_page_edit_invalidates_the_accepted_manifest_index_revision() {
    let fixture = Fixture::new();
    fixture.page("alpha", "# Alpha\n\nnative-before-canary\n");
    let mut base = fixture.open("alpha");
    search::index(&base, false, Some(&fixture.cache)).unwrap();
    let before = base.read("pages/overview.md").unwrap();
    base.commit(vec![tars_kb::storage::Write {
        path: "pages/overview.md".into(),
        expected: Some(tars_kb::storage::hash(&before)),
        bytes: Some(b"# Alpha\n\nnative-after-canary\n".to_vec()),
    }])
    .unwrap();
    assert!(
        search::search(&base, "native-before", &fixture.ranked())
            .unwrap_err()
            .to_string()
            .contains("index_stale")
    );
    let options = SearchOptions {
        verified: true,
        ..fixture.ranked()
    };
    assert!(
        search::search(&base, "native-before", &options).unwrap()["hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        search::search(&base, "native-after", &options).unwrap()["hits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn verified_search_rejects_tampered_concept_revision() {
    let fixture = Fixture::new();
    let base = fixture.memory();
    search::index(&base, false, Some(&fixture.cache)).unwrap();
    let path = base.root.join("data/memory/concepts/routing.json");
    let mut concept: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    concept["aliases"] = json!(["unaccepted alias"]);
    fs::write(path, serde_json::to_vec(&concept).unwrap()).unwrap();
    let options = SearchOptions {
        verified: true,
        ..fixture.ranked()
    };
    let error = search::search(&base, "unaccepted", &options).unwrap_err();
    assert!(error.to_string().contains("history_integrity"));
}
