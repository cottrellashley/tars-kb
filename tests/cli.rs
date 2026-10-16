use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_tars-kb")
}
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .arg("--root")
        .arg(root)
        .args(args)
        .arg("--json")
        .output()
        .unwrap()
}
fn good(root: &Path, args: &[&str]) -> Value {
    let o = run(root, args);
    assert!(
        o.status.success(),
        "{:?}: {} {}",
        args,
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).unwrap()
}
fn setup() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let r = t.path().join("library");
    good(&r, &["init", "demo", "--title", "Demonstration"]);
    (t, r)
}
#[test]
fn readers_share_a_stable_snapshot_without_exclusive_open() {
    let (_t, r) = setup();
    let a = tars_kb::library::Base::open(&r, "demo").unwrap();
    let b = tars_kb::library::Base::open(&r, "demo").unwrap();
    assert_eq!(a.revision().unwrap(), b.revision().unwrap());
}
#[test]
fn resolved_times_are_valid_memory_points() {
    let (_t, r) = setup();
    for args in [
        vec!["time", "resolve", "2026-10-16"],
        vec![
            "time",
            "resolve",
            "2026-10-16T12:30:00",
            "--zone",
            "Europe/London",
        ],
        vec!["time", "normalize", "2026-10-16T12:30:00+01:00"],
    ] {
        tars_kb::temporal::point(&good(&r, &args)).unwrap();
    }
    for invalid in [
        "2026-10-16 12:30:00+02:00",
        "2026-10-16T12:30+02:00",
        "2026-10-16T12:30:00[Europe/Paris]",
    ] {
        assert!(
            !run(&r, &["time", "resolve", invalid, "--zone", "Europe/London"])
                .status
                .success()
        );
    }
}
#[test]
fn dry_run_respects_retry_identity_without_mutating_source() {
    let (t, r) = setup();
    good(&r, &["memory", "init", "demo"]);
    let input = t.path().join("batch.json");
    let ops = json!([{"op":"concept-init","id":"topic","input":{"title":"Topic","summary":"A synthetic topic"}}]);
    fs::write(&input, serde_json::to_vec(&ops).unwrap()).unwrap();
    let args = [
        "batch",
        "demo",
        "--input",
        input.to_str().unwrap(),
        "--idempotency-key",
        "retry",
    ];
    good(&r, &args);
    let profile = r.join("bases/demo/data/memory/profile.json");
    let before = fs::read(&profile).unwrap();
    let mut dry = args.to_vec();
    dry.push("--dry-run");
    let result = good(&r, &dry);
    assert_eq!(result["committed"], false);
    assert_eq!(fs::read(&profile).unwrap(), before);
    let mut changed = ops;
    changed[0]["id"] = "different".into();
    fs::write(&input, serde_json::to_vec(&changed).unwrap()).unwrap();
    let result = run(&r, &dry);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("idempotency_conflict"));
    assert_eq!(fs::read(&profile).unwrap(), before);
}
#[test]
fn native_legacy_interface_and_lazy_page_read() {
    let (_t, r) = setup();
    let list = good(&r, &["list"]);
    assert_eq!(list[0]["id"], "demo");
    let show = good(&r, &["show", "demo"]);
    assert_eq!(show.as_object().unwrap().len(), 3);
    let p = r.join("bases/demo/knowledge-base.json");
    let mut m: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    m["assets"].as_array_mut().unwrap().push(
        json!({"id":"unrelated","path":"data/unrelated.json","media_type":"application/json"}),
    );
    fs::write(p, serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(run(&r, &["show", "demo"]).status.success());
    assert!(!run(&r, &["validate", "demo"]).status.success());
}
#[test]
fn page_writes_compare_revision_and_preserve_unknown_metadata() {
    let (t, r) = setup();
    let p = r.join("bases/demo/knowledge-base.json");
    let mut m: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    m["future_optional"] = json!({"keep":"yes"});
    fs::write(&p, serde_json::to_vec(&m).unwrap()).unwrap();
    let before = good(&r, &["read", "demo"]);
    let input = t.path().join("draft.md");
    fs::write(&input, "# Updated\n\nStraße\n").unwrap();
    let args = [
        "page",
        "replace",
        "demo",
        "overview",
        "--expect-revision",
        before["revision"].as_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
    ];
    good(&r, &args);
    assert!(!run(&r, &args).status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(p).unwrap()).unwrap()["future_optional"]["keep"],
        "yes"
    );
    let hits = good(&r, &["search", "demo", "STRASSE"]);
    assert_eq!(hits.as_array().unwrap().len(), 1);
}
#[test]
fn bounded_output_and_cursor_staleness() {
    let (t, r) = setup();
    let draft = t.path().join("b.md");
    fs::write(&draft, "# Second\n").unwrap();
    good(
        &r,
        &[
            "page",
            "create",
            "demo",
            "second",
            "--title",
            "Second",
            "--input",
            draft.to_str().unwrap(),
        ],
    );
    let v = good(&r, &["outline", "demo", "overview", "--limit", "1"]);
    assert_eq!(v["output_version"], "1.0");
    let big = t.path().join("big.md");
    fs::write(&big, "x".repeat(20_000)).unwrap();
    good(
        &r,
        &[
            "page",
            "create",
            "demo",
            "big",
            "--title",
            "Big",
            "--input",
            big.to_str().unwrap(),
        ],
    );
    let o = run(&r, &["read", "demo", "big", "--max-bytes", "512"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("output_limit"));
}
#[test]
fn export_stays_partitioned_and_rejects_overwrite() {
    let (t, r) = setup();
    good(&r, &["init", "other", "--title", "Other"]);
    let out = t.path().join("export");
    good(
        &r,
        &[
            "export",
            "demo",
            "--output",
            out.to_str().unwrap(),
            "--format",
            "html",
        ],
    );
    assert!(out.join("knowledge-base.json").is_file());
    assert!(!out.join("other").exists());
    assert!(
        !run(&r, &["export", "demo", "--output", out.to_str().unwrap()])
            .status
            .success()
    );
    assert!(
        !run(
            &r,
            &[
                "export",
                "demo",
                "--output",
                r.join("inside").to_str().unwrap()
            ]
        )
        .status
        .success()
    );
}
#[test]
fn unknown_version_and_symlink_fail_without_content_disclosure() {
    let (t, r) = setup();
    let secret = t.path().join("secret");
    fs::write(&secret, "fixture-private-marker").unwrap();
    let page = r.join("bases/demo/pages/overview.md");
    fs::remove_file(&page).unwrap();
    std::os::unix::fs::symlink(&secret, &page).unwrap();
    let o = run(&r, &["show", "demo"]);
    assert!(!o.status.success());
    assert!(!String::from_utf8_lossy(&o.stdout).contains("fixture-private-marker"));
    fs::remove_file(page).unwrap();
    let p = r.join("bases/demo/knowledge-base.json");
    let mut m: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    m["schema_version"] = "9.0".into();
    fs::write(p, serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(!run(&r, &["show", "demo"]).status.success());
}
#[test]
fn relative_dates_require_both_clock_and_zone() {
    let (_t, r) = setup();
    assert!(!run(&r, &["time", "resolve", "yesterday"]).status.success());
    let v = good(
        &r,
        &[
            "time",
            "resolve",
            "yesterday",
            "--reference",
            "2026-03-30T00:30:00Z",
            "--zone",
            "Europe/London",
        ],
    );
    assert_eq!(v["value"], "2026-03-29");
    assert!(
        !run(
            &r,
            &[
                "time",
                "resolve",
                "2026-03-29T01:30:00",
                "--zone",
                "Europe/London"
            ]
        )
        .status
        .success()
    );
    assert!(
        !run(
            &r,
            &[
                "time",
                "resolve",
                "2026-10-25T01:30:00",
                "--zone",
                "Europe/London"
            ]
        )
        .status
        .success()
    );
}
#[test]
fn incomplete_portable_base_refuses_missing_journal() {
    let (_t, r) = setup();
    fs::write(r.join("bases/demo/.tars-kb-pending"), "missing").unwrap();
    let o = run(&r, &["show", "demo"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("recovery_required"));
}
#[test]
fn metadata_listing_does_not_open_page_content() {
    let (_t, r) = setup();
    fs::remove_file(r.join("bases/demo/pages/overview.md")).unwrap();
    assert!(run(&r, &["list"]).status.success());
}
#[cfg(feature = "fault-injection")]
#[test]
fn subprocess_crash_recovers_before_or_after_commit() {
    for point in ["0", "1", "commit"] {
        let (t, r) = setup();
        let before = good(&r, &["read", "demo"]);
        let draft = t.path().join("new.md");
        fs::write(&draft, "# After crash\n").unwrap();
        let out = Command::new(bin())
            .arg("--root")
            .arg(&r)
            .args([
                "page",
                "replace",
                "demo",
                "overview",
                "--expect-revision",
                before["revision"].as_str().unwrap(),
                "--input",
                draft.to_str().unwrap(),
            ])
            .env("TARS_KB_CRASH_AFTER", point)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(86));
        let after = good(&r, &["read", "demo"]);
        assert_eq!(
            after["content"].as_str().unwrap().contains("After crash"),
            point == "commit"
        );
        good(&r, &["validate", "demo"]);
    }
}
