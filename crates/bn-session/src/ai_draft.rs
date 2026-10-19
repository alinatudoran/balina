//! AI-drafted networks: the JSON shape a text model is asked to emit, and
//! its construction into a validated [`Network`]. Everything here is pure and
//! offline — the LLM call lives in the app layer, which feeds failures back
//! to the model for one repair round-trip. Errors are therefore collected
//! exhaustively (not fail-fast) and worded for the model as much as the user.
//!
//! The draft carries structure + states only; CPTs stay at `add_node`'s
//! uniform default (and `add_edge`'s uniform-preserving expansion). Numbers
//! are a deliberate per-node act via CPT elicitation, not bulk guesses.

use bn_core::model::{Network, NodeKind, State};
use serde::Deserialize;

/// Guardrail against runaway generations, far above editor-scale networks.
pub const MAX_DRAFT_NODES: usize = 150;

#[derive(Clone, Debug, Deserialize)]
pub struct DraftNetwork {
    #[serde(default)]
    pub name: String,
    pub nodes: Vec<DraftNode>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DraftNode {
    pub name: String,
    #[serde(default)]
    pub title: String,
    /// "chance" (default) | "decision" | "utility".
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub states: Vec<String>,
    /// Parent node names; order defines the CPT's parent axes.
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub comment: String,
}

/// Parse the model's reply. Tolerates a Markdown code fence and prose around
/// the JSON object (models add both despite instructions).
pub fn parse_draft(reply: &str) -> Result<DraftNetwork, String> {
    let json = extract_json_object(reply).ok_or("reply contains no JSON object")?;
    serde_json::from_str(json).map_err(|e| format!("invalid draft JSON: {e}"))
}

/// The outermost `{...}` span, so fences and surrounding prose don't matter.
fn extract_json_object(reply: &str) -> Option<&str> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    (start < end).then(|| &reply[start..=end])
}

/// Build a validated network, or every problem found. Node/edge checks ride
/// on the same `bn-core` validation the editor uses (`add_node`/`add_edge`:
/// duplicate names, cycles, utility-with-children, …).
pub fn build_network(draft: &DraftNetwork) -> Result<Network, Vec<String>> {
    let mut errors = Vec::new();
    if draft.nodes.is_empty() {
        return Err(vec!["draft has no nodes".into()]);
    }
    if draft.nodes.len() > MAX_DRAFT_NODES {
        return Err(vec![format!(
            "draft has {} nodes; the limit is {MAX_DRAFT_NODES}",
            draft.nodes.len()
        )]);
    }

    let name = if draft.name.trim().is_empty() { "AI draft" } else { draft.name.trim() };
    let mut net = Network::new(name);

    // Pass 1: nodes + states. Skipped nodes are reported, not built, so pass
    // 2 reports their edges as unknown-node problems with context.
    for dn in &draft.nodes {
        let node_label = if dn.name.trim().is_empty() {
            "(unnamed node)".to_string()
        } else {
            format!("node `{}`", dn.name.trim())
        };
        let kind = match dn.kind.trim().to_ascii_lowercase().as_str() {
            "" | "chance" => NodeKind::Chance,
            "decision" => NodeKind::Decision,
            "utility" => NodeKind::Utility,
            other => {
                errors.push(format!(
                    "{node_label}: unknown kind `{other}` (use chance, decision, or utility)"
                ));
                continue;
            }
        };
        let states: Vec<State> = dn
            .states
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(State::new)
            .collect();
        match kind {
            NodeKind::Utility => {
                if !states.is_empty() {
                    errors.push(format!("{node_label}: utility nodes must not have states"));
                    continue;
                }
            }
            _ => {
                if states.len() < 2 {
                    errors.push(format!(
                        "{node_label}: needs at least 2 states, got {}",
                        states.len()
                    ));
                    continue;
                }
                let mut seen = std::collections::HashSet::new();
                if let Some(dup) = states.iter().find(|s| !seen.insert(s.name.as_str())) {
                    errors.push(format!("{node_label}: duplicate state `{}`", dup.name));
                    continue;
                }
            }
        }
        match net.add_node(dn.name.trim(), kind, states) {
            Ok(id) => {
                let n = net.node_mut(id);
                n.title = dn.title.trim().to_string();
                n.comment = dn.comment.trim().to_string();
            }
            Err(e) => errors.push(format!("{node_label}: {e}")),
        }
    }

    // Pass 2: edges by name, in draft order (defines the CPT parent axes).
    for dn in &draft.nodes {
        let Some(child) = net.find_by_name(dn.name.trim()) else { continue };
        for pname in &dn.parents {
            let pname = pname.trim();
            if pname == dn.name.trim() {
                errors.push(format!("node `{}`: lists itself as a parent", dn.name.trim()));
                continue;
            }
            let Some(parent) = net.find_by_name(pname) else {
                errors.push(format!(
                    "node `{}`: unknown parent `{pname}`",
                    dn.name.trim()
                ));
                continue;
            };
            if let Err(e) = net.add_edge(parent, child) {
                errors.push(format!(
                    "edge `{pname}` -> `{}`: {e}",
                    dn.name.trim()
                ));
            }
        }
    }

    if errors.is_empty() { Ok(net) } else { Err(errors) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, states: &[&str], parents: &[&str]) -> String {
        format!(
            r#"{{"name": "{name}", "states": [{}], "parents": [{}]}}"#,
            states.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(","),
            parents.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(","),
        )
    }

    fn draft(nodes: &[String]) -> DraftNetwork {
        parse_draft(&format!(r#"{{"name": "t", "nodes": [{}]}}"#, nodes.join(","))).unwrap()
    }

    fn errors_of(nodes: &[String]) -> Vec<String> {
        build_network(&draft(nodes)).unwrap_err()
    }

    #[test]
    fn happy_path_builds_uniform_cpts() {
        let d = draft(&[
            node("Smoker", &["yes", "no"], &[]),
            node("Cancer", &["present", "absent"], &["Smoker"]),
            node("Dyspnea", &["yes", "no"], &["Cancer"]),
        ]);
        let net = build_network(&d).unwrap();
        assert_eq!(net.node_ids().len(), 3);
        let cancer = net.find_by_name("Cancer").unwrap();
        // 1 binary parent × 2 own states, every row uniform.
        assert_eq!(net.node(cancer).table.data, vec![0.5, 0.5, 0.5, 0.5]);
        assert_eq!(net.node(cancer).parents.len(), 1);
    }

    #[test]
    fn parse_tolerates_fences_and_prose() {
        let reply = format!(
            "Here is the draft:\n```json\n{{\"name\": \"t\", \"nodes\": [{}]}}\n```\nDone.",
            node("A", &["y", "n"], &[])
        );
        assert_eq!(parse_draft(&reply).unwrap().nodes.len(), 1);
    }

    #[test]
    fn garbage_is_a_parse_error() {
        assert!(parse_draft("no json here").is_err());
        assert!(parse_draft("{not valid json}").is_err());
    }

    #[test]
    fn cycle_is_rejected() {
        let errs = errors_of(&[
            node("A", &["y", "n"], &["C"]),
            node("B", &["y", "n"], &["A"]),
            node("C", &["y", "n"], &["B"]),
        ]);
        assert!(errs.iter().any(|e| e.contains("cycle")), "{errs:?}");
    }

    #[test]
    fn duplicate_node_name_is_rejected() {
        let errs = errors_of(&[node("A", &["y", "n"], &[]), node("A", &["y", "n"], &[])]);
        assert!(errs.iter().any(|e| e.contains("duplicate node name")), "{errs:?}");
    }

    #[test]
    fn unknown_parent_is_rejected() {
        let errs = errors_of(&[node("A", &["y", "n"], &["Ghost"])]);
        assert!(errs.iter().any(|e| e.contains("unknown parent `Ghost`")), "{errs:?}");
    }

    #[test]
    fn self_parent_is_rejected() {
        let errs = errors_of(&[node("A", &["y", "n"], &["A"])]);
        assert!(errs.iter().any(|e| e.contains("itself as a parent")), "{errs:?}");
    }

    #[test]
    fn duplicate_edge_is_rejected() {
        let errs = errors_of(&[
            node("A", &["y", "n"], &[]),
            node("B", &["y", "n"], &["A", "A"]),
        ]);
        assert!(errs.iter().any(|e| e.contains("link already exists")), "{errs:?}");
    }

    #[test]
    fn too_few_states_is_rejected() {
        let errs = errors_of(&[node("A", &["only"], &[]), node("B", &[], &[])]);
        assert_eq!(errs.len(), 2, "{errs:?}");
        assert!(errs.iter().all(|e| e.contains("at least 2 states")));
    }

    #[test]
    fn duplicate_state_is_rejected() {
        let errs = errors_of(&[node("A", &["y", "y"], &[])]);
        assert!(errs.iter().any(|e| e.contains("duplicate state `y`")), "{errs:?}");
    }

    #[test]
    fn utility_with_children_is_rejected() {
        let d = parse_draft(
            r#"{"nodes": [
                {"name": "U", "kind": "utility"},
                {"name": "A", "states": ["y", "n"], "parents": ["U"]}
            ]}"#,
        )
        .unwrap();
        let errs = build_network(&d).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("utility nodes cannot have children")), "{errs:?}");
    }

    #[test]
    fn utility_with_states_is_rejected() {
        let d = parse_draft(
            r#"{"nodes": [{"name": "U", "kind": "utility", "states": ["a", "b"]}]}"#,
        )
        .unwrap();
        let errs = build_network(&d).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("must not have states")), "{errs:?}");
    }

    #[test]
    fn unknown_kind_is_rejected() {
        let d = parse_draft(r#"{"nodes": [{"name": "A", "kind": "chaos", "states": ["y","n"]}]}"#)
            .unwrap();
        let errs = build_network(&d).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("unknown kind `chaos`")), "{errs:?}");
    }

    #[test]
    fn empty_draft_is_rejected() {
        let d = parse_draft(r#"{"nodes": []}"#).unwrap();
        assert_eq!(build_network(&d).unwrap_err(), vec!["draft has no nodes".to_string()]);
    }

    #[test]
    fn errors_are_collected_not_fail_fast() {
        let errs = errors_of(&[
            node("A", &["only"], &[]),
            node("B", &["y", "n"], &["Ghost"]),
        ]);
        assert_eq!(errs.len(), 2, "{errs:?}");
    }

    #[test]
    fn decision_and_utility_build() {
        let d = parse_draft(
            r#"{"name": "id", "nodes": [
                {"name": "Weather", "states": ["rain", "sun"]},
                {"name": "Umbrella", "kind": "decision", "states": ["take", "leave"]},
                {"name": "Comfort", "kind": "utility", "parents": ["Weather", "Umbrella"]}
            ]}"#,
        )
        .unwrap();
        let net = build_network(&d).unwrap();
        let comfort = net.find_by_name("Comfort").unwrap();
        assert_eq!(net.node(comfort).kind, NodeKind::Utility);
        assert_eq!(net.node(comfort).parents.len(), 2);
    }
}
