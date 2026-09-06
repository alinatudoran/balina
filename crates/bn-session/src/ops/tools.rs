//! Analysis tools: sensitivity to findings, influence-diagram solving.

use bn_core::model::{NodeId, NodeKind};

use crate::doc::Document;
use crate::error::CmdError;
use crate::session::Session;
use crate::views::{ArcStrengthRow, check_node, IdSolutionView, SensRowView};

pub fn run_sensitivity(s: &mut Session, target: NodeId) -> Result<Vec<SensRowView>, CmdError> {
    check_node(&s.doc.net, target)?;
    if s.doc.net.node(target).kind == NodeKind::Utility {
        return Err(CmdError::BadRequest("target must be a chance or decision node".into()));
    }
    let cands: Vec<NodeId> = s
        .doc
        .net
        .nodes()
        .filter(|(id, n)| n.kind != NodeKind::Utility && *id != target)
        .map(|(id, _)| id)
        .collect();
    let engine = s.bridge.engine_mut(&s.doc);
    let rows = bn_core::sensitivity::sensitivity_to_findings(engine, &s.doc.net, target, &cands)?;
    Ok(rows
        .into_iter()
        .map(|r| SensRowView {
            node: r.node,
            name: s.doc.net.node(r.node).name.clone(),
            mutual_info: r.mutual_info,
            entropy_reduction_pct: r.entropy_reduction_pct,
            variance_reduction: r.variance_reduction,
        })
        .collect())
}

pub fn solve_influence_diagram(s: &Session) -> Result<IdSolutionView, CmdError> {
    let sol = bn_core::decision::solve_influence_diagram(&s.doc.net, &s.doc.evidence)?;
    Ok(IdSolutionView { meu: sol.meu, text: format_id_solution(&s.doc, &sol) })
}

pub fn run_arc_strengths(s: &mut Session) -> Result<Vec<ArcStrengthRow>, CmdError> {
    use bn_core::inference::Finding;
    use bn_core::InferenceError;

    let edges: Vec<(NodeId, NodeId)> = s.doc.net.edges().into_iter().collect();
    if edges.is_empty() {
        return Ok(vec![]);
    }
    let engine = s.bridge.engine_mut(&s.doc);
    let saved = engine.evidence().clone();

    let mut out = Vec::with_capacity(edges.len());
    for (parent, child) in &edges {
        let p_parent = match engine.beliefs(*parent) {
            Ok(p) => p,
            Err(_) => {
                engine.set_evidence(saved.clone());
                continue;
            }
        };
        let p_child = match engine.beliefs(*child) {
            Ok(p) => p,
            Err(_) => {
                engine.set_evidence(saved.clone());
                continue;
            }
        };
        // I(X;Y) = Σ_x P(x) · Σ_y P(y|x) · log₂[P(y|x)/P(y)]
        let mut mi = 0.0f64;
        for (sx, &px) in p_parent.iter().enumerate() {
            if px <= 0.0 {
                continue;
            }
            engine.set_evidence(saved.clone());
            engine.set_finding(*parent, Finding::Hard(sx))?;
            let post = match engine.beliefs(*child) {
                Ok(p) => p,
                Err(InferenceError::ConflictingEvidence) => continue,
                Err(e) => {
                    engine.set_evidence(saved);
                    return Err(e.into());
                }
            };
            for (&py_given_x, &py) in post.iter().zip(p_child.iter()) {
                if py_given_x > 0.0 && py > 0.0 {
                    mi += px * py_given_x * (py_given_x / py).log2();
                }
            }
        }
        let parent_name = s.doc.net.node(*parent).name.clone();
        let child_name = s.doc.net.node(*child).name.clone();
        out.push(ArcStrengthRow {
            parent: *parent,
            child: *child,
            parent_name,
            child_name,
            mutual_info: mi.max(0.0),
        });
    }
    engine.set_evidence(saved);
    out.sort_by(|a, b| b.mutual_info.partial_cmp(&a.mutual_info).unwrap());
    Ok(out)
}

/// Format an ID solution as readable text.
pub fn format_id_solution(doc: &Document, sol: &bn_core::decision::IdSolution) -> String {
    let mut out = format!("Maximum expected utility: {:.4}\n", sol.meu);
    for pol in &sol.policies {
        let dname = doc.net.node(pol.decision).name.clone();
        out.push_str(&format!("\nPolicy for {dname}:\n"));
        if pol.domain.is_empty() {
            let st = &doc.net.node(pol.decision).states[pol.best[0]].name;
            out.push_str(&format!("  always → {st}\n"));
            continue;
        }
        let header: Vec<String> =
            pol.domain.iter().map(|&d| doc.net.node(d).name.clone()).collect();
        for (i, &b) in pol.best.iter().enumerate() {
            let a = bn_core::factor::decode_index(i, &pol.domain_cards);
            let cond: Vec<String> = a
                .iter()
                .zip(&pol.domain)
                .zip(&header)
                .map(|((&st, &d), h)| format!("{h}={}", doc.net.node(d).states[st].name))
                .collect();
            out.push_str(&format!(
                "  {} → {}\n",
                cond.join(", "),
                doc.net.node(pol.decision).states[b].name
            ));
        }
    }
    out
}
