use crate::{
    Result,
    storage::{self, Lock, Write},
};
use anyhow::{Context, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub struct Base {
    pub library: PathBuf,
    pub root: PathBuf,
    pub id: String,
    pub manifest: Value,
    lock: Lock,
    declared_paths: HashSet<String>,
}
pub fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value.as_bytes()[0].is_ascii_lowercase()
            && value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
        "invalid_id: use a lowercase letter followed by letters, digits or hyphens (up to 128 bytes)"
    );
    Ok(())
}
pub fn now() -> String {
    jiff::Timestamp::now().to_string()
}
pub fn today() -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string()
}
pub fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    let s = v
        .get(k)
        .and_then(Value::as_str)
        .with_context(|| format!("invalid_metadata: {k} must be a string"))?;
    ensure!(
        !s.trim().is_empty(),
        "invalid_metadata: {k} must not be empty"
    );
    Ok(s)
}
pub fn array<'a>(v: &'a Value, k: &str) -> Result<&'a Vec<Value>> {
    v.get(k)
        .and_then(Value::as_array)
        .with_context(|| format!("invalid_metadata: {k} must be an array"))
}
pub fn version(v: &Value) -> Result<()> {
    let s = text(v, "schema_version")?;
    let (major, minor) = s
        .split_once('.')
        .context("unsupported_version: expected major.minor")?;
    ensure!(
        major == "1" && !minor.is_empty() && minor.bytes().all(|c| c.is_ascii_digit()),
        "unsupported_version: supported storage major is 1"
    );
    if let Some(features) = v.get("required_features") {
        ensure!(
            features.as_array().is_some_and(|a| a.is_empty()),
            "unsupported_feature: unknown required features"
        );
    }
    Ok(())
}
fn date(value: &Value, key: &str) -> Result<()> {
    let s = text(value, key)?;
    ensure!(s.len() == 10, "invalid_date: expected YYYY-MM-DD");
    s.parse::<jiff::civil::Date>().context("invalid_date")?;
    Ok(())
}
fn unique_rows<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    let rows = array(value, key)?;
    let mut ids = HashSet::new();
    for row in rows {
        let id = text(row, "id")?;
        identifier(id)?;
        ensure!(ids.insert(id), "invalid_metadata: duplicate {key} id {id}");
    }
    Ok(rows)
}
pub fn validate_manifest(m: &Value, id: &str) -> Result<()> {
    version(m)?;
    ensure!(
        text(m, "id")? == id,
        "invalid_metadata: manifest/catalog ID mismatch"
    );
    for k in ["title", "summary", "owner", "language"] {
        text(m, k)?;
    }
    ensure!(
        ["private", "shareable"].contains(&text(m, "visibility")?),
        "invalid_metadata: invalid visibility"
    );
    let version = text(m, "version")?;
    let v = version.split(['-', '+']).next().unwrap();
    ensure!(
        v.split('.').count() == 3
            && v.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())),
        "invalid_metadata: content version must be semver"
    );
    date(m, "created_at")?;
    date(m, "updated_at")?;
    let pages = unique_rows(m, "pages")?;
    ensure!(!pages.is_empty(), "invalid_metadata: base must have a page");
    let sources = unique_rows(m, "sources")?;
    let assets = unique_rows(m, "assets")?;
    let source_ids: HashSet<&str> = sources.iter().map(|s| s["id"].as_str().unwrap()).collect();
    let mut paths = HashSet::new();
    let mut content_ids = HashSet::new();
    for s in sources {
        text(s, "title")?;
        text(s, "uri")?;
        date(s, "checked_at")?;
        ensure!(
            [
                "workspace",
                "repository",
                "official",
                "conversation",
                "observation"
            ]
            .contains(&text(s, "type")?),
            "invalid_metadata: source type"
        );
    }
    for p in pages {
        let pid = text(p, "id")?;
        content_ids.insert(pid);
        text(p, "title")?;
        let path = text(p, "path")?;
        ensure!(
            path.starts_with("pages/") && path.ends_with(".md"),
            "unsafe_path: page path"
        );
        storage::relative_path(path)?;
        ensure!(
            paths.insert(path),
            "invalid_metadata: duplicate content path"
        );
        ensure!(
            [
                "overview",
                "concept",
                "reference",
                "procedure",
                "decision",
                "status"
            ]
            .contains(&text(p, "kind")?),
            "invalid_metadata: page kind"
        );
        ensure!(
            [
                "documented",
                "implemented",
                "observed",
                "proposed",
                "pending"
            ]
            .contains(&text(p, "status")?),
            "invalid_metadata: page status"
        );
        for k in ["sources", "tags"] {
            let mut seen = HashSet::new();
            for value in array(p, k)? {
                let s = value
                    .as_str()
                    .context("invalid_metadata: tags/sources must be strings")?;
                ensure!(
                    !s.is_empty() && seen.insert(s),
                    "invalid_metadata: empty/duplicate tag or source"
                );
                if k == "sources" {
                    ensure!(
                        source_ids.contains(s),
                        "invalid_metadata: undeclared source {s}"
                    );
                }
            }
        }
    }
    ensure!(
        pages
            .iter()
            .any(|p| p["id"].as_str() == m["entrypoint"].as_str()),
        "invalid_metadata: missing entrypoint"
    );
    for a in assets {
        ensure!(
            content_ids.insert(text(a, "id")?),
            "invalid_metadata: duplicate page/asset id"
        );
        let path = text(a, "path")?;
        let media = text(a, "media_type")?;
        ensure!(
            media.contains('/') && !media.contains(char::is_whitespace),
            "invalid_metadata: media type"
        );
        if media.eq_ignore_ascii_case("application/json") {
            ensure!(
                path.starts_with("data/") && path.ends_with(".json"),
                "unsafe_path: JSON asset path"
            );
        } else {
            ensure!(
                path.starts_with("assets/"),
                "unsafe_path: opaque asset path"
            );
        }
        storage::relative_path(path)?;
        ensure!(
            paths.insert(path),
            "invalid_metadata: duplicate content path"
        );
    }
    Ok(())
}
pub fn catalog(library: &Path) -> Result<(PathBuf, Value)> {
    let root = library
        .canonicalize()
        .context("library_not_found: use --root or init a library")?;
    ensure!(root.is_dir(), "library_not_found: root is not a directory");
    let _lock = Lock::acquire(&root, "catalog", true)?;
    let journal = storage::confined(&root, ".runtime/catalog.json")?;
    storage::recover(&root, &journal)?;
    let c = storage::read_json(&storage::confined(&root, "catalog.json")?)?;
    validate_catalog(&root, &c)?;
    Ok((root, c))
}
fn validate_catalog(root: &Path, c: &Value) -> Result<()> {
    version(c)?;
    for row in unique_rows(c, "knowledge_bases")? {
        let id = text(row, "id")?;
        ensure!(
            text(row, "path")? == format!("bases/{id}"),
            "unsafe_path: noncanonical base path"
        );
        storage::confined(root, text(row, "path")?)?;
    }
    Ok(())
}
impl Base {
    pub fn open(library: &Path, id: &str) -> Result<Self> {
        identifier(id)?;
        let (library, c) = catalog(library)?;
        ensure!(
            array(&c, "knowledge_bases")?.iter().any(|r| r["id"] == id),
            "not_found: unknown knowledge base {id}"
        );
        let root = storage::confined(&library, &format!("bases/{id}"))?;
        ensure!(root.is_dir(), "not_found: missing base directory");
        let lock = Lock::acquire(&library, &format!("base-{id}"), false)?;
        let journal = storage::confined(&library, &format!(".runtime/transactions/{id}.json"))?;
        if journal.exists() || root.join(".tars-kb-pending").exists() {
            // Release before upgrading: two recovering readers must not deadlock.
            lock.unlock()?;
            lock.set(true)?;
            storage::recover(&root, &journal)?;
            lock.set(false)?;
        }
        let manifest = storage::read_json(&storage::confined(&root, "knowledge-base.json")?)?;
        validate_manifest(&manifest, id)?;
        let declared_paths = declared_paths(&manifest);
        Ok(Self {
            library,
            root,
            id: id.into(),
            manifest,
            lock,
            declared_paths,
        })
    }
    pub fn read_json(&self, relative: &str) -> Result<Value> {
        storage::read_json(&storage::confined(&self.root, relative)?)
    }
    pub fn read(&self, relative: &str) -> Result<Vec<u8>> {
        storage::read_bytes(&storage::confined(&self.root, relative)?)
    }
    pub fn revision(&self) -> Result<String> {
        Ok(storage::hash(&self.read("knowledge-base.json")?))
    }
    pub fn write(&self, path: &str, value: &Value) -> Result<Write> {
        let p = storage::confined(&self.root, path)?;
        let expected = if p.exists() {
            Some(storage::hash(&storage::read_bytes(&p)?))
        } else {
            None
        };
        Ok(Write {
            path: path.into(),
            expected,
            bytes: Some(storage::json_bytes(value)?),
        })
    }
    pub fn commit(&mut self, mut writes: Vec<Write>) -> Result<Value> {
        let manifest_before = storage::hash(&storage::json_bytes(&self.manifest)?);
        self.lock.unlock()?;
        self.lock.set(true)?;
        let result = (|| {
            let journal = storage::confined(
                &self.library,
                &format!(".runtime/transactions/{}.json", self.id),
            )?;
            storage::recover(&self.root, &journal)?;
            let current = self.read_json("knowledge-base.json")?;
            ensure!(
                storage::hash(&storage::json_bytes(&current)?) == manifest_before,
                "conflict: manifest changed since read"
            );
            if let Some(w) = writes.iter().find(|w| w.path == "knowledge-base.json") {
                let bytes = w
                    .bytes
                    .as_ref()
                    .context("invalid_transaction: cannot delete manifest")?;
                validate_manifest(&storage::parse_json(bytes)?, &self.id)?;
            }
            let mut next_manifest =
                if let Some(w) = writes.iter().find(|w| w.path == "knowledge-base.json") {
                    storage::parse_json(w.bytes.as_ref().unwrap())?
                } else {
                    current.clone()
                };
            let mut generation = storage::json_bytes(&current)?;
            for w in &writes {
                generation.extend_from_slice(w.path.as_bytes());
                if let Some(b) = &w.bytes {
                    generation.extend_from_slice(storage::hash(b).as_bytes());
                }
            }
            next_manifest["x-tars-kb-revision"] = storage::hash(&generation).into();
            let next_bytes = storage::json_bytes(&next_manifest)?;
            if let Some(w) = writes.iter_mut().find(|w| w.path == "knowledge-base.json") {
                w.bytes = Some(next_bytes);
            } else {
                writes.push(Write {
                    path: "knowledge-base.json".into(),
                    expected: Some(storage::hash(&self.read("knowledge-base.json")?)),
                    bytes: Some(next_bytes),
                });
            }
            let tx = storage::transact(&self.root, &journal, &writes)?;
            self.manifest = self.read_json("knowledge-base.json")?;
            self.declared_paths = declared_paths(&self.manifest);
            Ok(
                json!({"committed":true,"transaction":tx,"revision":self.revision()?,"files":writes.len()}),
            )
        })();
        self.lock.set(false)?;
        result
    }
    pub fn summary(&self) -> Value {
        json!({"id":self.id,"title":self.manifest["title"],"version":self.manifest["version"],"visibility":self.manifest["visibility"],"pages":self.manifest["pages"].as_array().map_or(0,Vec::len),"assets":self.manifest["assets"].as_array().map_or(0,Vec::len)})
    }
    pub fn declared(&self, path: &str) -> bool {
        self.declared_paths.contains(path)
    }
}
fn declared_paths(manifest: &Value) -> HashSet<String> {
    ["pages", "assets"]
        .iter()
        .flat_map(|key| {
            manifest[*key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p["path"].as_str().map(str::to_owned))
        })
        .collect()
}
pub fn list(root: &Path) -> Result<Value> {
    let (root, c) = catalog(root)?;
    let mut out = Vec::new();
    for row in array(&c, "knowledge_bases")? {
        let b = Base::open(&root, text(row, "id")?)?;
        out.push(b.summary());
    }
    Ok(out.into())
}
pub fn initialize(library: &Path, id: &str, title: &str, owner: &str) -> Result<Value> {
    identifier(id)?;
    ensure!(
        !title.trim().is_empty() && !owner.trim().is_empty(),
        "invalid_input: title/owner required"
    );
    if !library.exists() {
        let p = storage::safe_absolute(library)?;
        storage::private_dir(&p)?;
    }
    let root = library.canonicalize()?;
    let _lock = Lock::acquire(&root, "catalog", true)?;
    let journal = storage::confined(&root, ".runtime/catalog.json")?;
    storage::recover(&root, &journal)?;
    let catpath = storage::confined(&root, "catalog.json")?;
    let mut cat = if catpath.exists() {
        storage::read_json(&catpath)?
    } else {
        json!({"schema_version":"1.0","knowledge_bases":[]})
    };
    validate_catalog(&root, &cat)?;
    ensure!(
        !array(&cat, "knowledge_bases")?
            .iter()
            .any(|r| r["id"] == id),
        "already_exists: base already registered"
    );
    ensure!(
        !root.join(format!("bases/{id}")).exists(),
        "already_exists: base directory exists"
    );
    let m = json!({"schema_version":"1.0","id":id,"title":title,"summary":title,"version":"0.1.0","visibility":"private","owner":owner,"language":"en","created_at":today(),"updated_at":today(),"entrypoint":"overview","pages":[{"id":"overview","title":title,"path":"pages/overview.md","kind":"overview","status":"documented","sources":[],"tags":[]}],"sources":[],"assets":[]});
    cat["knowledge_bases"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":id,"path":format!("bases/{id}")}));
    let expected = if catpath.exists() {
        Some(storage::hash(&storage::read_bytes(&catpath)?))
    } else {
        None
    };
    storage::transact(
        &root,
        &journal,
        &[
            Write {
                path: format!("bases/{id}/knowledge-base.json"),
                expected: None,
                bytes: Some(storage::json_bytes(&m)?),
            },
            Write {
                path: format!("bases/{id}/pages/overview.md"),
                expected: None,
                bytes: Some(
                    format!("# {title}\n\nDescribe this knowledge base's purpose and sources.\n")
                        .into_bytes(),
                ),
            },
            Write {
                path: "catalog.json".into(),
                expected,
                bytes: Some(storage::json_bytes(&cat)?),
            },
        ],
    )?;
    Ok(json!({"id":id,"path":root.join(format!("bases/{id}"))}))
}
pub fn source_add(base: &mut Base, input: Value) -> Result<Value> {
    let id = text(&input, "id")?.to_owned();
    identifier(&id)?;
    ensure!(
        !array(&base.manifest, "sources")?
            .iter()
            .any(|s| s["id"] == id),
        "already_exists: source ID"
    );
    let mut m = base.manifest.clone();
    m["sources"].as_array_mut().unwrap().push(input);
    m["updated_at"] = today().into();
    validate_manifest(&m, &base.id)?;
    let w = base.write("knowledge-base.json", &m)?;
    let receipt = base.commit(vec![w])?;
    Ok(json!({"id":id,"receipt":receipt}))
}
pub fn files_recursive(root: &Path) -> Result<Vec<String>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        for e in fs::read_dir(dir)? {
            let p = e?.path();
            let m = fs::symlink_metadata(&p)?;
            ensure!(
                !m.file_type().is_symlink(),
                "unsafe_path: symlink inside base"
            );
            if m.is_dir() {
                walk(root, &p, out)?;
            } else {
                ensure!(m.is_file(), "unsafe_path: special file inside base");
                out.push(
                    p.strip_prefix(root)?
                        .to_str()
                        .context("invalid UTF-8 path")?
                        .into(),
                );
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}
