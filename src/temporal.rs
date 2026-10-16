//! Deliberate temporal semantics. Observations never acquire implicit duration.
use crate::Result;
use anyhow::{anyhow, bail, ensure};
use jiff::{Timestamp, civil::Date, tz::TimeZone};
use serde_json::{Value, json};
use std::cmp::Ordering;

#[derive(Clone, Debug)]
pub enum Point {
    Instant(Timestamp),
    Date(Date, Option<String>),
}

pub fn timestamp(value: &str) -> Result<Timestamp> {
    ensure!(
        value.len() >= 20
            && matches!(value.as_bytes().get(10), Some(b'T' | b't'))
            && value.as_bytes().get(13) == Some(&b':')
            && value.as_bytes().get(16) == Some(&b':'),
        "invalid_time: expected an offset-qualified RFC 3339 timestamp with seconds"
    );
    ensure!(
        value.bytes().all(|b| b.is_ascii_digit()
            || matches!(b, b'-' | b'+' | b':' | b'.' | b'T' | b't' | b'Z' | b'z')),
        "invalid_time: unsupported RFC 3339 timestamp spelling"
    );
    let offset = value
        .as_bytes()
        .get(value.len().saturating_sub(6)..)
        .unwrap_or_default();
    let qualified = value.ends_with(['Z', 'z'])
        || (offset.len() == 6
            && matches!(offset[0], b'+' | b'-')
            && offset[3] == b':'
            && [1, 2, 4, 5].into_iter().all(|i| offset[i].is_ascii_digit()));
    ensure!(
        qualified,
        "invalid_time: RFC 3339 requires Z or an explicit HH:MM offset"
    );
    ensure!(
        !value
            .split(['T', 't'])
            .nth(1)
            .is_some_and(|v| v.get(6..8) == Some("60")),
        "invalid_time: leap seconds are not supported"
    );
    value.parse().map_err(|e| anyhow!("invalid_time: {e}"))
}

pub fn query_point(value: &str) -> Result<Point> {
    if value.len() == 10 {
        Ok(Point::Date(parse_date(value)?, None))
    } else {
        Ok(Point::Instant(timestamp(value)?))
    }
}
fn parse_date(text: &str) -> Result<Date> {
    ensure!(
        text.len() == 10
            && text.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }),
        "invalid_date: expected YYYY-MM-DD"
    );
    text.parse().map_err(|e| anyhow!("invalid_date: {e}"))
}

pub fn point(value: &Value) -> Result<Point> {
    if let Some(original) = value.get("source_value") {
        ensure!(
            original.is_string(),
            "invalid_time: source_value must be text"
        );
    }
    let text = value["value"]
        .as_str()
        .ok_or_else(|| anyhow!("invalid_time: point.value is required"))?;
    match value["type"].as_str() {
        Some("instant") => {
            ensure!(
                value.get("time_zone").is_none(),
                "invalid_time: instant point cannot also declare a calendar timezone"
            );
            Ok(Point::Instant(timestamp(text)?))
        }
        Some("date") => {
            let date = parse_date(text)?;
            let zone = match value.get("time_zone") {
                None => None,
                Some(Value::String(zone)) => {
                    TimeZone::get(zone).map_err(|e| anyhow!("invalid_zone: {e}"))?;
                    Some(zone.clone())
                }
                Some(_) => bail!("invalid_zone: time_zone must be a named zone string or absent"),
            };
            Ok(Point::Date(date, zone))
        }
        _ => bail!("invalid_time: point.type must be instant or date"),
    }
}

pub fn normalize_point(value: &mut Value) -> Result<()> {
    let parsed = point(value)?;
    if let Point::Instant(at) = parsed {
        let original = value["value"].as_str().unwrap().to_string();
        value["value"] = json!(at.to_string());
        if original != at.to_string() && value.get("source_value").is_none() {
            value["source_value"] = json!(original);
        }
    }
    Ok(())
}

pub fn compare(a: &Point, b: &Point) -> Option<Ordering> {
    match (a, b) {
        (Point::Instant(a), Point::Instant(b)) => Some(a.cmp(b)),
        (Point::Date(a, az), Point::Date(b, bz)) if az == bz || az.is_none() || bz.is_none() => {
            Some(a.cmp(b))
        }
        _ => None,
    }
}

pub fn normalize(value: &mut Value) -> Result<()> {
    let forbidden: &[&str] = match value["kind"].as_str() {
        Some("timeless") => &["at", "from", "until", "description"],
        Some("observed" | "event") => &["from", "until", "description"],
        Some("interval") => &["at", "description"],
        Some("unknown") => &["at", "from", "until"],
        _ => &[],
    };
    for key in forbidden {
        ensure!(
            value.get(*key).is_none(),
            "invalid_time: temporal kind has incompatible {key} field"
        );
    }
    match value["kind"].as_str() {
        Some("timeless") => {}
        Some("unknown") => ensure!(
            value["description"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty()),
            "invalid_time: unknown timing needs a description"
        ),
        Some("observed" | "event") => normalize_point(
            value
                .get_mut("at")
                .ok_or_else(|| anyhow!("invalid_time: at is required"))?,
        )?,
        Some("interval") => {
            normalize_point(
                value
                    .get_mut("from")
                    .ok_or_else(|| anyhow!("invalid_time: from is required"))?,
            )?;
            let from = point(&value["from"])?;
            ensure!(
                value.get("until").is_some(),
                "invalid_time: until is required; null means an explicitly open end"
            );
            if !value["until"].is_null() {
                normalize_point(&mut value["until"])?;
                let until = point(&value["until"])?;
                ensure!(
                    same_basis(&from, &until),
                    "invalid_time: interval endpoints must use the same precision and zone"
                );
                ensure!(
                    compare(&from, &until) == Some(Ordering::Less),
                    "invalid_time: interval must have from < until"
                );
            }
        }
        _ => bail!(
            "invalid_time: explicit timeless, observed, event, interval or unknown kind required"
        ),
    }
    Ok(())
}

fn same_basis(a: &Point, b: &Point) -> bool {
    matches!((a, b), (Point::Instant(_), Point::Instant(_)))
        || matches!((a,b), (Point::Date(_,az),Point::Date(_,bz)) if az == bz)
}

fn interval_query(from: &Point, query: &Point) -> Result<Option<Point>> {
    match (from, query) {
        (Point::Date(_, Some(zone)), Point::Instant(at)) => Ok(Some(Point::Date(
            at.to_zoned(TimeZone::get(zone)?).date(),
            Some(zone.clone()),
        ))),
        (Point::Date(_, None), Point::Instant(_)) | (Point::Instant(_), Point::Date(_, _)) => {
            Ok(None)
        }
        _ => Ok(Some(query.clone())),
    }
}

/// Applicability returns true, false or unknown. Events and observations are
/// evidence, not continuing state, and are deliberately excluded from this view.
pub fn applicable(temporal: &Value, query: &Point) -> Result<Option<bool>> {
    match temporal["kind"].as_str() {
        Some("timeless") => Ok(Some(true)),
        Some("observed" | "event") => Ok(Some(false)),
        Some("unknown") => Ok(None),
        Some("interval") => {
            let from = point(&temporal["from"])?;
            let Some(q) = interval_query(&from, query)? else {
                return Ok(None);
            };
            let Some(start) = compare(&q, &from) else {
                return Ok(None);
            };
            if start == Ordering::Less {
                return Ok(Some(false));
            }
            if temporal["until"].is_null() {
                return Ok(Some(true));
            }
            let end = point(&temporal["until"])?;
            Ok(compare(&q, &end).map(|order| order == Ordering::Less))
        }
        _ => bail!("invalid_time: unknown temporal kind"),
    }
}

pub fn label(temporal: &Value) -> &str {
    match temporal["kind"].as_str() {
        Some("observed") => "point_observation",
        Some("event") => "event",
        Some("interval") => "asserted_interval",
        Some("timeless") => "timeless",
        _ => "unknown_timing",
    }
}

pub fn overlaps(a: &Value, b: &Value) -> Result<Option<bool>> {
    let ak = a["kind"].as_str().unwrap_or("");
    let bk = b["kind"].as_str().unwrap_or("");
    if ak == "unknown" || bk == "unknown" {
        return Ok(None);
    }
    if ak == "timeless" || bk == "timeless" {
        return Ok(Some(true));
    }
    if matches!(ak, "observed" | "event") && matches!(bk, "observed" | "event") {
        return Ok(compare(&point(&a["at"])?, &point(&b["at"])?).map(|o| o == Ordering::Equal));
    }
    if ak == "interval" && bk == "interval" {
        let af = point(&a["from"])?;
        let bf = point(&b["from"])?;
        if !same_basis(&af, &bf) {
            return Ok(None);
        }
        if !a["until"].is_null() && compare(&point(&a["until"])?, &bf) != Some(Ordering::Greater) {
            return Ok(Some(false));
        }
        if !b["until"].is_null() && compare(&point(&b["until"])?, &af) != Some(Ordering::Greater) {
            return Ok(Some(false));
        }
        return Ok(Some(true));
    }
    let (interval, evidence) = if ak == "interval" { (a, b) } else { (b, a) };
    applicable(interval, &point(&evidence["at"])?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observations_never_imply_current_state() {
        let evidence =
            json!({"kind":"observed","at":{"type":"instant","value":"2026-10-16T12:00:00Z"}});
        assert_eq!(
            applicable(&evidence, &query_point("2026-10-16T12:00:00Z").unwrap()).unwrap(),
            Some(false)
        );
    }
    #[test]
    fn half_open_bounds_and_offsets() {
        let mut interval = json!({"kind":"interval","from":{"type":"instant","value":"2026-10-16T10:00:00+01:00"},"until":{"type":"instant","value":"2026-10-16T10:00:00Z"}});
        normalize(&mut interval).unwrap();
        assert_eq!(
            applicable(&interval, &query_point("2026-10-16T09:00:00Z").unwrap()).unwrap(),
            Some(true)
        );
        assert_eq!(
            applicable(&interval, &query_point("2026-10-16T10:00:00Z").unwrap()).unwrap(),
            Some(false)
        );
        assert_eq!(interval["from"]["value"], "2026-10-16T09:00:00Z");
    }
    #[test]
    fn calendar_days_use_zone_rules_without_midnight_fabrication() {
        let mut interval = json!({"kind":"interval","from":{"type":"date","value":"2026-03-29","time_zone":"Europe/London"},"until":{"type":"date","value":"2026-03-30","time_zone":"Europe/London"}});
        normalize(&mut interval).unwrap();
        assert_eq!(
            applicable(&interval, &query_point("2026-03-29T22:59:59Z").unwrap()).unwrap(),
            Some(true)
        );
        assert_eq!(
            applicable(&interval, &query_point("2026-03-29T23:00:00Z").unwrap()).unwrap(),
            Some(false)
        );
        interval["from"]
            .as_object_mut()
            .unwrap()
            .remove("time_zone");
        interval["until"]
            .as_object_mut()
            .unwrap()
            .remove("time_zone");
        assert_eq!(
            applicable(&interval, &query_point("2026-03-29T12:00:00Z").unwrap()).unwrap(),
            None
        );
    }
    #[test]
    fn invalid_and_unknown_dates_stay_explicit() {
        assert!(timestamp("2026-01-01T23:59:60Z").is_err());
        assert!(timestamp("2026-01-01t23:59:60z").is_err());
        assert!(timestamp("2026-01-01 23:59:60Z").is_err());
        assert!(timestamp("2026-01-01T23:59:59").is_err());
        assert!(timestamp("2026-01-01T23:59:59+01").is_err());
        assert!(timestamp("2026-01-01T23:59:59+0100").is_err());
        assert!(timestamp("2026-01-01T23:59:59Z[Europe/London]").is_err());
        assert!(normalize(&mut json!({"kind":"timeless","at":{"type":"instant","value":"2026-01-01T00:00:00Z"}})).is_err());
        assert!(
            normalize(&mut json!({"kind":"unknown","description":"Not recorded","until":null}))
                .is_err()
        );
        assert!(
            normalize(
                &mut json!({"kind":"event","at":{"type":"date","value":"2026-01-01"},"until":null})
            )
            .is_err()
        );
        assert!(query_point("2026-02-29").is_err());
        assert!(
            normalize(&mut json!({"kind":"interval","from":{"type":"date","value":"2026-01-01"}}))
                .is_err()
        );
        assert_eq!(
            applicable(
                &json!({"kind":"unknown","description":"not recorded"}),
                &query_point("2026-01-01").unwrap()
            )
            .unwrap(),
            None
        );
    }
}
