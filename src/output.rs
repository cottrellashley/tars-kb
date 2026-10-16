//! Bounded JSON pages. Cursors bind the result revision and exact request scope.
use crate::{Result, storage};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

pub fn bounded(
    mut value: Value,
    key: &str,
    limit: usize,
    max_bytes: usize,
    cursor: Option<&str>,
    scope: &str,
    revision: &str,
) -> Result<Value> {
    ensure!(
        (1..=1000).contains(&limit),
        "invalid_limit: expected 1..1000"
    );
    ensure!(
        (512..=8 * 1024 * 1024).contains(&max_bytes),
        "invalid_budget: expected 512..8388608 bytes"
    );
    let scope = storage::hash(scope.as_bytes());
    let offset = if let Some(c) = cursor {
        ensure!(c.len() <= 2048, "invalid_cursor");
        let c = storage::parse_json(&hex::decode(c).context("invalid_cursor")?)?;
        ensure!(
            c["revision"] == revision && c["scope"] == scope,
            "stale_cursor: query or revision changed"
        );
        c["offset"].as_u64().context("invalid_cursor")? as usize
    } else {
        0
    };
    let all = value[key]
        .as_array()
        .context("invalid_result: expected array")?
        .clone();
    ensure!(
        offset <= all.len(),
        "invalid_cursor: offset outside results"
    );
    let end = (offset + limit).min(all.len());
    value[key] = Value::Array(all[offset..end].to_vec());
    value["output_version"] = "1.0".into();
    value["revision"] = revision.into();
    loop {
        let count = value[key].as_array().unwrap().len();
        let next = offset + count;
        let more = next < all.len();
        value["truncated"] = more.into();
        value["next_cursor"] = if more && count > 0 {
            hex::encode(serde_json::to_vec(
                &json!({"revision":revision,"scope":scope,"offset":next}),
            )?)
            .into()
        } else {
            Value::Null
        };
        if serde_json::to_vec(&value)?.len() <= max_bytes {
            return Ok(value);
        }
        ensure!(
            count > 0,
            "output_limit: metadata alone exceeds requested byte budget"
        );
        value[key].as_array_mut().unwrap().pop();
        if value[key].as_array().unwrap().is_empty() {
            let required =
                serde_json::to_vec(&json!({"item":all[offset],"revision":revision}))?.len();
            value["next_cursor"] = Value::Null;
            value["truncated"] = true.into();
            value["required_item_bytes"] = required.into();
            ensure!(
                serde_json::to_vec(&value)?.len() <= max_bytes,
                "output_limit: increase --max-bytes"
            );
            return Ok(value);
        }
    }
}
pub fn single(mut value: Value, max_bytes: usize) -> Result<Value> {
    value["output_version"] = "1.0".into();
    value["truncated"] = false.into();
    let bytes = serde_json::to_vec(&value)?.len();
    ensure!(
        bytes <= max_bytes,
        "output_limit: record needs {bytes} bytes; increase --max-bytes or select a section"
    );
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_bind_scope_and_revision() {
        let v = json!({"items":[{"id":1},{"id":2}]});
        let p = bounded(v.clone(), "items", 1, 4096, None, "query", "rev").unwrap();
        assert_eq!(p["items"].as_array().unwrap().len(), 1);
        let c = p["next_cursor"].as_str().unwrap();
        assert!(bounded(v.clone(), "items", 1, 4096, Some(c), "other", "rev").is_err());
        let p2 = bounded(v, "items", 1, 4096, Some(c), "query", "rev").unwrap();
        assert_eq!(p2["items"][0]["id"], 2);
    }
    #[test]
    fn giant_item_is_not_split() {
        let v = json!({"items":[{"text":"x".repeat(4000),"temporal":"unknown"}]});
        let p = bounded(v, "items", 20, 512, None, "x", "r").unwrap();
        assert!(p["truncated"].as_bool().unwrap());
        assert!(p["items"].as_array().unwrap().is_empty());
        assert!(serde_json::to_vec(&p).unwrap().len() <= 512);
    }
}
