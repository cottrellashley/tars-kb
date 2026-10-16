use crate::{
    Result,
    library::{self, Base},
    storage::{self, Write},
};
use anyhow::{Context, ensure};
use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    ops::Range,
    path::Path,
};

pub fn page<'a>(base: &'a Base, id: &str) -> Result<&'a Value> {
    library::identifier(id)?;
    base.manifest["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .context("not_found: unknown page")
}
pub fn contents(base: &Base, id: &str) -> Result<String> {
    String::from_utf8(base.read(library::text(page(base, id)?, "path")?)?)
        .context("invalid_utf8: Markdown page")
}
pub fn slug(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-' || *c == '_')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}
pub fn outline_text(body: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut heading: Option<(u32, usize, String)> = None;
    let mut seen = HashMap::<String, usize>::new();
    for (e, r) in Parser::new_ext(body, Options::all()).into_offset_iter() {
        match e {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some((level as u32, r.start, String::new()))
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, _, s)) = &mut heading {
                    s.push_str(&t)
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, start, title)) = heading.take() {
                    let raw = slug(&title);
                    let n = seen.entry(raw.clone()).or_default();
                    let id = if *n == 0 { raw } else { format!("{raw}-{n}") };
                    *n += 1;
                    out.push(json!({"id":id,"title":title,"level":level,"byte_start":start,"line":body[..start].bytes().filter(|b|*b==b'\n').count()+1}));
                }
            }
            _ => {}
        }
    }
    out
}
pub fn show(base: &Base, id: Option<&str>, section: Option<&str>, revision: bool) -> Result<Value> {
    let id = id.unwrap_or(base.manifest["entrypoint"].as_str().unwrap());
    let original = contents(base, id)?;
    let mut body = original.clone();
    if let Some(section) = section {
        let headings = outline_text(&body);
        let n = headings
            .iter()
            .position(|h| h["id"] == section)
            .context("not_found: unknown section")?;
        let h = &headings[n];
        let start = h["byte_start"].as_u64().unwrap() as usize;
        let level = h["level"].as_u64().unwrap();
        let end = headings[n + 1..]
            .iter()
            .find(|p| p["level"].as_u64().unwrap() <= level)
            .map_or(body.len(), |p| p["byte_start"].as_u64().unwrap() as usize);
        body = body[start..end].into();
    }
    let mut result = json!({"knowledge_base":base.id,"metadata":page(base,id)?,"content":body});
    if revision {
        result["revision"] = storage::hash(original.as_bytes()).into();
    }
    Ok(result)
}
pub fn outline(base: &Base, id: &str) -> Result<Value> {
    let body = contents(base, id)?;
    Ok(
        json!({"base_id":base.id,"page":id,"revision":storage::hash(body.as_bytes()),"sections":outline_text(&body)}),
    )
}

fn declared_in(manifest: &Value, path: &str) -> bool {
    ["pages", "assets"].iter().any(|key| {
        manifest[*key]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["path"] == path))
    })
}

fn local_target_in(
    base: &Base,
    manifest: &Value,
    from: &str,
    href: &str,
) -> Result<Option<(String, Option<String>)>> {
    if href.starts_with("kb://") {
        let u = url::Url::parse(href)?;
        ensure!(
            u.username().is_empty() && u.password().is_none() && u.query().is_none(),
            "invalid_link: KB URI"
        );
        let id = u.host_str().context("invalid_link: missing base")?;
        library::identifier(id)?;
        let pid = u.path().trim_start_matches('/');
        library::identifier(pid)?;
        if id == base.id {
            let p = manifest["pages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == pid)
                .context("invalid_link: unknown target page")?;
            return Ok(Some((
                library::text(p, "path")?.into(),
                u.fragment().map(str::to_string),
            )));
        }
        let (_, catalog) = library::catalog(&base.library)?;
        ensure!(
            catalog["knowledge_bases"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["id"] == id),
            "invalid_link: unknown target base"
        );
        // Cross-base verification reads metadata only. It never opens target pages.
        let p = storage::confined(&base.library, &format!("bases/{id}/knowledge-base.json"))?;
        let m = storage::read_json(&p)?;
        library::validate_manifest(&m, id)?;
        ensure!(
            m["pages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["id"] == pid),
            "invalid_link: unknown target page"
        );
        return Ok(None);
    }
    if let Ok(u) = url::Url::parse(href) {
        ensure!(
            ["https", "http", "mailto"].contains(&u.scheme()),
            "invalid_link: executable or unsupported URI scheme"
        );
        return Ok(None);
    }
    ensure!(
        !href.starts_with('/') && !href.starts_with('\\'),
        "invalid_link: absolute path"
    );
    let fake = url::Url::parse(&format!("https://kb.invalid/{from}"))?;
    let target = fake.join(href)?;
    ensure!(
        target.host_str() == Some("kb.invalid") && target.query().is_none(),
        "invalid_link: unsupported relative target"
    );
    let decoded = percent_decode(target.path().trim_start_matches('/'))?;
    ensure!(
        declared_in(manifest, &decoded),
        "invalid_link: undeclared target {decoded}"
    );
    Ok(Some((decoded, target.fragment().map(str::to_string))))
}
fn percent_decode(s: &str) -> Result<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            ensure!(i + 2 < bytes.len(), "invalid_link: percent encoding");
            let h = std::str::from_utf8(&bytes[i + 1..i + 3])?;
            out.push(u8::from_str_radix(h, 16)?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(String::from_utf8(out)?)
}
pub fn validate_links(base: &Base) -> Result<()> {
    validate_candidate_links(base, &base.manifest, &HashMap::new())
}

/// Read the proposed page body in preference to its live bytes. All incoming links
/// are checked too, so removing a heading cannot silently break a sibling page.
fn validate_candidate_links(
    base: &Base,
    manifest: &Value,
    changed: &HashMap<String, String>,
) -> Result<()> {
    library::validate_manifest(manifest, &base.id)?;
    let mut anchors = HashMap::<String, HashSet<String>>::new();
    for p in manifest["pages"].as_array().unwrap() {
        let path = library::text(p, "path")?;
        let body = if let Some(body) = changed.get(path) {
            body.clone()
        } else {
            String::from_utf8(base.read(path)?)?
        };
        for event in Parser::new_ext(&body, Options::all()) {
            if let Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) = event
                && let Some((target, fragment)) = local_target_in(base, manifest, path, &dest_url)?
            {
                ensure!(
                    changed.contains_key(&target)
                        || storage::confined(&base.root, &target)?.is_file(),
                    "invalid_link: missing target {target}"
                );
                if let Some(fragment) = fragment
                    && target.starts_with("pages/")
                {
                    if !anchors.contains_key(&target) {
                        let text = if let Some(body) = changed.get(&target) {
                            body.clone()
                        } else {
                            String::from_utf8(base.read(&target)?)?
                        };
                        anchors.insert(
                            target.clone(),
                            outline_text(&text)
                                .into_iter()
                                .map(|h| h["id"].as_str().unwrap().to_owned())
                                .collect(),
                        );
                    }
                    ensure!(
                        anchors[&target].contains(&percent_decode(&fragment)?),
                        "invalid_link: unknown heading {fragment}"
                    );
                }
            }
        }
    }
    Ok(())
}
pub fn validate(base: &Base) -> Result<Value> {
    let mut declared = HashSet::from(["knowledge-base.json".to_string()]);
    for p in base.manifest["pages"].as_array().unwrap() {
        let path = library::text(p, "path")?;
        String::from_utf8(base.read(path)?)?;
        declared.insert(path.into());
    }
    for a in base.manifest["assets"].as_array().unwrap() {
        let path = library::text(a, "path")?;
        let bytes = base.read(path)?;
        if library::text(a, "media_type")?.eq_ignore_ascii_case("application/json") {
            storage::parse_json(&bytes)?;
        }
        declared.insert(path.into());
    }
    for path in library::files_recursive(&base.root)? {
        ensure!(declared.contains(&path), "undeclared_file: {path}");
    }
    validate_links(base)?;
    Ok(base.summary())
}
pub fn create(base: &mut Base, id: &str, title: &str, body: &str) -> Result<Value> {
    library::identifier(id)?;
    ensure!(!title.trim().is_empty(), "invalid_input: title required");
    ensure!(page(base, id).is_err(), "already_exists: page ID");
    let path = format!("pages/{id}.md");
    let mut manifest = base.manifest.clone();
    manifest["pages"].as_array_mut().unwrap().push(json!({"id":id,"title":title,"path":path,"kind":"reference","status":"documented","sources":[],"tags":[]}));
    manifest["updated_at"] = library::today().into();
    validate_candidate_links(
        base,
        &manifest,
        &HashMap::from([(path.clone(), body.to_owned())]),
    )?;
    let mw = base.write("knowledge-base.json", &manifest)?;
    let receipt = base.commit(vec![
        Write {
            path,
            expected: None,
            bytes: Some(body.as_bytes().to_vec()),
        },
        mw,
    ])?;
    Ok(json!({"id":id,"revision":storage::hash(body.as_bytes()),"receipt":receipt}))
}
pub fn replace(base: &mut Base, id: &str, body: &str, expected: &str) -> Result<Value> {
    let p = page(base, id)?;
    let path = library::text(p, "path")?.to_owned();
    let old = base.read(&path)?;
    ensure!(
        storage::hash(&old) == expected,
        "conflict: page revision changed"
    );
    let mut m = base.manifest.clone();
    m["updated_at"] = library::today().into();
    validate_candidate_links(base, &m, &HashMap::from([(path.clone(), body.to_owned())]))?;
    let mw = base.write("knowledge-base.json", &m)?;
    let receipt = base.commit(vec![
        Write {
            path,
            expected: Some(expected.into()),
            bytes: Some(body.as_bytes().to_vec()),
        },
        mw,
    ])?;
    Ok(json!({"id":id,"revision":storage::hash(body.as_bytes()),"receipt":receipt}))
}
pub fn patch(base: &mut Base, id: &str, change: &Value, expected: &str) -> Result<Value> {
    let old = contents(base, id)?;
    let find = library::text(change, "find")?;
    let replacement = change["replace"]
        .as_str()
        .context("invalid_input: replace must be string")?;
    ensure!(
        old.matches(find).count() == 1,
        "ambiguous_patch: find must match exactly once"
    );
    replace(base, id, &old.replacen(find, replacement, 1), expected)
}

fn page_anchor(id: &str) -> String {
    format!("page-{id}")
}

fn heading_anchor(id: &str, heading: &str) -> String {
    // IDs cannot contain underscores, making the boundary unambiguous even for
    // page IDs containing repeated hyphens or headings containing underscores.
    format!("{}__{heading}", page_anchor(id))
}

fn reading_destination(base: &Base, from: &str, href: &str) -> Result<String> {
    let Some((path, fragment)) = local_target_in(base, &base.manifest, from, href)? else {
        return Ok(href.to_owned());
    };
    if let Some(page) = base.manifest["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["path"] == path)
    {
        let id = library::text(page, "id")?;
        return Ok(format!(
            "#{}",
            if let Some(fragment) = fragment {
                heading_anchor(id, &percent_decode(&fragment)?)
            } else {
                page_anchor(id)
            }
        ));
    }
    let mut target = url::Url::parse("https://kb.invalid/")?;
    target.set_path(&path);
    target.set_fragment(fragment.as_deref());
    Ok(target
        .as_str()
        .trim_start_matches("https://kb.invalid/")
        .to_owned())
}

fn markdown_label(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        if "\\[]*_`<>".contains(ch) {
            result.push('\\');
        }
        result.push(if ch == '\n' { ' ' } else { ch });
    }
    result
}

/// Keep the original prose/code, normalize parsed links (including references)
/// into page-scoped inline links, and add explicit portable Markdown anchors.
/// Canonical source pages are copied unchanged alongside this reading edition.
fn reading_markdown(base: &Base, id: &str, from: &str, body: &str) -> Result<String> {
    struct Link {
        range: Range<usize>,
        destination: String,
        title: String,
        label: String,
    }
    let mut stack: Vec<Link> = Vec::new();
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for (event, range) in Parser::new_ext(body, Options::all()).into_offset_iter() {
        match event {
            Event::Start(
                Tag::Link {
                    dest_url, title, ..
                }
                | Tag::Image {
                    dest_url, title, ..
                },
            ) => {
                stack.push(Link {
                    range,
                    destination: reading_destination(base, from, &dest_url)?,
                    title: title.to_string(),
                    label: String::new(),
                });
            }
            Event::Text(text) | Event::Code(text) => {
                for link in &mut stack {
                    link.label.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                for link in &mut stack {
                    link.label.push(' ');
                }
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some(link) = stack.pop()
                    && stack.is_empty()
                {
                    let title = if link.title.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " \"{}\"",
                            link.title
                                .replace('\\', "\\\\")
                                .replace('"', "\\\"")
                                .replace('\n', " ")
                        )
                    };
                    let destination = link.destination.replace('<', "%3C").replace('>', "%3E");
                    edits.push((
                        link.range,
                        format!("[{}](<{destination}>{title})", markdown_label(&link.label)),
                    ));
                }
            }
            _ => {}
        }
    }
    for heading in outline_text(body) {
        let offset = heading["byte_start"].as_u64().unwrap() as usize;
        edits.push((
            offset..offset,
            format!(
                "<a id=\"{}\"></a>\n\n",
                heading_anchor(id, heading["id"].as_str().unwrap())
            ),
        ));
    }
    edits.sort_by(|a, b| {
        b.0.start
            .cmp(&a.0.start)
            .then_with(|| b.0.end.cmp(&a.0.end))
    });
    let mut result = body.to_owned();
    for (range, replacement) in edits {
        result.replace_range(range, &replacement);
    }
    Ok(result)
}

fn html_fragment(
    markdown: &str,
    namespace: Option<&str>,
    mut destination: impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    let headings = outline_text(markdown);
    let mut heading_index = 0;
    let mut events = Vec::new();
    for event in Parser::new_ext(markdown, Options::all()) {
        events.push(match event {
            Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
            Event::Start(Tag::Heading { level, .. }) => {
                let id = headings
                    .get(heading_index)
                    .and_then(|h| h["id"].as_str())
                    .unwrap_or("heading")
                    .to_string();
                heading_index += 1;
                let id = namespace.map_or(id.clone(), |page| heading_anchor(page, &id));
                Event::Start(Tag::Heading {
                    level,
                    id: Some(id.into()),
                    classes: Vec::new(),
                    attrs: Vec::new(),
                })
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Link {
                link_type,
                dest_url: destination(&dest_url)?.into(),
                title,
                id,
            }),
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => Event::Start(Tag::Link {
                link_type: LinkType::Inline,
                dest_url: destination(&dest_url)?.into(),
                title,
                id: CowStr::Borrowed(""),
            }),
            Event::End(TagEnd::Image) => Event::End(TagEnd::Link),
            e => e,
        });
    }
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, events.into_iter());
    Ok(body)
}

fn html_document(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><title>{}</title><style>body{{max-width:78ch;margin:3rem auto;padding:0 1.2rem;font:16px/1.6 system-ui;color:#182334}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;background:#f3f5f7;padding:1rem}}table{{border-collapse:collapse}}td,th{{border:1px solid #ccd3dd;padding:.5rem}}a{{color:#2456a5}}</style><body>{body}</body></html>",
        escape(title)
    )
}

pub fn render_html(markdown: &str, title: &str) -> String {
    let body = html_fragment(markdown, None, |href| {
        if let Ok(url) = url::Url::parse(href)
            && !["https", "http", "mailto", "kb"].contains(&url.scheme())
        {
            return Ok("#invalid-link".into());
        }
        Ok(href.to_owned())
    })
    .expect("unscoped renderer has an infallible destination transform");
    html_document(title, &body)
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
pub fn export(base: &Base, output: &Path, format: &str) -> Result<Value> {
    ensure!(
        ["markdown", "html", "json"].contains(&format),
        "invalid_format"
    );
    validate(base)?;
    let output = storage::safe_absolute(output)?;
    ensure!(
        !output.starts_with(&base.library) && !base.library.starts_with(&output),
        "unsafe_output: export must be outside library"
    );
    ensure!(!output.exists(), "already_exists: output directory");
    let parent = output.parent().context("missing output parent")?;
    storage::private_dir(parent)?;
    let stage = tempfile::Builder::new()
        .prefix(".tars-kb-export-")
        .tempdir_in(parent)?;
    let mut reading = format!(
        "# {}\n\nContent version: {}. Visibility: {}.\n\n",
        markdown_label(library::text(&base.manifest, "title")?),
        markdown_label(library::text(&base.manifest, "version")?),
        markdown_label(library::text(&base.manifest, "visibility")?)
    );
    let mut html = format!(
        "<h1>{}</h1><p>Content version: {}. Visibility: {}.</p><nav aria-label=\"Pages\"><ul>",
        escape(library::text(&base.manifest, "title")?),
        escape(library::text(&base.manifest, "version")?),
        escape(library::text(&base.manifest, "visibility")?)
    );
    reading.push_str("## Contents\n\n");
    for page in base.manifest["pages"].as_array().unwrap() {
        let id = library::text(page, "id")?;
        let title = library::text(page, "title")?;
        reading.push_str(&format!(
            "- [{}](#{})\n",
            markdown_label(title),
            page_anchor(id)
        ));
        html.push_str(&format!(
            "<li><a href=\"#{}\">{}</a></li>",
            page_anchor(id),
            escape(title)
        ));
    }
    reading.push('\n');
    html.push_str("</ul></nav>");
    let mut pages = Vec::new();
    for p in base.manifest["pages"].as_array().unwrap() {
        let id = library::text(p, "id")?;
        let body = contents(base, id)?;
        let from = library::text(p, "path")?;
        let combined_body = reading_markdown(base, id, from, &body)?;
        reading.push_str(&format!(
            "<a id=\"{}\"></a>\n\n## {}\n\nPage: `{id}` · status: `{}` · sources: `{}`\n\n{combined_body}\n\n",
            page_anchor(id),
            markdown_label(library::text(p, "title")?),
            library::text(p, "status")?,
            p["sources"]
        ));
        html.push_str(&format!("<section id=\"{}\"><h2>{}</h2><p>Page: <code>{}</code>; status: {}; sources: {}</p>{}</section>",
            page_anchor(id), escape(library::text(p, "title")?), escape(id),
            escape(library::text(p, "status")?), escape(&p["sources"].to_string()),
            html_fragment(&body, Some(id), |href| reading_destination(base, from, href))?));
        pages.push(json!({"metadata":p,"content":body}));
    }
    // The portable pages/ tree retains original bytes. Only the reading edition
    // receives scoped headings and normalized links; raw HTML remains inert.
    for path in library::files_recursive(&base.root)? {
        let target = stage.path().join(&path);
        storage::atomic(&target, &base.read(&path)?)?;
    }
    match format {
        "markdown" => storage::atomic(&stage.path().join("reading.md"), reading.as_bytes())?,
        "html" => storage::atomic(
            &stage.path().join("reading.html"),
            html_document(library::text(&base.manifest, "title")?, &html).as_bytes(),
        )?,
        "json" => storage::atomic(
            &stage.path().join("knowledge.json"),
            &storage::json_bytes(&json!({"manifest":base.manifest,"pages":pages}))?,
        )?,
        _ => unreachable!(),
    }
    let mut checks = String::new();
    for path in library::files_recursive(stage.path())? {
        checks.push_str(&format!(
            "{}  {path}\n",
            storage::hash(&storage::read_bytes(&stage.path().join(&path))?)
        ));
    }
    storage::atomic(&stage.path().join("SHA256SUMS"), checks.as_bytes())?;
    // rename does not overwrite an existing non-empty destination; check existence
    // again to keep the local-owner no-overwrite contract clear.
    ensure!(!output.exists(), "already_exists: output directory");
    fs::rename(stage.path(), &output)?;
    storage::sync_dir(parent)?;
    Ok(
        json!({"id":base.id,"output":output,"format":format,"pages":pages.len(),"assets":base.manifest["assets"].as_array().unwrap().len()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heading_sections_have_stable_duplicates() {
        let h = outline_text("# One\n\n## Two\n\n## Two\n");
        assert_eq!(h[2]["id"], "two-1");
    }
    #[test]
    fn html_escapes_raw_script_and_does_not_embed_images() {
        let h = render_html(
            "<script>alert(1)</script>\n\n![alt](https://example.com/a.png)",
            "x",
        );
        assert!(!h.contains("<script>"));
        assert!(!h.contains("<img"));
        assert!(h.contains("default-src 'none'"));
    }
}
