//! Scoped literal and ranked retrieval. Indexes contain derived private content,
//! never canonical records. An indexed snapshot is not a live filesystem claim.

use crate::{Result, library::Base, storage};
use anyhow::{Context, bail, ensure};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use unicode_casefold::UnicodeCaseFold;

const INDEX_SCHEMA: &str = "1";
const PARSER: &str = "markdown-headings-1/unicode61-1";

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub ranked: bool,
    pub verified: bool,
    pub limit: usize,
    pub max_bytes: usize,
    pub concept: Option<String>,
    pub cache_root: Option<PathBuf>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            ranked: false,
            verified: false,
            limit: 8,
            max_bytes: 8192,
            concept: None,
            cache_root: None,
        }
    }
}

struct Document {
    key: String,
    title: String,
    headings: String,
    body: String,
    metadata: Value,
    digest: String,
}

struct Snapshot {
    documents: Vec<Document>,
    signature: String,
    manifest_revision: String,
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("invalid_metadata: missing string {key}"))
}

fn folded(value: &str) -> String {
    value.case_fold().collect()
}

fn heading_text(markdown: &str) -> String {
    let mut active = false;
    let mut text = String::new();
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::Heading { .. }) => active = true,
            Event::End(TagEnd::Heading(_)) => {
                active = false;
                text.push('\n');
            }
            Event::Text(value) | Event::Code(value) if active => text.push_str(&value),
            _ => {}
        }
    }
    text
}

fn finish_document(
    key: String,
    title: String,
    headings: String,
    body: String,
    metadata: Value,
) -> Result<Document> {
    let digest = storage::hash(&serde_json::to_vec(&json!({
        "title": title, "headings": headings, "body": body, "metadata": metadata
    }))?);
    Ok(Document {
        key,
        title,
        headings,
        body,
        metadata,
        digest,
    })
}

fn snapshot(base: &Base) -> Result<Snapshot> {
    let manifest_bytes = base.read("knowledge-base.json")?;
    let manifest: Value = storage::parse_json(&manifest_bytes)?;
    ensure!(
        manifest == base.manifest,
        "working_tree_changed: reopen the base before collecting search inputs"
    );
    let mut sources = BTreeMap::new();
    sources.insert(
        "knowledge-base.json".to_owned(),
        storage::hash(&manifest_bytes),
    );
    let mut documents = Vec::new();
    for page in base.manifest["pages"]
        .as_array()
        .context("invalid_manifest: pages must be an array")?
    {
        let id = required(page, "id")?;
        let path = required(page, "path")?;
        let bytes = base.read(path)?;
        let revision = storage::hash(&bytes);
        sources.insert(path.to_owned(), revision.clone());
        let body = String::from_utf8(bytes).context("invalid_page: Markdown must be UTF-8")?;
        let metadata = json!({
            "kind": "page", "id": id, "page": id, "path": path,
            "reference": format!("kb://{}/{}",base.id,id),
            "title": page["title"], "status": page["status"],
            "sources": page["sources"], "tags": page["tags"],
            "concepts": page.get("concepts").cloned().unwrap_or_else(||json!([])),
            "revision": revision
        });
        documents.push(finish_document(
            format!("page:{id}"),
            required(page, "title")?.to_owned(),
            heading_text(&body),
            body,
            metadata,
        )?);
    }
    let assets = base.manifest["assets"]
        .as_array()
        .context("invalid_manifest: assets must be an array")?;
    let declared: HashMap<&str, &Value> = assets
        .iter()
        .filter_map(|asset| asset["path"].as_str().map(|path| (path, asset)))
        .collect();
    let profile_path = "data/memory/profile.json";
    if declared.contains_key(profile_path) {
        let profile_bytes = base.read(profile_path)?;
        let profile: Value = storage::parse_json(&profile_bytes)?;
        sources.insert(profile_path.to_owned(), storage::hash(&profile_bytes));
        let mut concept_terms = HashMap::new();
        for concept in profile["concepts"]
            .as_array()
            .context("invalid_memory: profile concept inventory is missing")?
        {
            let id = concept.as_str().context("invalid_memory: concept ID")?;
            crate::library::identifier(id)?;
            let path = format!("data/memory/concepts/{id}.json");
            ensure!(
                declared.contains_key(path.as_str()),
                "undeclared_concept: {path}"
            );
            let bytes = base.read(&path)?;
            sources.insert(path, storage::hash(&bytes));
            let concept = storage::parse_json(&bytes)?;
            ensure!(
                concept["id"] == id
                    && concept["schema_version"] == "1.0"
                    && concept["profile"] == crate::memory::PROFILE,
                "invalid_memory: concept identity or profile mismatch"
            );
            if let Some(features) = concept.get("required_features") {
                ensure!(
                    features.as_array().is_some_and(|values| values.is_empty()),
                    "unsupported_feature: unknown required concept features"
                );
            }
            let expected = required(&concept, "revision")?;
            let mut canonical = concept.clone();
            canonical
                .as_object_mut()
                .context("invalid_memory: concept object")?
                .remove("revision");
            ensure!(
                expected == storage::hash(&serde_json::to_vec(&canonical)?),
                "history_integrity: concept bytes differ from accepted revision"
            );
            let mut terms = required(&concept, "title")?.to_owned();
            for alias in concept["aliases"]
                .as_array()
                .context("invalid_memory: concept aliases must be an array")?
            {
                terms.push('\n');
                terms.push_str(
                    alias
                        .as_str()
                        .context("invalid_memory: concept alias must be a string")?,
                );
            }
            concept_terms.insert(id.to_owned(), terms);
        }
        for id in profile["assertions"]
            .as_array()
            .context("invalid_memory: profile assertion inventory is missing")?
        {
            let id = id.as_str().context("invalid_memory: assertion ID")?;
            crate::library::identifier(id)?;
            let path = format!("data/memory/assertions/{id}.json");
            ensure!(
                declared.contains_key(path.as_str()),
                "undeclared_assertion: {path}"
            );
            sources.insert(path.clone(), storage::hash(&base.read(&path)?));
        }
        let facts = crate::memory::facts(base, None, &crate::memory::FactFilter::default())?;
        for fact in facts["items"]
            .as_array()
            .context("invalid_memory: expected facts items array")?
        {
            let content = &fact["content"];
            let id = required(fact, "id")?;
            let body = required(content, "text")?.to_owned();
            let mut headings = String::new();
            for concept in content["concepts"]
                .as_array()
                .context("invalid_memory: assertion concepts must be an array")?
            {
                let concept = concept.as_str().context("invalid_memory: concept ID")?;
                let terms = concept_terms
                    .get(concept)
                    .context("invalid_memory: unknown assertion concept")?;
                headings.push_str(terms);
                headings.push('\n');
            }
            let metadata = json!({
                "kind": "assertion", "id": id,
                "reference": format!("kb-memory://{}/assertion/{}",base.id,id),
                "path": format!("data/memory/assertions/{id}.json"),
                "concepts": content["concepts"], "basis": content["basis"],
                "temporal": content["temporal"], "scope": content["scope"],
                "sources": content["sources"], "revision": fact["revision"],
                "sequence": fact["sequence"], "recorded_at": fact["recorded_at"],
                "temporal_status": fact["temporal_status"], "freshness": fact["freshness"],
                "conflicts": fact["conflicts"]
            });
            documents.push(finish_document(
                format!("assertion:{id}"),
                id.to_owned(),
                headings,
                body,
                metadata,
            )?);
        }
    }
    Ok(Snapshot {
        documents,
        signature: storage::hash(&serde_json::to_vec(&sources)?),
        manifest_revision: storage::hash(&manifest_bytes),
    })
}

/// Legacy page-only case-folded substring search with the existing JSON hit shape.
pub fn literal(base: &Base, query: &str) -> Result<Value> {
    ensure!(!query.trim().is_empty(), "invalid_query: query is empty");
    let query = folded(query);
    let mut hits = Vec::new();
    for page in base.manifest["pages"]
        .as_array()
        .context("invalid_manifest: pages must be an array")?
    {
        let path = required(page, "path")?;
        let bytes = base.read(path)?;
        let text = std::str::from_utf8(&bytes).context("invalid_page: UTF-8 required")?;
        for (line, text) in text.lines().enumerate() {
            if folded(text).contains(&query) {
                hits.push(json!({
                    "knowledge_base":base.id,"page":page["id"],"path":path,
                    "line":line+1,"text":text
                }));
            }
        }
    }
    Ok(Value::Array(hits))
}

fn no_symlinks(path: &Path) -> Result<()> {
    for component in path.ancestors() {
        ensure!(
            !component.is_symlink(),
            "unsafe_cache_path: symbolic links are not allowed: {}",
            component.display()
        );
    }
    Ok(())
}

fn cache_path(base: &Base, root: Option<&Path>, create: bool) -> Result<PathBuf> {
    crate::library::identifier(&base.id)?;
    let root = if let Some(root) = root {
        root.to_owned()
    } else if let Some(root) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(root).join("tars-kb")
    } else {
        PathBuf::from(std::env::var_os("HOME").context("cache_unavailable: HOME is unset")?)
            .join(".cache/tars-kb")
    };
    ensure!(
        root.is_absolute(),
        "unsafe_cache_path: root must be absolute"
    );
    let root = storage::safe_absolute(&root).context("unsafe_cache_path")?;
    let library = base.library.canonicalize()?;
    ensure!(
        !root.starts_with(&library) && !library.starts_with(&root),
        "unsafe_cache_path: cache must be separate from the canonical library"
    );
    no_symlinks(&root)?;
    let key = storage::hash(library.as_os_str().as_encoded_bytes());
    let directory = root.join(key).join(&base.id);
    no_symlinks(&directory)?;
    if create {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(&directory)?;
        // Never chmod a caller-supplied ancestor such as /tmp.
        fs::set_permissions(
            directory.parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(directory.join("search-v1.sqlite3"))
}

fn library_key(base: &Base) -> Result<String> {
    Ok(storage::hash(
        base.library.canonicalize()?.as_os_str().as_encoded_bytes(),
    ))
}

fn validate_index_identity(base: &Base, metadata: &BTreeMap<String, String>) -> Result<()> {
    ensure!(
        metadata.get("base_id").map(String::as_str) == Some(base.id.as_str())
            && metadata.get("library_key") == Some(&library_key(base)?),
        "index_identity_mismatch: rebuild the selected base index"
    );
    Ok(())
}

fn cache_lock(path: &Path, exclusive: bool) -> Result<File> {
    let path = path.with_file_name("index.lock");
    no_symlinks(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(exclusive)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .context("index_missing: create the selected base index first")?;
    if exclusive {
        file.lock()?;
    } else {
        file.lock_shared()?;
    }
    Ok(file)
}

fn configure(connection: &Connection) -> Result<()> {
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.pragma_update(None, "trusted_schema", "OFF")?;
    Ok(())
}

fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
         CREATE TABLE docs(key TEXT UNIQUE NOT NULL,digest TEXT NOT NULL,metadata TEXT NOT NULL,concepts TEXT NOT NULL);
         CREATE VIRTUAL TABLE search_text USING fts5(title,headings,body,tokenize='unicode61');",
    )?;
    Ok(())
}

fn metadata(connection: &Connection) -> Result<BTreeMap<String, String>> {
    let mut statement = connection.prepare("SELECT key,value FROM meta")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut values = BTreeMap::new();
    for row in rows {
        let (key, value) = row?;
        values.insert(key, value);
    }
    ensure!(
        values.get("schema_version").map(String::as_str) == Some(INDEX_SCHEMA)
            && values.get("parser").map(String::as_str) == Some(PARSER),
        "index_version_unsupported: rebuild the selected base index"
    );
    Ok(values)
}

fn update_connection(connection: &mut Connection, base: &Base, source: &Snapshot) -> Result<Value> {
    let existing: HashMap<String, (i64, String)> = {
        let mut statement = connection.prepare("SELECT key,rowid,digest FROM docs")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?;
        let mut values = HashMap::new();
        for row in rows {
            let (key, value) = row?;
            values.insert(key, value);
        }
        values
    };
    let transaction = connection.transaction()?;
    let mut changed = 0;
    for doc in &source.documents {
        if existing.get(&doc.key).map(|(_, digest)| digest.as_str()) == Some(&doc.digest) {
            continue;
        }
        let rowid = if let Some((rowid, _)) = existing.get(&doc.key) {
            transaction.execute("DELETE FROM search_text WHERE rowid=?", [rowid])?;
            transaction.execute(
                "UPDATE docs SET digest=?,metadata=?,concepts=? WHERE rowid=?",
                params![
                    doc.digest,
                    serde_json::to_string(&doc.metadata)?,
                    serde_json::to_string(&doc.metadata["concepts"])?,
                    rowid
                ],
            )?;
            *rowid
        } else {
            transaction.execute(
                "INSERT INTO docs(key,digest,metadata,concepts) VALUES(?,?,?,?)",
                params![
                    doc.key,
                    doc.digest,
                    serde_json::to_string(&doc.metadata)?,
                    serde_json::to_string(&doc.metadata["concepts"])?
                ],
            )?;
            transaction.last_insert_rowid()
        };
        transaction.execute(
            "INSERT INTO search_text(rowid,title,headings,body) VALUES(?,?,?,?)",
            params![rowid, doc.title, doc.headings, doc.body],
        )?;
        changed += 1;
    }
    let mut removed = 0;
    let current: std::collections::HashSet<&str> = source
        .documents
        .iter()
        .map(|doc| doc.key.as_str())
        .collect();
    for (key, (rowid, _)) in &existing {
        if !current.contains(key.as_str()) {
            transaction.execute("DELETE FROM search_text WHERE rowid=?", [rowid])?;
            transaction.execute("DELETE FROM docs WHERE rowid=?", [rowid])?;
            removed += 1;
        }
    }
    let values = [
        ("schema_version", INDEX_SCHEMA.to_owned()),
        ("parser", PARSER.to_owned()),
        ("base_id", base.id.clone()),
        ("library_key", library_key(base)?),
        ("manifest_revision", source.manifest_revision.clone()),
        ("source_signature", source.signature.clone()),
        ("indexed_at", crate::library::now()),
    ];
    for (key, value) in values {
        transaction.execute(
            "INSERT INTO meta(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key,value],
        )?;
    }
    transaction.commit()?;
    Ok(json!({
        "base_id":base.id,"documents":source.documents.len(),"updated":changed,"removed":removed,
        "source_signature":source.signature,"manifest_revision":source.manifest_revision,
        "consistency":"verified_source_snapshot","index_schema":INDEX_SCHEMA
    }))
}

/// Scan only declared page/memory inputs and update changed index rows. Explicit
/// rebuild replaces a corrupt/obsolete cache without altering canonical content.
pub fn index(base: &Base, rebuild: bool, cache_root: Option<&Path>) -> Result<Value> {
    let source = snapshot(base)?;
    let path = cache_path(base, cache_root, true)?;
    let _lock = cache_lock(&path, true)?;
    no_symlinks(&path)?;
    if rebuild || !path.exists() {
        let temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
        let mut connection = Connection::open(temporary.path())?;
        configure(&connection)?;
        create_schema(&connection)?;
        let result = update_connection(&mut connection, base, &source)?;
        connection.close().map_err(|(_, error)| error)?;
        temporary.persist(&path).map_err(|error| error.error)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok(result)
    } else {
        let mut connection =
            Connection::open(&path).context("index_unreadable: use an explicit index rebuild")?;
        configure(&connection)?;
        let meta =
            metadata(&connection).context("index_unreadable: use an explicit index rebuild")?;
        validate_index_identity(base, &meta)?;
        update_connection(&mut connection, base, &source)
    }
}

fn clipped(value: &str, bytes: usize) -> String {
    if value.len() <= bytes {
        return value.to_owned();
    }
    let mut end = bytes.saturating_sub(3).min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

fn envelope(base: &Base, query: &str, mode: &str, signature: &str, revision: &str) -> Value {
    json!({
        "output_version":"1.0","base_id":base.id,"query":query,"mode":mode,
        "source_signature":signature,"snapshot_revision":revision,
        "consistency":if mode=="ranked" {"indexed_snapshot_unverified"}else{"verified_source_snapshot"},
        "scope":"declared pages and supported current nonretracted memory assertions; opaque assets/evidence not searched",
        "hits":[],"truncated":false,"next_cursor":null,
        "pagination":"stable search cursors are not yet supported; increase limit to expand this snapshot"
    })
}

fn add_hit(envelope: &mut Value, hit: Value, options: &SearchOptions) -> Result<bool> {
    let reference = hit.get("reference").cloned().unwrap_or(Value::Null);
    let required_bytes = serde_json::to_vec(&hit)?.len();
    let hits = envelope["hits"].as_array_mut().unwrap();
    if hits.len() >= options.limit {
        envelope["truncated"] = json!(true);
        return Ok(false);
    }
    hits.push(hit);
    if serde_json::to_vec(envelope)?.len() > options.max_bytes {
        envelope["hits"].as_array_mut().unwrap().pop();
        envelope["truncated"] = json!(true);
        let reference_only = json!({
            "kind":"reference","reference":reference,"required_item_bytes":required_bytes,
            "reason":"complete hit and its qualifiers exceed the remaining output budget"
        });
        envelope["hits"]
            .as_array_mut()
            .unwrap()
            .push(reference_only);
        if serde_json::to_vec(envelope)?.len() > options.max_bytes {
            envelope["hits"].as_array_mut().unwrap().pop();
        }
        return Ok(false);
    }
    Ok(true)
}

fn matches_concept(metadata: &Value, concept: Option<&str>) -> bool {
    concept.is_none_or(|concept| {
        metadata["concepts"]
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(concept)))
    })
}

/// New bounded response. Legacy callers should use `literal` to retain the
/// original page-only, unbounded hit-array shape.
pub fn search(base: &Base, query: &str, options: &SearchOptions) -> Result<Value> {
    ensure!(!query.trim().is_empty(), "invalid_query: query is empty");
    ensure!(
        options.limit > 0,
        "invalid_limit: expected a positive limit"
    );
    if let Some(concept) = &options.concept {
        crate::library::identifier(concept)?;
    }
    if !options.ranked {
        let source = snapshot(base)?;
        let mut result = envelope(
            base,
            query,
            "literal",
            &source.signature,
            &source.manifest_revision,
        );
        ensure!(
            serde_json::to_vec(&result)?.len() <= options.max_bytes,
            "output_budget_too_small: cannot fit the search response metadata"
        );
        let query = folded(query);
        for doc in source.documents {
            if !matches_concept(&doc.metadata, options.concept.as_deref()) {
                continue;
            }
            for (line, text) in doc.body.lines().enumerate() {
                if folded(text).contains(&query) {
                    let mut hit = doc.metadata.clone();
                    hit["line"] = json!(line + 1);
                    hit["snippet"] = json!(clipped(text, 384));
                    if !add_hit(&mut result, hit, options)? {
                        return Ok(result);
                    }
                }
            }
        }
        return Ok(result);
    }
    if options.verified {
        index(base, false, options.cache_root.as_deref())?;
    }
    let path = cache_path(base, options.cache_root.as_deref(), false)?;
    let _lock = cache_lock(&path, false)?;
    no_symlinks(&path)?;
    ensure!(
        path.is_file(),
        "index_missing: create the selected base index first"
    );
    let connection = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("index_unreadable: use an explicit index rebuild")?;
    configure(&connection)?;
    let metadata =
        metadata(&connection).context("index_unreadable: use an explicit index rebuild")?;
    validate_index_identity(base, &metadata)?;
    let signature = metadata
        .get("source_signature")
        .context("index_unreadable: missing source signature")?;
    let revision = metadata
        .get("manifest_revision")
        .context("index_unreadable: missing manifest revision")?;
    ensure!(
        revision == &base.revision()?,
        "index_stale: the selected base has a newer accepted manifest; use --verified or index update"
    );
    let mut result = envelope(base, query, "ranked", signature, revision);
    if options.verified {
        result["consistency"] = json!("verified_source_snapshot");
    }
    result["indexed_at"] = json!(metadata.get("indexed_at"));
    result["manifest_matches_open_base"] = json!(true);
    ensure!(
        serde_json::to_vec(&result)?.len() <= options.max_bytes,
        "output_budget_too_small: cannot fit the search response metadata"
    );
    // Bind SQL parameters and quote every ordinary term. Operators in user text
    // do not become a query language and cannot become SQL.
    let expression = query
        .split_whitespace()
        .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ");
    let mut statement = connection.prepare(
        "SELECT docs.metadata,snippet(search_text,2,'','',' … ',32),bm25(search_text,8.0,4.0,1.0)
         FROM search_text JOIN docs ON docs.rowid=search_text.rowid
         WHERE search_text MATCH ?1 AND (?2 IS NULL OR EXISTS(SELECT 1 FROM json_each(docs.concepts) WHERE value=?2))
         ORDER BY bm25(search_text,8.0,4.0,1.0),docs.key LIMIT ?3"
    )?;
    let cap = i64::try_from(options.limit.saturating_add(1)).unwrap_or(i64::MAX);
    let mut rows = statement.query(params![expression, options.concept, cap])?;
    while let Some(row) = rows.next()? {
        let mut hit: Value = serde_json::from_str(&row.get::<_, String>(0)?)?;
        hit["snippet"] = json!(clipped(&row.get::<_, String>(1)?, 384));
        hit["score"] = json!(row.get::<_, f64>(2)?);
        if !add_hit(&mut result, hit, options)? {
            break;
        }
    }
    Ok(result)
}

/// Pack existing hits with provenance and qualifiers; never generate an answer.
pub fn context(base: &Base, query: &str, options: &SearchOptions) -> Result<Value> {
    let mut result = search(base, query, options)?;
    result["operation"] = json!("context");
    if serde_json::to_vec(&result)?.len() > options.max_bytes {
        bail!("output_budget_too_small: cannot fit the context response");
    }
    Ok(result)
}
