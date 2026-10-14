//! Shared memory (arra) over HTTP.
//!
//! Clean-room: arra's code was not read. Routes below were observed with
//! read-only HTTP requests against one running arra; the learn BODY is still
//! an assumption. All paths are relative to `config.arra_url` and editable:
//!   GET  {arra_health_path}  (/api/health)                    → 2xx when alive
//!   POST {arra_learn_path}   (/api/v1/learn) JSON {pattern, source, concepts}
//!   GET  {arra_search_path}  (/api/v1/search)?q=<query>&limit=<n> → {results:[{content,..}]}
//! See docs/DESIGN.md.

use crate::orca::{pick_array, pick_str};
use crate::registry::Config;
use crate::Outcome;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::time::Duration;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into()
}

pub fn url(cfg: &Config, path: &str) -> String {
    format!(
        "{}/{}",
        cfg.arra_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn unreachable(cfg: &Config, e: impl std::fmt::Display) -> anyhow::Error {
    anyhow!(
        "arra memory is not reachable at {} ({e}). Start arra (port 47778) or set \
         `arra_url` in config.json under the codename home.",
        cfg.arra_url
    )
}

pub fn health(cfg: &Config) -> Result<String> {
    let mut r = agent()
        .get(url(cfg, &cfg.arra_health_path))
        .call()
        .map_err(|e| unreachable(cfg, e))?;
    let status = r.status().as_u16();
    let body = r.body_mut().read_to_string().unwrap_or_default();
    if !(200..300).contains(&status) {
        bail!("arra health returned HTTP {status}: {body}");
    }
    Ok(body)
}

pub fn learn_body(lesson: &str, source: &str) -> Value {
    json!({"pattern": lesson, "source": source, "concepts": ["codename"]})
}

pub fn remember(cfg: &Config, lesson: &str, source: &str) -> Result<Outcome> {
    let body = learn_body(lesson, source).to_string();
    let mut r = agent()
        .post(url(cfg, &cfg.arra_learn_path))
        .header("Content-Type", "application/json")
        .send(body.as_str())
        .map_err(|e| unreachable(cfg, e))?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    if !(200..300).contains(&status) {
        bail!("arra learn returned HTTP {status}: {text} (check arra_learn_path in config.json)");
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    Ok(Outcome::ok(
        "remembered",
        json!({"stored": true, "arra": v}),
    ))
}

/// Extracts (title/snippet) lines from a search response, leniently.
pub fn search_hits(v: &Value) -> Vec<String> {
    pick_array(v, &["results", "items", "hits", "documents", "docs"])
        .into_iter()
        .map(|h| {
            pick_str(
                h,
                &["content", "pattern", "text", "snippet", "title", "summary"],
            )
            .unwrap_or_else(|| h.to_string())
        })
        .collect()
}

pub fn recall(cfg: &Config, query: &str, limit: usize) -> Result<Outcome> {
    let mut r = agent()
        .get(url(cfg, &cfg.arra_search_path))
        .query("q", query)
        .query("limit", limit.to_string())
        .call()
        .map_err(|e| unreachable(cfg, e))?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    if !(200..300).contains(&status) {
        bail!("arra search returned HTTP {status}: {text} (check arra_search_path in config.json)");
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let hits = search_hits(&v);
    let human = if hits.is_empty() {
        "no matches".to_string()
    } else {
        hits.iter()
            .enumerate()
            .map(|(i, h)| {
                format!(
                    "{}. {}",
                    i + 1,
                    h.lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(160)
                        .collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok(Outcome::ok(
        human,
        json!({"query": query, "hits": hits, "raw": v}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_join() {
        let c = Config {
            arra_url: "http://127.0.0.1:47778/".into(),
            ..Default::default()
        };
        assert_eq!(
            url(&c, "/api/v1/search"),
            "http://127.0.0.1:47778/api/v1/search"
        );
    }

    #[test]
    fn hits_lenient() {
        let v = json!({"results": [{"content": "a"}, {"title": "b"}, {"x": 1}]});
        let h = search_hits(&v);
        assert_eq!(h[0], "a");
        assert_eq!(h[1], "b");
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn unreachable_is_clear() {
        let c = Config {
            arra_url: "http://127.0.0.1:9".into(),
            ..Default::default()
        };
        let e = recall(&c, "x", 3).unwrap_err().to_string();
        assert!(e.contains("not reachable"), "{e}");
    }
}
