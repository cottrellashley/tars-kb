//! Portable temporal memory. Canonical assertion files retain every accepted
//! revision; the disposable index is never the source of historical answers.
use crate::{
    Result,
    library::{Base, identifier, now, today},
    storage::{Write, hash},
    temporal,
};
use anyhow::{anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const PROFILE: &str = "tars-memory/1.0";
const STATE: &str = "data/memory/profile.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FactFilter {
    pub valid_at: Option<String>,
    pub known_at: Option<String>,
    pub known_revision: Option<u64>,
    pub view: Option<String>,
}

fn strings<'a>(value: &'a Value, key: &str) -> Result<Vec<&'a str>> {
    value[key]
        .as_array()
        .ok_or_else(|| anyhow!("invalid_record: {key} must be an array"))?
        .iter()
        .map(|v| {
            v.as_str()
                .ok_or_else(|| anyhow!("invalid_record: {key} entries must be strings"))
        })
        .collect()
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow!("invalid_record: nonempty {key} required"))
}
fn concept_path(id: &str) -> String {
    format!("data/memory/concepts/{id}.json")
}
fn assertion_path(id: &str) -> String {
    format!("data/memory/assertions/{id}.json")
}
fn state(base: &Base) -> Result<Value> {
    ensure!(base.declared(STATE), "undeclared_asset: memory profile");
    let value = base.read_json(STATE).map_err(|e| {
        anyhow!("memory_unavailable: initialize the supported memory profile first: {e}")
    })?;
    ensure!(
        value["schema_version"] == "1.0" && value["profile"] == PROFILE,
        "unsupported_profile: expected {PROFILE}"
    );
    supported_features(&value)?;
    ensure!(
        value["sequence"].is_u64(),
        "invalid_profile: missing sequence"
    );
    for key in ["concepts", "assertions"] {
        let mut seen = BTreeSet::new();
        for id in strings(&value, key)? {
            identifier(id)?;
            ensure!(seen.insert(id), "invalid_profile: duplicate inventory ID");
        }
    }
    for key in ["created_at", "history_started_at", "last_recorded_at"] {
        temporal::timestamp(field(&value, key)?)?;
    }
    ensure!(
        value["transactions"].is_array() && value["idempotency"].is_object(),
        "invalid_profile: transaction/idempotency inventory required"
    );
    ensure!(
        value["transactions"].as_array().unwrap().len() as u64
            == value["sequence"].as_u64().unwrap(),
        "invalid_profile: transaction sequence gap"
    );
    Ok(value)
}
fn supported_features(value: &Value) -> Result<()> {
    if let Some(features) = value.get("required_features") {
        ensure!(
            features.as_array().is_some_and(|v| v.is_empty()),
            "unsupported_feature: unknown required memory features"
        );
    }
    Ok(())
}

fn declare(manifest: &mut Value, id: &str, path: &str) -> Result<()> {
    let assets = manifest["assets"]
        .as_array_mut()
        .ok_or_else(|| anyhow!("invalid_manifest: assets must be an array"))?;
    if let Some(existing) = assets.iter().find(|v| v["path"] == path) {
        ensure!(
            existing["id"] == id,
            "asset_conflict: path is already declared with another identity"
        );
        return Ok(());
    }
    ensure!(
        !assets.iter().any(|v| v["id"] == id),
        "asset_conflict: reserved memory asset ID already exists"
    );
    assets.push(json!({"id":id,"path":path,"media_type":"application/json"}));
    Ok(())
}

pub fn memory_init(base: &mut Base) -> Result<Value> {
    if base.root.join(STATE).exists() {
        let s = state(base)?;
        return Ok(json!({"initialized":false,"profile":PROFILE,"sequence":s["sequence"]}));
    }
    let at = now();
    let mut manifest = base.manifest.clone();
    declare(&mut manifest, "memory-profile", STATE)?;
    manifest["updated_at"] = json!(today());
    let profile = json!({"schema_version":"1.0","profile":PROFILE,"sequence":0,"created_at":at,"history_started_at":at,"last_recorded_at":at,"concepts":[],"assertions":[],"transactions":[],"idempotency":{}});
    let writes = vec![
        base.write(STATE, &profile)?,
        base.write("knowledge-base.json", &manifest)?,
    ];
    let commit = base.commit(writes)?;
    Ok(json!({"initialized":true,"profile":PROFILE,"sequence":0,"commit":commit}))
}

struct Transaction<'a> {
    base: &'a mut Base,
    state: Value,
    manifest: Value,
    files: BTreeMap<String, Value>,
    sequence: u64,
    at: String,
    changed_manifest: bool,
}
impl<'a> Transaction<'a> {
    fn new(base: &'a mut Base, at: &str) -> Result<Self> {
        temporal::timestamp(at)?;
        let state = state(base)?;
        let sequence = state["sequence"]
            .as_u64()
            .unwrap()
            .checked_add(1)
            .ok_or_else(|| anyhow!("sequence_exhausted"))?;
        Ok(Self {
            manifest: base.manifest.clone(),
            base,
            state,
            files: BTreeMap::new(),
            sequence,
            at: at.into(),
            changed_manifest: false,
        })
    }
    fn read(&self, path: &str) -> Result<Value> {
        if let Some(v) = self.files.get(path) {
            Ok(v.clone())
        } else {
            self.base.read_json(path)
        }
    }
    fn known(&self, key: &str, id: &str) -> Result<bool> {
        Ok(strings(&self.state, key)?.contains(&id))
    }
    fn add(
        &mut self,
        key: &str,
        id: &str,
        path: String,
        asset_id: String,
        value: Value,
    ) -> Result<()> {
        ensure!(!self.known(key, id)?, "already_exists: {id}");
        ensure!(
            !self.base.root.join(&path).exists(),
            "already_exists: unregistered file exists"
        );
        self.state[key].as_array_mut().unwrap().push(json!(id));
        declare(&mut self.manifest, &asset_id, &path)?;
        self.changed_manifest = true;
        self.files.insert(path, value);
        Ok(())
    }
    fn validate_content(&self, content: &mut Value) -> Result<Vec<Value>> {
        ensure!(
            content.is_object(),
            "invalid_record: assertion must be an object"
        );
        supported_features(content)?;
        for reserved in [
            "recorded_at",
            "sequence",
            "revision",
            "source_snapshots",
            "retracted",
        ] {
            ensure!(
                content.get(reserved).is_none(),
                "reserved_field: {reserved} is generated by the writer"
            );
        }
        identifier(field(content, "id")?)?;
        field(content, "text")?;
        ensure!(
            matches!(
                field(content, "basis")?,
                "definition" | "observed" | "reported" | "decision" | "requirement" | "inferred"
            ),
            "invalid_basis"
        );
        let concepts = strings(content, "concepts")?;
        ensure!(
            !concepts.is_empty(),
            "invalid_record: at least one concept is required"
        );
        let mut unique = BTreeSet::new();
        for id in concepts {
            identifier(id)?;
            ensure!(unique.insert(id), "duplicate_concept");
            ensure!(self.known("concepts", id)?, "unknown_concept: {id}");
        }
        temporal::normalize(
            content
                .get_mut("temporal")
                .ok_or_else(|| anyhow!("invalid_time: temporal is required"))?,
        )?;
        if content["temporal"]["kind"] == "observed"
            && let temporal::Point::Instant(at) = temporal::point(&content["temporal"]["at"])?
        {
            ensure!(
                at <= temporal::timestamp(&self.at)?,
                "future_observation: observation cannot occur after local acceptance"
            );
        }
        if let Some(cardinality) = content.get("cardinality") {
            ensure!(
                cardinality == "single" || cardinality == "multi",
                "invalid_cardinality"
            );
        }
        if let Some(review) = content.get("review_after") {
            temporal::timestamp(
                review
                    .as_str()
                    .ok_or_else(|| anyhow!("invalid_review_after"))?,
            )?;
        }
        let sources = strings(content, "sources")?;
        ensure!(
            !sources.is_empty(),
            "invalid_record: at least one existing source is required"
        );
        let available = self.manifest["sources"]
            .as_array()
            .ok_or_else(|| anyhow!("invalid_manifest: sources"))?;
        let mut snapshots = Vec::new();
        let mut unique = BTreeSet::new();
        for id in sources {
            ensure!(unique.insert(id), "duplicate_source");
            snapshots.push(
                available
                    .iter()
                    .find(|source| source["id"] == id)
                    .ok_or_else(|| anyhow!("unknown_source: {id}"))?
                    .clone(),
            );
        }
        if let Some(relations) = content.get("relations") {
            for relation in relations
                .as_array()
                .ok_or_else(|| anyhow!("invalid_relations"))?
            {
                let target = field(relation, "target")?;
                identifier(target)?;
                ensure!(
                    target != content["id"].as_str().unwrap(),
                    "relation_cycle: self target"
                );
                ensure!(
                    self.known("assertions", target)?,
                    "unknown_assertion: {target}"
                );
                ensure!(
                    matches!(
                        field(relation, "kind")?,
                        "supports" | "disputes" | "related" | "reconfirms"
                    ),
                    "invalid_relation: lifecycle relationships are writer-owned"
                );
                if relation["kind"] == "reconfirms" {
                    ensure!(
                        !self.lineage_reaches(target, content["id"].as_str().unwrap())?,
                        "relation_cycle: reconfirmation lineage must be acyclic"
                    );
                }
            }
        }
        Ok(snapshots)
    }
    fn lineage_reaches(&self, start: &str, target: &str) -> Result<bool> {
        let mut pending = vec![start.to_string()];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if id == target {
                return Ok(true);
            }
            if !seen.insert(id.clone()) {
                continue;
            }
            identifier(&id)?;
            ensure!(self.known("assertions", &id)?, "unknown_assertion: {id}");
            let record = self.read(&assertion_path(&id))?;
            let content = &record["revisions"]
                .as_array()
                .and_then(|r| r.last())
                .ok_or_else(|| anyhow!("invalid_assertion_history"))?["content"];
            if let Some(relations) = content["relations"].as_array() {
                for relation in relations {
                    if relation["kind"] == "reconfirms" {
                        pending.push(field(relation, "target")?.into());
                    }
                }
            }
        }
        Ok(false)
    }
    fn revision(
        &self,
        content: Value,
        snapshots: Vec<Value>,
        mode: &str,
        reason: Option<&str>,
        retracted: bool,
    ) -> Result<Value> {
        let mut revision = json!({"sequence":self.sequence,"recorded_at":self.at,"mode":mode,"content":content,"source_snapshots":snapshots,"retracted":retracted});
        if let Some(reason) = reason {
            revision["reason"] = json!(reason);
        }
        revision["revision"] = json!(hash(&serde_json::to_vec(&revision)?));
        Ok(revision)
    }
    fn remember(&mut self, mut content: Value, mode: &str) -> Result<Value> {
        let snapshots = self.validate_content(&mut content)?;
        let id = field(&content, "id")?.to_string();
        let revision = self.revision(content, snapshots, mode, None, false)?;
        let record = json!({"schema_version":"1.0","profile":PROFILE,"id":id,"revisions":[revision.clone()]});
        self.add(
            "assertions",
            &id,
            assertion_path(&id),
            format!("memory-assertion-{}", &hash(id.as_bytes())[..32]),
            record,
        )?;
        Ok(
            json!({"id":id,"revision":revision["revision"],"sequence":self.sequence,"recorded_at":self.at}),
        )
    }
    fn concept(&mut self, id: &str, mut input: Value) -> Result<Value> {
        identifier(id)?;
        ensure!(input.is_object(), "invalid_concept");
        supported_features(&input)?;
        field(&input, "title")?;
        field(&input, "summary")?;
        for key in ["aliases", "parents", "related", "pages"] {
            if input.get(key).is_none() {
                input[key] = json!([]);
            }
            let mut seen = BTreeSet::new();
            for value in strings(&input, key)? {
                ensure!(
                    !value.trim().is_empty() && seen.insert(value),
                    "invalid_concept: empty or duplicate {key}"
                );
            }
        }
        for key in ["parents", "related"] {
            for target in strings(&input, key)? {
                identifier(target)?;
                ensure!(target != id, "concept_cycle: self target");
                ensure!(self.known("concepts", target)?, "unknown_concept: {target}");
            }
        }
        for page in strings(&input, "pages")? {
            ensure!(
                self.manifest["pages"]
                    .as_array()
                    .is_some_and(|pages| pages.iter().any(|p| p["id"] == page)),
                "unknown_page: {page}"
            );
        }
        input["id"] = json!(id);
        input["schema_version"] = json!("1.0");
        input["profile"] = json!(PROFILE);
        input["created_at"] = json!(self.at);
        input["sequence"] = json!(self.sequence);
        let mut for_hash = input.clone();
        for_hash.as_object_mut().unwrap().remove("revision");
        input["revision"] = json!(hash(&serde_json::to_vec(&for_hash)?));
        self.add(
            "concepts",
            id,
            concept_path(id),
            format!("memory-concept-{}", &hash(id.as_bytes())[..32]),
            input,
        )?;
        Ok(json!({"id":id,"sequence":self.sequence}))
    }
    fn revise(
        &mut self,
        id: &str,
        mode: &str,
        input: Value,
        expected: Option<&str>,
    ) -> Result<Value> {
        identifier(id)?;
        ensure!(self.known("assertions", id)?, "unknown_assertion: {id}");
        let path = assertion_path(id);
        let mut record = if self.files.contains_key(&path) {
            self.read(&path)?
        } else {
            load_record(self.base, id, self.state["sequence"].as_u64().unwrap())?
        };
        let previous = record["revisions"]
            .as_array()
            .and_then(|v| v.last())
            .ok_or_else(|| anyhow!("invalid_assertion_history"))?
            .clone();
        ensure!(
            expected.is_some(),
            "expected_revision_required: revise requires the revision previously read"
        );
        ensure!(
            previous["revision"].as_str() == expected,
            "revision_conflict: assertion changed"
        );
        ensure!(
            !previous["retracted"].as_bool().unwrap_or(false),
            "assertion_retracted: create a new assertion to reassert"
        );
        let old = previous["content"].clone();
        match mode {
            "correct" => {
                let reason = field(&input, "reason")?;
                let changes = input.get("content").unwrap_or(&input);
                ensure!(changes.is_object(), "invalid_correction");
                let mut content = old;
                for (key, value) in changes.as_object().unwrap() {
                    if key != "reason" && key != "remove_fields" {
                        content[key] = value.clone();
                    }
                }
                if input.get("remove_fields").is_some() {
                    for key in strings(&input, "remove_fields")? {
                        ensure!(
                            !["id", "concepts", "text", "basis", "temporal", "sources"]
                                .contains(&key),
                            "invalid_correction: cannot remove required {key}"
                        );
                        content.as_object_mut().unwrap().remove(key);
                    }
                }
                ensure!(
                    content["id"] == id,
                    "identity_change: correction preserves assertion ID"
                );
                let snapshots = self.validate_content(&mut content)?;
                let revision = self.revision(content, snapshots, mode, Some(reason), false)?;
                record["revisions"]
                    .as_array_mut()
                    .unwrap()
                    .push(revision.clone());
                self.files.insert(path, record);
                Ok(json!({"id":id,"revision":revision["revision"],"sequence":self.sequence}))
            }
            "retract" => {
                let reason = field(&input, "reason")?;
                let revision = self.revision(
                    old,
                    previous["source_snapshots"].as_array().unwrap().clone(),
                    mode,
                    Some(reason),
                    true,
                )?;
                record["revisions"]
                    .as_array_mut()
                    .unwrap()
                    .push(revision.clone());
                self.files.insert(path, record);
                Ok(
                    json!({"id":id,"revision":revision["revision"],"sequence":self.sequence,"retracted":true}),
                )
            }
            "change" => {
                ensure!(
                    old["temporal"]["kind"] == "interval",
                    "invalid_change: real changes require an existing asserted interval"
                );
                let mut effective = input
                    .get("effective_at")
                    .cloned()
                    .ok_or_else(|| anyhow!("effective_time_required"))?;
                temporal::normalize_point(&mut effective)?;
                let ep = temporal::point(&effective)?;
                ensure!(
                    temporal::compare(&ep, &temporal::point(&old["temporal"]["from"])?)
                        == Some(std::cmp::Ordering::Greater),
                    "invalid_change: effective time must follow the old start"
                );
                ensure!(
                    temporal::applicable(&old["temporal"], &ep)? == Some(true),
                    "invalid_change: effective time is outside the old interval"
                );
                let mut replacement = input
                    .get("replacement")
                    .cloned()
                    .ok_or_else(|| anyhow!("replacement_required"))?;
                ensure!(
                    replacement["id"] != id,
                    "identity_change: changed state needs a new assertion ID"
                );
                temporal::normalize(
                    replacement
                        .get_mut("temporal")
                        .ok_or_else(|| anyhow!("invalid_time"))?,
                )?;
                ensure!(
                    replacement["temporal"]["kind"] == "interval"
                        && temporal::compare(
                            &temporal::point(&replacement["temporal"]["from"])?,
                            &ep
                        ) == Some(std::cmp::Ordering::Equal),
                    "invalid_change: replacement interval must start at effective_at"
                );
                for key in ["subject", "predicate", "scope"] {
                    ensure!(
                        old.get(key) == replacement.get(key),
                        "invalid_change: replacement must preserve {key}"
                    );
                }
                let mut closed = old;
                closed["temporal"]["until"] = effective;
                let snapshots = self.validate_content(&mut closed)?;
                let revision =
                    self.revision(closed, snapshots, mode, input["reason"].as_str(), false)?;
                record["revisions"].as_array_mut().unwrap().push(revision);
                self.files.insert(path, record);
                let result = self.remember(replacement, mode)?;
                Ok(json!({"closed_id":id,"replacement":result,"sequence":self.sequence}))
            }
            "reconfirm" => {
                let mut replacement = input
                    .get("replacement")
                    .or_else(|| input.get("content"))
                    .cloned()
                    .unwrap_or(input.clone());
                ensure!(
                    replacement["id"] != id,
                    "identity_change: reconfirmation needs a new observation ID"
                );
                ensure!(
                    replacement["temporal"]["kind"] == "observed",
                    "invalid_reconfirmation: a new point observation is required"
                );
                if old["temporal"]["kind"] == "observed" {
                    ensure!(
                        temporal::compare(
                            &temporal::point(&replacement["temporal"]["at"])?,
                            &temporal::point(&old["temporal"]["at"])?
                        ) == Some(std::cmp::Ordering::Greater),
                        "invalid_reconfirmation: observation time must advance with comparable precision"
                    );
                }
                for key in ["subject", "predicate", "scope", "value"] {
                    ensure!(
                        old.get(key) == replacement.get(key),
                        "invalid_reconfirmation: {key} differs; record new evidence instead"
                    );
                }
                if replacement.get("relations").is_none() {
                    replacement["relations"] = json!([]);
                }
                replacement["relations"]
                    .as_array_mut()
                    .ok_or_else(|| anyhow!("invalid_relations"))?
                    .push(json!({"kind":"reconfirms","target":id}));
                self.remember(replacement, mode)
            }
            _ => bail!("invalid_revision_mode: use correct, change, reconfirm or retract"),
        }
    }
    fn finish(
        mut self,
        results: Vec<Value>,
        idempotency: Option<&str>,
        digest: &str,
    ) -> Result<Value> {
        let prior = temporal::timestamp(field(&self.state, "last_recorded_at")?)?;
        let current = temporal::timestamp(&self.at)?;
        self.state["sequence"] = json!(self.sequence);
        self.state["last_recorded_at"] = json!(self.at);
        self.state["transactions"].as_array_mut().ok_or_else(||anyhow!("invalid_profile"))?.push(json!({"sequence":self.sequence,"recorded_at":self.at,"clock_regressed":current<prior,"previous_recorded_at":prior.to_string(),"operation_digest":digest,"results":results.clone()}));
        let receipt = json!({"sequence":self.sequence,"recorded_at":self.at,"results":results,"committed":true});
        if let Some(key) = idempotency {
            self.state["idempotency"][key] = json!({"digest":digest,"receipt":receipt});
        }
        self.files.insert(STATE.into(), self.state);
        if self.changed_manifest {
            self.manifest["updated_at"] = json!(today());
            self.files
                .insert("knowledge-base.json".into(), self.manifest);
        }
        let writes: Result<Vec<Write>> = self
            .files
            .iter()
            .map(|(path, value)| self.base.write(path, value))
            .collect();
        self.base.commit(writes?)?;
        Ok(receipt)
    }
}

fn apply_at(
    base: &mut Base,
    operations: Vec<Value>,
    idempotency: Option<&str>,
    at: &str,
) -> Result<Value> {
    ensure!(
        !operations.is_empty() && operations.len() <= 1000,
        "invalid_batch: require 1..1000 operations"
    );
    let digest = hash(&serde_json::to_vec(&operations)?);
    if let Some(key) = idempotency {
        ensure!(
            !key.trim().is_empty() && key.len() <= 200,
            "invalid_idempotency_key"
        );
        let state = state(base)?;
        if let Some(prior) = state["idempotency"].get(key) {
            ensure!(
                prior["digest"] == digest,
                "idempotency_conflict: key was used for another payload"
            );
            return Ok(prior["receipt"].clone());
        }
    }
    let mut tx = Transaction::new(base, at)?;
    let mut results = Vec::new();
    for operation in operations {
        results.push(match field(&operation, "op")? {
            "remember" => tx.remember(operation["input"].clone(), "assert")?,
            "concept-init" => tx.concept(field(&operation, "id")?, operation["input"].clone())?,
            "revise" => tx.revise(
                field(&operation, "id")?,
                field(&operation, "mode")?,
                operation["input"].clone(),
                operation["expect_revision"].as_str(),
            )?,
            _ => bail!("invalid_operation: unsupported memory batch operation"),
        });
    }
    tx.finish(results, idempotency, &digest)
}

pub fn batch(base: &mut Base, operations: Vec<Value>, idempotency: Option<&str>) -> Result<Value> {
    apply_at(base, operations, idempotency, &now())
}
pub fn concept_init(base: &mut Base, id: &str, input: Value) -> Result<Value> {
    batch(
        base,
        vec![json!({"op":"concept-init","id":id,"input":input})],
        None,
    )
}
pub fn remember(base: &mut Base, input: Value, idempotency: Option<&str>) -> Result<Value> {
    batch(
        base,
        vec![json!({"op":"remember","input":input})],
        idempotency,
    )
}
pub fn revise(
    base: &mut Base,
    id: &str,
    mode: &str,
    input: Value,
    expected_revision: Option<&str>,
    idempotency: Option<&str>,
) -> Result<Value> {
    batch(
        base,
        vec![
            json!({"op":"revise","id":id,"mode":mode,"input":input,"expect_revision":expected_revision}),
        ],
        idempotency,
    )
}

pub fn concept_get(base: &Base, id: &str) -> Result<Value> {
    identifier(id)?;
    ensure!(
        strings(&state(base)?, "concepts")?.contains(&id),
        "unknown_concept: {id}"
    );
    ensure!(
        base.declared(&concept_path(id)),
        "undeclared_asset: concept {id}"
    );
    let value = base.read_json(&concept_path(id))?;
    ensure!(
        value["schema_version"] == "1.0" && value["profile"] == PROFILE && value["id"] == id,
        "invalid_concept: identity or profile mismatch"
    );
    supported_features(&value)?;
    Ok(value)
}
pub fn concept_list(base: &Base) -> Result<Value> {
    let state = state(base)?;
    let items: Result<Vec<Value>> = strings(&state, "concepts")?
        .iter()
        .map(|id| base.read_json(&concept_path(id)))
        .collect();
    Ok(json!({"items":items?,"sequence":state["sequence"],"consistency":"accepted_revision"}))
}
pub fn history(base: &Base, id: &str) -> Result<Value> {
    identifier(id)?;
    let state = state(base)?;
    ensure!(
        strings(&state, "assertions")?.contains(&id),
        "unknown_assertion: {id}"
    );
    load_record(base, id, state["sequence"].as_u64().unwrap())
}
fn load_record(base: &Base, id: &str, max_sequence: u64) -> Result<Value> {
    ensure!(
        base.declared(&assertion_path(id)),
        "undeclared_asset: assertion {id}"
    );
    let value = base.read_json(&assertion_path(id))?;
    ensure!(
        value["schema_version"] == "1.0" && value["profile"] == PROFILE && value["id"] == id,
        "invalid_assertion: identity or profile mismatch"
    );
    supported_features(&value)?;
    let revisions = value["revisions"]
        .as_array()
        .ok_or_else(|| anyhow!("invalid_assertion_history"))?;
    ensure!(
        !revisions.is_empty(),
        "invalid_assertion_history: empty revisions"
    );
    let mut previous = 0;
    let mut seen = BTreeSet::new();
    for revision in revisions {
        supported_features(revision)?;
        let sequence = revision["sequence"]
            .as_u64()
            .ok_or_else(|| anyhow!("invalid_assertion_history: sequence"))?;
        ensure!(
            sequence >= previous && sequence > 0 && sequence <= max_sequence,
            "invalid_assertion_history: sequence ordering"
        );
        previous = sequence;
        temporal::timestamp(field(revision, "recorded_at")?)?;
        ensure!(
            matches!(
                field(revision, "mode")?,
                "assert" | "correct" | "change" | "reconfirm" | "retract"
            ),
            "invalid_assertion_history: unknown mode"
        );
        if matches!(revision["mode"].as_str(), Some("correct" | "retract")) {
            field(revision, "reason")?;
        }
        ensure!(
            revision["retracted"].as_bool() == Some(revision["mode"] == "retract"),
            "invalid_assertion_history: retraction mode mismatch"
        );
        ensure!(
            revision["content"]["id"] == id
                && revision["retracted"].is_boolean()
                && revision["source_snapshots"].is_array(),
            "invalid_assertion_history: content"
        );
        let content = &revision["content"];
        supported_features(content)?;
        field(content, "text")?;
        ensure!(
            matches!(
                field(content, "basis")?,
                "definition" | "observed" | "reported" | "decision" | "requirement" | "inferred"
            ),
            "invalid_basis"
        );
        ensure!(
            !strings(content, "concepts")?.is_empty() && !strings(content, "sources")?.is_empty(),
            "invalid_assertion_history: missing concepts or sources"
        );
        let mut timing = content["temporal"].clone();
        temporal::normalize(&mut timing)?;
        if timing["kind"] == "observed"
            && let temporal::Point::Instant(at) = temporal::point(&timing["at"])?
        {
            ensure!(
                at <= temporal::timestamp(field(revision, "recorded_at")?)?,
                "future_observation: invalid accepted observation"
            );
        }
        if let Some(review) = content.get("review_after") {
            temporal::timestamp(
                review
                    .as_str()
                    .ok_or_else(|| anyhow!("invalid_review_after"))?,
            )?;
        }
        if let Some(cardinality) = content.get("cardinality") {
            ensure!(
                cardinality == "single" || cardinality == "multi",
                "invalid_cardinality"
            );
        }
        let stored = field(revision, "revision")?;
        ensure!(
            seen.insert(stored),
            "invalid_assertion_history: duplicate revision"
        );
        let mut bytes = revision.clone();
        bytes
            .as_object_mut()
            .ok_or_else(|| anyhow!("invalid_assertion_history"))?
            .remove("revision");
        ensure!(
            stored == hash(&serde_json::to_vec(&bytes)?),
            "history_integrity: accepted revision bytes changed"
        );
    }
    Ok(value)
}
pub fn fact(base: &Base, id: &str) -> Result<Value> {
    let record = history(base, id)?;
    let mut result = record["revisions"]
        .as_array()
        .and_then(|v| v.last())
        .cloned()
        .ok_or_else(|| anyhow!("invalid_assertion_history"))?;
    result["id"] = json!(id);
    result["temporal_status"] = json!(temporal::label(&result["content"]["temporal"]));
    Ok(result)
}

/// Full, explicit profile validation. It does not alter files or fetch sources.
pub fn validate(base: &Base) -> Result<Value> {
    let state = state(base)?;
    ensure!(base.declared(STATE), "undeclared_asset: memory profile");
    let sequence = state["sequence"].as_u64().unwrap();
    let transactions = state["transactions"].as_array().unwrap();
    ensure!(
        transactions.len() as u64 == sequence,
        "invalid_profile: transaction sequence gap"
    );
    let mut timestamps = BTreeMap::new();
    let mut previous = temporal::timestamp(field(&state, "created_at")?)?;
    for (index, transaction) in transactions.iter().enumerate() {
        ensure!(
            transaction["sequence"].as_u64() == Some(index as u64 + 1),
            "invalid_profile: transaction sequence ordering"
        );
        let digest = field(transaction, "operation_digest")?;
        ensure!(
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && transaction["results"].is_array(),
            "invalid_profile: missing operation digest/results"
        );
        let at = temporal::timestamp(field(transaction, "recorded_at")?)?;
        let prior = temporal::timestamp(field(transaction, "previous_recorded_at")?)?;
        ensure!(
            prior == previous && transaction["clock_regressed"].as_bool() == Some(at < previous),
            "invalid_profile: clock metadata mismatch"
        );
        previous = at;
        timestamps.insert(index as u64 + 1, at);
    }
    ensure!(
        temporal::timestamp(field(&state, "last_recorded_at")?)? == previous,
        "invalid_profile: last recorded time mismatch"
    );
    for (key, value) in state["idempotency"].as_object().unwrap() {
        ensure!(
            !key.trim().is_empty() && key.len() <= 200,
            "invalid_profile: idempotency key"
        );
        let receipt = &value["receipt"];
        let seq = receipt["sequence"]
            .as_u64()
            .ok_or_else(|| anyhow!("invalid_profile: idempotency sequence"))?;
        ensure!(
            seq > 0 && seq <= sequence,
            "invalid_profile: idempotency sequence outside history"
        );
        let transaction = &transactions[(seq - 1) as usize];
        ensure!(
            value["digest"] == transaction["operation_digest"]
                && receipt["results"] == transaction["results"]
                && receipt["recorded_at"] == transaction["recorded_at"]
                && receipt["committed"] == true,
            "invalid_profile: idempotency receipt differs from its transaction"
        );
    }
    let concepts: BTreeSet<&str> = strings(&state, "concepts")?.into_iter().collect();
    let assertions: BTreeSet<&str> = strings(&state, "assertions")?.into_iter().collect();
    for id in &concepts {
        ensure!(
            base.declared(&concept_path(id)),
            "undeclared_asset: concept {id}"
        );
        let value = concept_get(base, id)?;
        supported_features(&value)?;
        field(&value, "title")?;
        field(&value, "summary")?;
        for key in ["aliases", "parents", "related", "pages"] {
            let values = strings(&value, key)?;
            let mut unique = BTreeSet::new();
            for value in values {
                ensure!(
                    !value.trim().is_empty() && unique.insert(value),
                    "invalid_concept: empty or duplicate {key}"
                );
            }
        }
        let seq = value["sequence"]
            .as_u64()
            .ok_or_else(|| anyhow!("invalid_concept: sequence"))?;
        ensure!(
            seq > 0 && seq <= sequence,
            "invalid_concept: sequence outside accepted history"
        );
        ensure!(
            timestamps.get(&seq) == Some(&temporal::timestamp(field(&value, "created_at")?)?),
            "invalid_concept: acceptance clock mismatch"
        );
        for key in ["parents", "related"] {
            for target in strings(&value, key)? {
                ensure!(
                    concepts.contains(target) && target != *id,
                    "invalid_concept: missing/self relationship"
                );
            }
        }
        for page in strings(&value, "pages")? {
            ensure!(
                base.manifest["pages"]
                    .as_array()
                    .is_some_and(|pages| pages.iter().any(|p| p["id"] == page)),
                "invalid_concept: missing page"
            );
        }
        let revision = field(&value, "revision")?;
        let mut bytes = value.clone();
        bytes.as_object_mut().unwrap().remove("revision");
        ensure!(
            revision == hash(&serde_json::to_vec(&bytes)?),
            "history_integrity: concept bytes changed"
        );
    }
    let mut revision_count = 0;
    for id in &assertions {
        ensure!(
            base.declared(&assertion_path(id)),
            "undeclared_asset: assertion {id}"
        );
        let record = load_record(base, id, sequence)?;
        supported_features(&record)?;
        for revision in record["revisions"].as_array().unwrap() {
            revision_count += 1;
            let seq = revision["sequence"].as_u64().unwrap();
            ensure!(
                timestamps.get(&seq)
                    == Some(&temporal::timestamp(field(revision, "recorded_at")?)?),
                "invalid_assertion_history: transaction clock mismatch"
            );
            let content = &revision["content"];
            supported_features(content)?;
            field(content, "text")?;
            for concept in strings(content, "concepts")? {
                ensure!(concepts.contains(concept), "unknown_concept: {concept}");
            }
            let sources = strings(content, "sources")?;
            ensure!(!sources.is_empty(), "invalid_assertion: missing source");
            let snapshots = revision["source_snapshots"].as_array().unwrap();
            ensure!(
                sources.len() == snapshots.len(),
                "invalid_assertion_history: incomplete source snapshot"
            );
            for source in sources {
                let snapshot = snapshots
                    .iter()
                    .find(|v| v["id"] == source)
                    .ok_or_else(|| anyhow!("invalid_assertion_history: missing source snapshot"))?;
                for key in ["title", "uri", "type", "checked_at"] {
                    field(snapshot, key)?;
                }
                ensure!(
                    matches!(
                        field(snapshot, "type")?,
                        "workspace" | "repository" | "official" | "conversation" | "observation"
                    ),
                    "invalid_source_snapshot: type"
                );
                let checked = field(snapshot, "checked_at")?;
                ensure!(checked.len() == 10, "invalid_source_snapshot: date");
                checked.parse::<jiff::civil::Date>()?;
            }
            if let Some(relations) = content.get("relations") {
                for relation in relations
                    .as_array()
                    .ok_or_else(|| anyhow!("invalid_relations"))?
                {
                    ensure!(
                        assertions.contains(field(relation, "target")?),
                        "unknown_assertion: relation target"
                    );
                }
            }
        }
    }
    for path in crate::library::files_recursive(&base.root)? {
        if path.starts_with("data/memory/") {
            ensure!(base.declared(&path), "undeclared_asset: {path}");
        }
    }
    Ok(
        json!({"profile":PROFILE,"sequence":sequence,"concepts":concepts.len(),"assertions":assertions.len(),"revisions":revision_count,"valid":true}),
    )
}

fn cutoff(state: &Value, filter: &FactFilter) -> Result<u64> {
    ensure!(
        !(filter.known_at.is_some() && filter.known_revision.is_some()),
        "invalid_filter: choose known_at or known_revision"
    );
    let current = state["sequence"].as_u64().unwrap();
    if let Some(seq) = filter.known_revision {
        ensure!(seq <= current, "history_unavailable: future revision");
        return Ok(seq);
    }
    if let Some(at) = &filter.known_at {
        let at = temporal::timestamp(at)?;
        for transaction in state["transactions"]
            .as_array()
            .ok_or_else(|| anyhow!("invalid_profile"))?
        {
            if transaction["clock_regressed"] == true {
                let lower = temporal::timestamp(field(transaction, "recorded_at")?)?;
                let upper = temporal::timestamp(field(transaction, "previous_recorded_at")?)?;
                ensure!(
                    at < lower || at > upper,
                    "clock_regression: known_at is ambiguous; use known_revision"
                );
            }
        }
        ensure!(
            at >= temporal::timestamp(field(state, "history_started_at")?)?,
            "history_unavailable: requested time precedes portable history"
        );
        let mut seq = 0;
        for transaction in state["transactions"].as_array().unwrap() {
            if temporal::timestamp(field(transaction, "recorded_at")?)? <= at {
                seq = seq.max(
                    transaction["sequence"]
                        .as_u64()
                        .ok_or_else(|| anyhow!("invalid_profile"))?,
                );
            }
        }
        return Ok(seq);
    }
    Ok(current)
}

pub fn facts(base: &Base, concept: Option<&str>, filter: &FactFilter) -> Result<Value> {
    let state = state(base)?;
    let seq = cutoff(&state, filter)?;
    if let Some(concept) = concept {
        ensure!(
            strings(&state, "concepts")?.contains(&concept),
            "unknown_concept: {concept}"
        );
    }
    let view = filter
        .view
        .as_deref()
        .unwrap_or(if filter.valid_at.is_some() {
            "applicable"
        } else {
            "all"
        });
    ensure!(
        matches!(view, "all" | "applicable" | "last-observed"),
        "invalid_view"
    );
    let query = filter
        .valid_at
        .as_deref()
        .map(temporal::query_point)
        .transpose()?;
    ensure!(
        view != "applicable" || query.is_some(),
        "valid_time_required: applicable view requires valid_at"
    );
    ensure!(
        view != "all" || query.is_none(),
        "invalid_filter: all view has no valid-time cutoff; choose applicable or last-observed"
    );
    let mut items = Vec::new();
    let mut excluded = 0;
    let mut indeterminate = 0;
    let current_time = jiff::Timestamp::now();
    for id in strings(&state, "assertions")? {
        let record = load_record(base, id, state["sequence"].as_u64().unwrap())?;
        let Some(revision) = record["revisions"]
            .as_array()
            .ok_or_else(|| anyhow!("invalid_assertion_history"))?
            .iter()
            .rev()
            .find(|v| v["sequence"].as_u64().is_some_and(|s| s <= seq))
        else {
            continue;
        };
        if revision["retracted"] == true {
            continue;
        }
        let content = &revision["content"];
        if let Some(concept) = concept
            && !strings(content, "concepts")?.contains(&concept)
        {
            continue;
        }
        if view == "last-observed" && content["temporal"]["kind"] != "observed" {
            excluded += 1;
            continue;
        }
        if view == "last-observed"
            && let Some(query) = &query
        {
            match temporal::compare(&temporal::point(&content["temporal"]["at"])?, query) {
                Some(std::cmp::Ordering::Greater) => {
                    excluded += 1;
                    continue;
                }
                None => {
                    indeterminate += 1;
                    continue;
                }
                _ => {}
            }
        }
        if view == "applicable" {
            match temporal::applicable(&content["temporal"], query.as_ref().unwrap())? {
                Some(true) => {}
                Some(false) => {
                    excluded += 1;
                    continue;
                }
                None => {
                    indeterminate += 1;
                    continue;
                }
            }
        }
        let mut item = revision.clone();
        item["id"] = json!(id);
        item["temporal_status"] = json!(temporal::label(&content["temporal"]));
        item["freshness"] = json!(match content["review_after"].as_str() {
            Some(at) if temporal::timestamp(at)? <= current_time => "review_due",
            Some(_) => "within_review_window",
            None => "unspecified",
        });
        item["conflicts"] = json!([]);
        items.push(item);
    }
    if view == "last-observed" {
        // Keep ties and incomparable calendar/instant observations visible.
        let mut keep = vec![true; items.len()];
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, item) in items.iter().enumerate() {
            if let Some(key) = group_key(&item["content"]) {
                groups.entry(key).or_default().push(i);
            }
        }
        for group in groups.values() {
            for &i in group {
                for &j in group {
                    if i != j
                        && comparable_key(&items[i]["content"], &items[j]["content"])
                        && temporal::compare(
                            &temporal::point(&items[i]["content"]["temporal"]["at"])?,
                            &temporal::point(&items[j]["content"]["temporal"]["at"])?,
                        ) == Some(std::cmp::Ordering::Less)
                    {
                        keep[i] = false;
                    }
                }
            }
        }
        items = items
            .into_iter()
            .zip(keep)
            .filter_map(|(item, keep)| keep.then_some(item))
            .collect();
    }
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, item) in items.iter().enumerate() {
        if item["content"]["cardinality"] == "single"
            && item["content"].get("value").is_some()
            && let Some(key) = group_key(&item["content"])
        {
            groups.entry(key).or_default().push(i);
        }
    }
    for group in groups.values() {
        for (offset, &i) in group.iter().enumerate() {
            for &j in &group[(offset + 1)..] {
                let a = &items[i]["content"];
                let b = &items[j]["content"];
                if a["cardinality"] == "single"
                    && b["cardinality"] == "single"
                    && comparable_key(a, b)
                    && a.get("value").is_some()
                    && b.get("value").is_some()
                    && a["value"] != b["value"]
                    && temporal::overlaps(&a["temporal"], &b["temporal"])? == Some(true)
                {
                    let ai = items[i]["id"].clone();
                    let bi = items[j]["id"].clone();
                    items[i]["conflicts"].as_array_mut().unwrap().push(bi);
                    items[j]["conflicts"].as_array_mut().unwrap().push(ai);
                }
            }
        }
    }
    Ok(
        json!({"items":items,"sequence":seq,"current_sequence":state["sequence"],"view":view,"excluded":excluded,"indeterminate":indeterminate,"consistency":"accepted_revision","valid_at":filter.valid_at,"known_at":filter.known_at,"freshness_evaluated_at":current_time.to_string()}),
    )
}
fn comparable_key(a: &Value, b: &Value) -> bool {
    a.get("subject").is_some()
        && a.get("predicate").is_some()
        && a["subject"] == b["subject"]
        && a["predicate"] == b["predicate"]
        && a.get("scope") == b.get("scope")
        && basis_group(a) == basis_group(b)
}
fn group_key(value: &Value) -> Option<String> {
    if value.get("subject").is_none() || value.get("predicate").is_none() {
        return None;
    }
    Some(
        json!([
            value.get("subject"),
            value.get("predicate"),
            value.get("scope"),
            basis_group(value)
        ])
        .to_string(),
    )
}
fn basis_group(value: &Value) -> &str {
    match value["basis"].as_str().unwrap_or("") {
        "observed" | "reported" | "inferred" => "descriptive",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Base) {
        let dir = tempfile::tempdir().unwrap();
        crate::library::initialize(dir.path(), "example", "Synthetic example", "Example Author")
            .unwrap();
        let mut base = Base::open(dir.path(), "example").unwrap();
        crate::library::source_add(&mut base,json!({"id":"fixture","title":"Synthetic report","type":"observation","uri":"observation:synthetic","checked_at":"2026-01-01"})).unwrap();
        memory_init(&mut base).unwrap();
        let mut profile = state(&base).unwrap();
        for key in ["created_at", "history_started_at", "last_recorded_at"] {
            profile[key] = json!("2000-01-01T00:00:00Z");
        }
        let write = base.write(STATE, &profile).unwrap();
        base.commit(vec![write]).unwrap();
        apply_at(&mut base,vec![json!({"op":"concept-init","id":"routing","input":{"title":"Routing","summary":"Synthetic service route knowledge"}})],None,"2026-01-01T00:00:00Z").unwrap();
        (dir, base)
    }
    fn claim(id: &str, value: u16) -> Value {
        json!({"id":id,"concepts":["routing"],"text":format!("Synthetic service uses port {value}"),"basis":"reported","subject":"service:example","predicate":"port","value":value,"cardinality":"single","scope":{"environment":"synthetic"},"temporal":{"kind":"interval","from":{"type":"instant","value":"2026-01-01T00:00:00Z"},"until":null},"sources":["fixture"],"custom_optional":{"keep":true}})
    }
    fn insert(base: &mut Base, input: Value, at: &str) -> Value {
        apply_at(base, vec![json!({"op":"remember","input":input})], None, at).unwrap()
    }
    fn rev(base: &Base, id: &str) -> String {
        fact(base, id).unwrap()["revision"].as_str().unwrap().into()
    }
    #[test]
    fn real_change_preserves_both_valid_and_known_axes() {
        let (_dir, mut base) = fixture();
        insert(
            &mut base,
            claim("port-before", 4100),
            "2026-01-10T12:00:00Z",
        );
        let expected = rev(&base, "port-before");
        let mut after = claim("port-after", 4200);
        after["temporal"]["from"]["value"] = json!("2026-01-12T00:00:00Z");
        apply_at(&mut base,vec![json!({"op":"revise","id":"port-before","mode":"change","expect_revision":expected,"input":{"effective_at":{"type":"instant","value":"2026-01-12T00:00:00Z"},"replacement":after}})],None,"2026-01-16T12:00:00Z").unwrap();
        let filter = FactFilter {
            valid_at: Some("2026-01-14T00:00:00Z".into()),
            known_at: Some("2026-01-15T00:00:00Z".into()),
            ..Default::default()
        };
        let prior = facts(&base, None, &filter).unwrap();
        assert_eq!(prior["items"].as_array().unwrap().len(), 1);
        assert_eq!(prior["items"][0]["content"]["value"], 4100);
        let current = facts(
            &base,
            None,
            &FactFilter {
                known_at: None,
                ..filter
            },
        )
        .unwrap();
        assert_eq!(current["items"].as_array().unwrap().len(), 1);
        assert_eq!(current["items"][0]["content"]["value"], 4200);
        let boundary = facts(
            &base,
            None,
            &FactFilter {
                valid_at: Some("2026-01-12T00:00:00Z".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(boundary["items"].as_array().unwrap().len(), 1);
        assert_eq!(
            history(&base, "port-before").unwrap()["revisions"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn late_correction_retains_identity_unknown_metadata_and_source_snapshot() {
        let (_dir, mut base) = fixture();
        insert(&mut base, claim("port", 4100), "2026-01-10T12:00:00Z");
        let expected = rev(&base, "port");
        apply_at(&mut base,vec![json!({"op":"revise","id":"port","mode":"correct","expect_revision":expected,"input":{"reason":"Synthetic transcription correction","content":{"value":4200,"text":"Corrected synthetic port"}}})],None,"2026-01-16T12:00:00Z").unwrap();
        let current = fact(&base, "port").unwrap();
        assert_eq!(current["content"]["custom_optional"]["keep"], true);
        assert_eq!(current["source_snapshots"][0]["title"], "Synthetic report");
        let prior = facts(
            &base,
            None,
            &FactFilter {
                known_at: Some("2026-01-15T00:00:00Z".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(prior["items"][0]["content"]["value"], 4100);
        assert!(
            revise(
                &mut base,
                "port",
                "correct",
                json!({"reason":"stale","content":{"value":4300}}),
                Some(&expected),
                None
            )
            .is_err()
        );
        let expected = rev(&base, "port");
        apply_at(&mut base,vec![json!({"op":"revise","id":"port","mode":"correct","expect_revision":expected,"input":{"reason":"Remove explicitly obsolete optional metadata","remove_fields":["custom_optional"]}})],None,"2026-01-17T12:00:00Z").unwrap();
        assert!(
            fact(&base, "port").unwrap()["content"]
                .get("custom_optional")
                .is_none()
        );
    }
    #[test]
    fn retraction_is_not_negation_and_history_remains_portable() {
        let (dir, mut base) = fixture();
        insert(&mut base, claim("port", 4100), "2026-01-10T12:00:00Z");
        let expected = rev(&base, "port");
        apply_at(&mut base,vec![json!({"op":"revise","id":"port","mode":"retract","expect_revision":expected,"input":{"reason":"Source withdrew the statement"}})],None,"2026-01-16T12:00:00Z").unwrap();
        assert!(
            facts(&base, None, &FactFilter::default()).unwrap()["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(fact(&base, "port").unwrap()["content"]["value"], 4100);
        let target = tempfile::tempdir().unwrap();
        fn copy(source: &std::path::Path, target: &std::path::Path) {
            std::fs::create_dir_all(target).unwrap();
            for entry in std::fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target.join(entry.file_name()));
                } else {
                    std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
                }
            }
        }
        copy(&dir.path().join("bases"), &target.path().join("bases"));
        std::fs::copy(
            dir.path().join("catalog.json"),
            target.path().join("catalog.json"),
        )
        .unwrap();
        let restored = Base::open(target.path(), "example").unwrap();
        let filter = FactFilter {
            known_at: Some("2026-01-15T00:00:00Z".into()),
            ..Default::default()
        };
        assert_eq!(
            facts(&base, None, &filter).unwrap()["items"],
            facts(&restored, None, &filter).unwrap()["items"]
        );
    }
    #[test]
    fn observations_unknown_timing_and_last_observed_are_explicit() {
        let (_dir, mut base) = fixture();
        let mut first = claim("observed-one", 4100);
        first["basis"] = json!("observed");
        first["temporal"] =
            json!({"kind":"observed","at":{"type":"instant","value":"2026-01-10T12:00:00Z"}});
        insert(&mut base, first.clone(), "2026-01-10T12:01:00Z");
        let expected = rev(&base, "observed-one");
        let mut second = first;
        second["id"] = json!("observed-two");
        second["temporal"]["at"]["value"] = json!("2026-01-12T12:00:00Z");
        apply_at(&mut base,vec![json!({"op":"revise","id":"observed-one","mode":"reconfirm","expect_revision":expected,"input":{"replacement":second}})],None,"2026-01-12T12:01:00Z").unwrap();
        let applicable = facts(
            &base,
            None,
            &FactFilter {
                valid_at: Some("2026-01-12T12:00:00Z".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(applicable["excluded"], 2);
        assert!(applicable["items"].as_array().unwrap().is_empty());
        let prior = facts(
            &base,
            None,
            &FactFilter {
                valid_at: Some("2026-01-11T12:00:00Z".into()),
                view: Some("last-observed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(prior["items"][0]["id"], "observed-one");
        let latest = facts(
            &base,
            None,
            &FactFilter {
                view: Some("last-observed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(latest["items"].as_array().unwrap().len(), 1);
        assert_eq!(latest["items"][0]["id"], "observed-two");
        let mut unknown = claim("unknown", 4300);
        unknown["temporal"] = json!({"kind":"unknown","description":"Timing unavailable"});
        insert(&mut base, unknown, "2026-01-13T12:00:00Z");
        assert_eq!(
            facts(
                &base,
                None,
                &FactFilter {
                    valid_at: Some("2026-01-13T12:00:00Z".into()),
                    ..Default::default()
                }
            )
            .unwrap()["indeterminate"],
            1
        );
    }
    #[test]
    fn failed_batch_is_atomic_and_idempotency_checks_payload() {
        let (_dir, mut base) = fixture();
        let before = state(&base).unwrap();
        let operations = vec![
            json!({"op":"remember","input":claim("never-written",4100)}),
            json!({"op":"remember","input":{"id":"invalid"}}),
        ];
        assert!(batch(&mut base, operations, None).is_err());
        assert_eq!(state(&base).unwrap(), before);
        assert!(!base.root.join(assertion_path("never-written")).exists());
        let operation = vec![json!({"op":"remember","input":claim("once",4100)})];
        let first = apply_at(
            &mut base,
            operation.clone(),
            Some("same-request"),
            "2026-01-10T12:00:00Z",
        )
        .unwrap();
        assert_eq!(
            apply_at(
                &mut base,
                operation,
                Some("same-request"),
                "2026-01-11T12:00:00Z"
            )
            .unwrap(),
            first
        );
        assert!(
            apply_at(
                &mut base,
                vec![json!({"op":"remember","input":claim("different",4200)})],
                Some("same-request"),
                "2026-01-11T12:00:00Z"
            )
            .is_err()
        );
    }
    #[test]
    fn clocks_fail_closed_but_commit_sequence_remains_queryable() {
        let (_dir, mut base) = fixture();
        insert(&mut base, claim("port", 4100), "2026-01-10T12:00:00Z");
        let expected = rev(&base, "port");
        apply_at(&mut base,vec![json!({"op":"revise","id":"port","mode":"correct","expect_revision":expected,"input":{"reason":"Correction during regressed clock","content":{"value":4200}}})],None,"2026-01-09T12:00:00Z").unwrap();
        let error = facts(
            &base,
            None,
            &FactFilter {
                known_at: Some("2026-01-09T18:00:00Z".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().starts_with("clock_regression"));
        assert_eq!(
            facts(
                &base,
                None,
                &FactFilter {
                    known_revision: Some(2),
                    ..Default::default()
                }
            )
            .unwrap()["items"][0]["content"]["value"],
            4100
        );
        assert!(
            facts(
                &base,
                None,
                &FactFilter {
                    known_at: Some("1999-01-01T00:00:00Z".into()),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .to_string()
            .starts_with("history_unavailable")
        );
    }
    #[test]
    fn conflicts_are_scoped_and_requirements_do_not_overwrite_observations() {
        let (_dir, mut base) = fixture();
        insert(&mut base, claim("port-a", 4100), "2026-01-10T12:00:00Z");
        insert(&mut base, claim("port-b", 4200), "2026-01-10T12:00:00Z");
        let mut requirement = claim("desired", 4300);
        requirement["basis"] = json!("requirement");
        insert(&mut base, requirement, "2026-01-10T12:00:00Z");
        let result = facts(&base, None, &FactFilter::default()).unwrap();
        assert_eq!(result["items"][0]["conflicts"], json!(["port-b"]));
        assert_eq!(result["items"][1]["conflicts"], json!(["port-a"]));
        assert!(
            result["items"][2]["conflicts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn untrusted_input_cannot_backdate_acceptance_or_fabricate_sources() {
        let (_dir, mut base) = fixture();
        let mut input = claim("bad", 4100);
        input["recorded_at"] = json!("1900-01-01T00:00:00Z");
        assert!(remember(&mut base, input, None).is_err());
        let mut input = claim("bad", 4100);
        input["sources"] = json!(["missing"]);
        assert!(remember(&mut base, input, None).is_err());
        let mut input = claim("bad", 4100);
        input["temporal"] =
            json!({"kind":"observed","at":{"type":"instant","value":"2030-01-01T00:00:00Z"}});
        assert!(
            apply_at(
                &mut base,
                vec![json!({"op":"remember","input":input})],
                None,
                "2026-01-01T12:00:00Z"
            )
            .is_err()
        );
    }
    #[test]
    fn history_bytes_and_unsupported_profiles_are_checked() {
        let (_dir, mut base) = fixture();
        insert(&mut base, claim("port", 4100), "2026-01-10T12:00:00Z");
        let mut record = history(&base, "port").unwrap();
        record["revisions"][0]["content"]["value"] = json!(9999);
        std::fs::write(
            base.root.join(assertion_path("port")),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(
            fact(&base, "port")
                .unwrap_err()
                .to_string()
                .starts_with("history_integrity")
        );
        let mut profile = state(&base).unwrap();
        profile["profile"] = json!("tars-memory/99.0");
        std::fs::write(base.root.join(STATE), serde_json::to_vec(&profile).unwrap()).unwrap();
        assert!(
            concept_list(&base)
                .unwrap_err()
                .to_string()
                .starts_with("unsupported_profile")
        );
    }
    #[test]
    fn full_validation_checks_transactions_and_asset_registration() {
        let (_dir, mut base) = fixture();
        insert(&mut base, claim("port", 4100), "2026-01-10T12:00:00Z");
        assert_eq!(validate(&base).unwrap()["assertions"], 1);
        let mut profile = state(&base).unwrap();
        profile["transactions"][1]["recorded_at"] = json!("2026-01-11T12:00:00Z");
        let write = base.write(STATE, &profile).unwrap();
        base.commit(vec![write]).unwrap();
        assert!(validate(&base).is_err());
        let mut manifest = base.manifest.clone();
        manifest["assets"]
            .as_array_mut()
            .unwrap()
            .retain(|asset| asset["path"] != assertion_path("port"));
        let write = base.write("knowledge-base.json", &manifest).unwrap();
        base.commit(vec![write]).unwrap();
        assert!(
            fact(&base, "port")
                .unwrap_err()
                .to_string()
                .starts_with("undeclared_asset")
        );
    }
    #[test]
    fn reconfirmation_lineage_rejects_cycles_without_touching_history() {
        let (_dir, mut base) = fixture();
        insert(&mut base, claim("first", 4100), "2026-01-10T12:00:00Z");
        let mut second = claim("second", 4100);
        second["relations"] = json!([{"kind":"reconfirms","target":"first"}]);
        insert(&mut base, second, "2026-01-11T12:00:00Z");
        let before = state(&base).unwrap();
        let expected = rev(&base, "first");
        assert!(apply_at(&mut base,vec![json!({"op":"revise","id":"first","mode":"correct","expect_revision":expected,"input":{"reason":"Synthetic cycle attempt","content":{"relations":[{"kind":"reconfirms","target":"second"}]}}})],None,"2026-01-12T12:00:00Z").unwrap_err().to_string().starts_with("relation_cycle"));
        assert_eq!(state(&base).unwrap(), before);
    }
}
