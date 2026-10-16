use serde_json::json;
use std::fs;
use tars_kb::{
    library::{self, Base},
    pages, storage,
};

fn fixture() -> (tempfile::TempDir, Base) {
    let temporary = tempfile::tempdir().unwrap();
    let library = temporary.path().join("library");
    library::initialize(&library, "sample", "Synthetic pages", "Example author").unwrap();
    let base = Base::open(&library, "sample").unwrap();
    (temporary, base)
}

#[test]
fn candidate_links_validate_before_create_and_support_new_self_headings() {
    let (_temporary, mut base) = fixture();
    let before = base.read("knowledge-base.json").unwrap();
    assert!(
        pages::create(
            &mut base,
            "broken",
            "Broken",
            "# Broken\n\n[Missing](missing.md)\n"
        )
        .is_err()
    );
    assert_eq!(base.read("knowledge-base.json").unwrap(), before);
    assert!(!base.root.join("pages/broken.md").exists());

    pages::create(
        &mut base,
        "chapter",
        "Chapter",
        "# Chapter\n\n[Self](#chapter) [Stable](kb://sample/chapter#chapter)\n",
    )
    .unwrap();
    assert!(pages::validate(&base).is_ok());
    assert!(
        pages::create(
            &mut base,
            "unsafe",
            "Unsafe",
            "# Unsafe\n\n[No](javascript:alert(1))\n"
        )
        .is_err()
    );
    assert!(!base.root.join("pages/unsafe.md").exists());
}

#[test]
fn replacement_and_patch_preserve_incoming_heading_links_or_leave_no_change() {
    let (_temporary, mut base) = fixture();
    let chapter = "# Chapter\n\n## Details\n\nOriginal sentence.\n";
    pages::create(&mut base, "chapter", "Chapter", chapter).unwrap();
    let overview = pages::contents(&base, "overview").unwrap();
    pages::replace(
        &mut base,
        "overview",
        "# Overview\n\n[Read details](chapter.md#details)\n",
        &storage::hash(overview.as_bytes()),
    )
    .unwrap();
    let manifest = base.read("knowledge-base.json").unwrap();
    let revision = storage::hash(chapter.as_bytes());
    assert!(
        pages::replace(
            &mut base,
            "chapter",
            "# Chapter\n\nNo details heading.\n",
            &revision
        )
        .is_err()
    );
    assert_eq!(pages::contents(&base, "chapter").unwrap(), chapter);
    assert_eq!(base.read("knowledge-base.json").unwrap(), manifest);
    assert!(
        pages::patch(
            &mut base,
            "chapter",
            &json!({"find":"## Details","replace":"## Renamed"}),
            &revision
        )
        .is_err()
    );
    assert_eq!(pages::contents(&base, "chapter").unwrap(), chapter);

    pages::patch(
        &mut base,
        "chapter",
        &json!({"find":"Original sentence.","replace":"Corrected sentence."}),
        &revision,
    )
    .unwrap();
    assert!(pages::validate(&base).is_ok());
}

#[test]
fn combined_exports_scope_headings_rewrite_links_and_keep_canonical_bytes() {
    use pulldown_cmark::{Event, Options, Parser, Tag};

    let (temporary, mut base) = fixture();
    let initial = pages::contents(&base, "overview").unwrap();
    pages::replace(
        &mut base,
        "overview",
        "# Shared\n",
        &storage::hash(initial.as_bytes()),
    )
    .unwrap();
    let chapter = "# Shared\n\n## Shared\n\n[Back](overview.md#shared) [Stable](kb://sample/overview#shared) [Data](../data/fixture.json)\n\n[External][topic]\n\n[topic]: https://example.invalid/chapter\n\n![Synthetic image](https://example.invalid/image.png)\n\n<script>unsafe()</script>\n";
    pages::create(
        &mut base,
        "chapter",
        "Chapter",
        chapter
            .replace(" [Data](../data/fixture.json)", "")
            .as_str(),
    )
    .unwrap();
    let mut manifest = base.manifest.clone();
    manifest["assets"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"fixture","path":"data/fixture.json","media_type":"application/json"}));
    let writes = vec![
        base.write(
            "data/fixture.json",
            &json!({"schema_version":"1.0","purpose":"Synthetic export fixture"}),
        )
        .unwrap(),
        base.write("knowledge-base.json", &manifest).unwrap(),
    ];
    base.commit(writes).unwrap();
    let overview = "# Shared\n\n[Chapter](chapter.md#shared) [Self](#shared) [Duplicate][topic]\n\n[topic]: chapter.md#shared-1\n\n`[Not a link](missing.md)`\n";
    let previous = pages::contents(&base, "overview").unwrap();
    pages::replace(
        &mut base,
        "overview",
        overview,
        &storage::hash(previous.as_bytes()),
    )
    .unwrap();
    let previous = pages::contents(&base, "chapter").unwrap();
    pages::replace(
        &mut base,
        "chapter",
        chapter,
        &storage::hash(previous.as_bytes()),
    )
    .unwrap();

    let html_root = temporary.path().join("html");
    pages::export(&base, &html_root, "html").unwrap();
    let html = fs::read_to_string(html_root.join("reading.html")).unwrap();
    for anchor in [
        "page-overview",
        "page-chapter",
        "page-overview__shared",
        "page-chapter__shared",
        "page-chapter__shared-1",
    ] {
        assert_eq!(
            html.matches(&format!("id=\"{anchor}\"")).count(),
            1,
            "{anchor}"
        );
    }
    for destination in [
        "#page-chapter__shared",
        "#page-chapter__shared-1",
        "#page-overview__shared",
        "data/fixture.json",
        "https://example.invalid/chapter",
    ] {
        assert!(
            html.contains(&format!("href=\"{destination}\"")),
            "{destination}"
        );
    }
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img"));
    assert_eq!(
        fs::read_to_string(html_root.join("pages/chapter.md")).unwrap(),
        chapter
    );

    let markdown_root = temporary.path().join("markdown");
    pages::export(&base, &markdown_root, "markdown").unwrap();
    let markdown = fs::read_to_string(markdown_root.join("reading.md")).unwrap();
    assert!(markdown.contains("<a id=\"page-chapter__shared-1\"></a>"));
    assert!(markdown.contains("`[Not a link](missing.md)`"));
    let links: Vec<_> = Parser::new_ext(&markdown, Options::all())
        .filter_map(|event| match event {
            Event::Start(Tag::Link { dest_url, .. }) => Some(dest_url.to_string()),
            _ => None,
        })
        .collect();
    assert!(links.contains(&"#page-chapter__shared-1".to_owned()));
    assert!(links.contains(&"data/fixture.json".to_owned()));
    assert!(
        !links
            .iter()
            .any(|link| link.starts_with("chapter.md") || link.starts_with("overview.md"))
    );
    assert_eq!(
        fs::read_to_string(markdown_root.join("pages/overview.md")).unwrap(),
        overview
    );
}
