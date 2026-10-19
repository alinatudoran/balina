//! Findings extracted from free text by the structured model: parsing the
//! reply and validating it against the network's chance nodes. Pure and
//! offline like the other `ai_*` modules; errors are collected exhaustively
//! for the repair round-trip. The extraction stays document-in →
//! distributions-out so a future corpus mode can reuse it unchanged.

use serde::Deserialize;

/// Guardrail for one extraction call.
pub const MAX_EVIDENCE_NODES: usize = 64;

#[derive(Deserialize)]
struct FindingsReply {
    findings: Vec<FindingEntry>,
}

#[derive(Deserialize)]
struct FindingEntry {
    node: String,
    /// `null` = the text carries no information about this node (abstain).
    probabilities: Option<Vec<f64>>,
}

/// One parsed entry: node name, and `None` where the model abstained.
pub type ParsedFinding = (String, Option<Vec<f64>>);

/// Parse `{"findings": [{"node": ..., "probabilities": [...] | null}]}` out
/// of a model reply, tolerating fences and surrounding prose.
pub fn parse_findings_reply(reply: &str) -> Result<Vec<ParsedFinding>, String> {
    let start = reply.find('{').ok_or("reply contains no JSON object")?;
    let end = reply.rfind('}').ok_or("reply contains no JSON object")?;
    if start >= end {
        return Err("reply contains no JSON object".into());
    }
    serde_json::from_str::<FindingsReply>(&reply[start..=end])
        .map(|r| r.findings.into_iter().map(|f| (f.node, f.probabilities)).collect())
        .map_err(|e| format!("invalid findings JSON: {e}"))
}

/// Validate parsed findings against `(name, cardinality)` of the queried
/// nodes. Abstentions (null) and unmentioned nodes are dropped silently —
/// "no information" is a legal answer; everything else must line up.
/// Distributions are normalized so they read as percentages in review.
pub fn validate_findings(
    parsed: Vec<ParsedFinding>,
    nodes: &[(String, usize)],
) -> Result<Vec<(String, Vec<f64>)>, Vec<String>> {
    let mut errors = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (name, probs) in parsed {
        if !seen.insert(name.clone()) {
            errors.push(format!("node `{name}` appears more than once"));
            continue;
        }
        let Some((_, card)) = nodes.iter().find(|(n, _)| *n == name) else {
            errors.push(format!("unknown node `{name}`"));
            continue;
        };
        let Some(probs) = probs else { continue };
        if probs.len() != *card {
            errors.push(format!(
                "node `{name}`: expected {card} probabilities, got {}",
                probs.len()
            ));
            continue;
        }
        if probs.iter().any(|v| !v.is_finite() || *v < 0.0) {
            errors.push(format!("node `{name}`: values must be finite and >= 0"));
            continue;
        }
        let sum: f64 = probs.iter().sum();
        if sum <= 0.0 {
            errors.push(format!("node `{name}`: all values are zero"));
            continue;
        }
        out.push((name, probs.iter().map(|v| v / sum).collect()));
    }
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

#[derive(Deserialize)]
struct DefiniteReply {
    definite: Vec<DefiniteEntry>,
}

#[derive(Deserialize)]
struct DefiniteEntry {
    node: String,
    state: String,
}

/// Parse `{"definite": [{"node": ..., "state": ...}]}` — the text model's
/// list of findings the text states outright — tolerating fences and prose.
pub fn parse_definite_reply(reply: &str) -> Result<Vec<(String, String)>, String> {
    let start = reply.find('{').ok_or("reply contains no JSON object")?;
    let end = reply.rfind('}').ok_or("reply contains no JSON object")?;
    if start >= end {
        return Err("reply contains no JSON object".into());
    }
    serde_json::from_str::<DefiniteReply>(&reply[start..=end])
        .map(|r| r.definite.into_iter().map(|d| (d.node, d.state)).collect())
        .map_err(|e| format!("invalid definite-findings JSON: {e}"))
}

/// Map `(node, state)` pairs onto state indices of `(name, states)` nodes.
/// Snapping only refines suggestions, so unknown nodes/states and nodes
/// named more than once are dropped instead of failing the extraction.
pub fn resolve_definite(
    pairs: Vec<(String, String)>,
    nodes: &[(String, Vec<String>)],
) -> Vec<(String, usize)> {
    let mut counts = std::collections::HashMap::new();
    for (name, _) in &pairs {
        *counts.entry(name.clone()).or_insert(0usize) += 1;
    }
    pairs
        .into_iter()
        .filter(|(name, _)| counts[name] == 1)
        .filter_map(|(name, state)| {
            let (_, states) = nodes.iter().find(|(n, _)| *n == name)?;
            let idx = states.iter().position(|s| s.eq_ignore_ascii_case(state.trim()))?;
            Some((name, idx))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes() -> Vec<(String, usize)> {
        vec![("Smoking".into(), 2), ("Dyspnea".into(), 3)]
    }

    #[test]
    fn parse_tolerates_fence_prose_and_null() {
        let parsed = parse_findings_reply(
            "Findings:\n```json\n{\"findings\": [\
             {\"node\": \"Smoking\", \"probabilities\": [0.9, 0.1]},\
             {\"node\": \"Dyspnea\", \"probabilities\": null}]}\n```",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1], ("Dyspnea".to_string(), None));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_findings_reply("nope").is_err());
        assert!(parse_findings_reply("{\"rows\": []}").is_err());
    }

    #[test]
    fn validate_normalizes_and_drops_abstentions() {
        let ok = validate_findings(
            vec![
                ("Smoking".into(), Some(vec![3.0, 1.0])),
                ("Dyspnea".into(), None),
            ],
            &nodes(),
        )
        .unwrap();
        assert_eq!(ok, vec![("Smoking".to_string(), vec![0.75, 0.25])]);
    }

    #[test]
    fn validate_collects_all_problems() {
        let errs = validate_findings(
            vec![
                ("Ghost".into(), Some(vec![1.0])),
                ("Smoking".into(), Some(vec![0.5])),
                ("Dyspnea".into(), Some(vec![-1.0, 0.5, 0.5])),
            ],
            &nodes(),
        )
        .unwrap_err();
        assert_eq!(errs.len(), 3, "{errs:?}");
        assert!(errs[0].contains("unknown node `Ghost`"));
        assert!(errs[1].contains("expected 2 probabilities"));
        assert!(errs[2].contains("finite and >= 0"));
    }

    #[test]
    fn validate_rejects_duplicates_and_zero_rows() {
        let errs = validate_findings(
            vec![
                ("Smoking".into(), Some(vec![0.0, 0.0])),
                ("Smoking".into(), Some(vec![1.0, 0.0])),
            ],
            &nodes(),
        )
        .unwrap_err();
        assert!(errs.iter().any(|e| e.contains("all values are zero")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("more than once")), "{errs:?}");
    }

    #[test]
    fn parse_definite_tolerates_fence_and_empty() {
        let pairs = parse_definite_reply(
            "Sure:\n```json\n{\"definite\": [{\"node\": \"Smoking\", \"state\": \"no\"}]}\n```",
        )
        .unwrap();
        assert_eq!(pairs, vec![("Smoking".to_string(), "no".to_string())]);
        assert!(parse_definite_reply("{\"definite\": []}").unwrap().is_empty());
        assert!(parse_definite_reply("nothing definite").is_err());
    }

    #[test]
    fn resolve_definite_drops_unknown_and_duplicates() {
        let nodes = vec![
            ("Smoking".to_string(), vec!["yes".to_string(), "no".to_string()]),
            ("Asia".to_string(), vec!["yes".to_string(), "no".to_string()]),
            ("Dyspnea".to_string(), vec!["present".to_string(), "absent".to_string()]),
        ];
        let resolved = resolve_definite(
            vec![
                ("Smoking".into(), " No".into()),
                ("Ghost".into(), "yes".into()),
                ("Dyspnea".into(), "maybe".into()),
                ("Asia".into(), "yes".into()),
                ("Asia".into(), "no".into()),
            ],
            &nodes,
        );
        assert_eq!(resolved, vec![("Smoking".to_string(), 1)]);
    }
}
