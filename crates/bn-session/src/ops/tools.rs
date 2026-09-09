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

/// Render arc-strength rows as CSV text (header + one row per arc).
/// Platform-agnostic; callers do the I/O.
pub fn arc_strengths_csv(rows: &[ArcStrengthRow]) -> Result<String, CmdError> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["parent", "child", "strength_bits"])
        .map_err(|e| CmdError::Io(e.to_string()))?;
    for r in rows {
        wtr.write_record([&r.parent_name, &r.child_name, &r.mutual_info.to_string()])
            .map_err(|e| CmdError::Io(e.to_string()))?;
    }
    let buf = wtr.into_inner().map_err(|e| CmdError::Io(e.to_string()))?;
    String::from_utf8(buf).map_err(|e| CmdError::Io(e.to_string()))
}

/// Render sensitivity-to-findings rows as CSV text (header + one row per
/// candidate node). `target_name` is repeated per row so the file records
/// which target the run was for. Platform-agnostic; callers do the I/O.
pub fn sensitivity_csv(target_name: &str, rows: &[SensRowView]) -> Result<String, CmdError> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["target", "node", "mutual_info_bits", "entropy_reduction_pct", "variance_reduction"])
        .map_err(|e| CmdError::Io(e.to_string()))?;
    for r in rows {
        let variance = r.variance_reduction.map(|v| v.to_string()).unwrap_or_default();
        wtr.write_record([
            target_name,
            &r.name,
            &r.mutual_info.to_string(),
            &r.entropy_reduction_pct.to_string(),
            &variance,
        ])
        .map_err(|e| CmdError::Io(e.to_string()))?;
    }
    let buf = wtr.into_inner().map_err(|e| CmdError::Io(e.to_string()))?;
    String::from_utf8(buf).map_err(|e| CmdError::Io(e.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_strengths_csv_quotes_names() {
        let row = |p: &str, c: &str, mi: f64| ArcStrengthRow {
            parent: NodeId::default(),
            child: NodeId::default(),
            parent_name: p.into(),
            child_name: c.into(),
            mutual_info: mi,
        };
        let rows = vec![row("Smoking, heavy", "Lung \"cancer\"", 0.25), row("A", "B", 0.0625)];
        let csv = arc_strengths_csv(&rows).unwrap();
        assert_eq!(
            csv,
            "parent,child,strength_bits\n\
             \"Smoking, heavy\",\"Lung \"\"cancer\"\"\",0.25\n\
             A,B,0.0625\n"
        );
    }

    #[test]
    fn sensitivity_csv_records_target_and_optional_variance() {
        let row = |n: &str, mi: f64, ent: f64, var: Option<f64>| SensRowView {
            node: NodeId::default(),
            name: n.into(),
            mutual_info: mi,
            entropy_reduction_pct: ent,
            variance_reduction: var,
        };
        let rows = vec![row("Smoking, heavy", 0.25, 12.5, Some(0.125)), row("B", 0.0625, 3.5, None)];
        let csv = sensitivity_csv("Lung \"cancer\"", &rows).unwrap();
        assert_eq!(
            csv,
            "target,node,mutual_info_bits,entropy_reduction_pct,variance_reduction\n\
             \"Lung \"\"cancer\"\"\",\"Smoking, heavy\",0.25,12.5,0.125\n\
             \"Lung \"\"cancer\"\"\",B,0.0625,3.5,\n"
        );
    }
}
