//! Precomputed view/result structs handed to the UI, plus small helpers the
//! canvas needs (stale-id checks, ancestor sets for synchronous cycle
//! validation). The UI runs in-process and reads the [`crate::Document`] and
//! [`crate::EngineBridge`] directly — there is no snapshot DTO layer anymore;
//! only data that benefits from precomputation (CPT row labels, report text)
//! is packaged here.

use std::collections::HashSet;

use bn_core::model::{Network, NodeId, NodeKind};
use slotmap::SecondaryMap;

use crate::doc::Document;
use crate::error::CmdError;

/// Reject a stale node id (node deleted, undo rewound past its creation) as
/// a clean BadRequest, never a panic. Dialogs and the selection hold ids
/// across mutations, so stale ids are a normal occurrence.
pub fn check_node(net: &Network, id: NodeId) -> Result<(), CmdError> {
    if net.contains(id) {
        Ok(())
    } else {
        Err(CmdError::BadRequest("node no longer exists".into()))
    }
}

/// Transitive ancestor sets in topo order: anc(id) = ∪ parents p of
/// ({p} ∪ anc(p)). Lets the canvas answer cycle checks synchronously during
/// a link drag.
pub fn ancestor_sets(net: &Network) -> SecondaryMap<NodeId, HashSet<NodeId>> {
    let mut anc: SecondaryMap<NodeId, HashSet<NodeId>> = SecondaryMap::new();
    for id in net.topo_order() {
        let mut set = HashSet::new();
        for &p in &net.node(id).parents {
            set.insert(p);
            if let Some(pa) = anc.get(p) {
                set.extend(pa.iter().copied());
            }
        }
        anc.insert(id, set);
    }
    anc
}

// ---------------------------------------------------------------------------
// Result structs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SaveResult {
    pub path: std::path::PathBuf,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CptRow {
    /// Parent-state names, one per parent (axis order).
    pub labels: Vec<String>,
    pub values: Vec<f64>,
}

#[derive(Clone, Debug)]
pub struct CptView {
    pub node: NodeId,
    pub node_name: String,
    pub is_utility: bool,
    pub out_card: usize,
    /// State names, or `["utility"]` for utility nodes.
    pub column_headers: Vec<String>,
    /// Parent names, axis order.
    pub parent_headers: Vec<String>,
    pub rows: Vec<CptRow>,
}

#[derive(Clone, Debug)]
pub struct SensRowView {
    pub node: NodeId,
    pub name: String,
    pub mutual_info: f64,
    pub entropy_reduction_pct: f64,
    pub variance_reduction: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct ArcStrengthRow {
    pub parent: NodeId,
    pub child: NodeId,
    pub parent_name: String,
    pub child_name: String,
    /// I(parent; child | evidence), in bits. Always ≥ 0.
    pub mutual_info: f64,
}

#[derive(Clone, Debug)]
pub struct IdSolutionView {
    pub meu: f64,
    /// Preformatted policy text (monospace display).
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct LearnCptsResult {
    pub report: String,
    pub summary: String,
}

#[derive(Clone, Debug)]
pub struct StructureLearnResult {
    pub report: String,
    pub summary: String,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Edit drafts
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct StateDraft {
    pub name: String,
    pub value: Option<f64>,
    /// Index of the pre-edit state this draft came from (None = brand-new).
    pub orig: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct NodePropsPatch {
    pub name: String,
    pub title: String,
    pub comment: String,
    pub kind: NodeKind,
    pub states: Vec<StateDraft>,
}

// ---------------------------------------------------------------------------
// CPT view builder
// ---------------------------------------------------------------------------

pub fn cpt_view(doc: &Document, id: NodeId) -> CptView {
    let n = doc.net.node(id);
    let is_utility = n.kind == NodeKind::Utility;
    let out = n.out_card();
    let parents = n.parents.clone();
    let rows = doc.net.row_count(id);
    let row_views = (0..rows)
        .map(|r| {
            let assignment = doc.net.row_assignment(id, r);
            CptRow {
                labels: assignment
                    .iter()
                    .enumerate()
                    .map(|(ai, &st)| doc.net.node(parents[ai]).states[st].name.clone())
                    .collect(),
                values: n.table.data[r * out..(r + 1) * out].to_vec(),
            }
        })
        .collect();
    CptView {
        node: id,
        node_name: n.name.clone(),
        is_utility,
        out_card: out,
        column_headers: if is_utility {
            vec!["utility".into()]
        } else {
            n.states.iter().map(|s| s.name.clone()).collect()
        },
        parent_headers: parents.iter().map(|&p| doc.net.node(p).name.clone()).collect(),
        rows: row_views,
    }
}
