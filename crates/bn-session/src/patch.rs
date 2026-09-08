//! By-name structure patches: how a structure-learning result crosses a
//! serialization boundary (the web worker). NodeIds are slotmap keys and only
//! mean something inside one `Network` instance, so
//! [`apply_structure_outcome`](crate::ops::learn::apply_structure_outcome)'s
//! clone-swap (which relies on shared NodeIds) can't be used with a network
//! that was serialized and re-parsed. Instead the worker diffs its learned
//! clone against the input ([`structure_patch`]) and the app re-applies the
//! diff to its own network by node name ([`apply_structure_patch`]).

use bn_core::model::{Network, Table};
use serde::{Deserialize, Serialize};

use crate::doc::Dirt;
use crate::error::CmdError;
use crate::session::Session;
use crate::views::StructureLearnResult;

/// One node whose parent set and/or table changed during learning.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodePatch {
    pub name: String,
    /// Parent names in the learned `Node.parents` order — this order defines
    /// the CPT's parent axes, so it must be preserved exactly.
    pub parents: Vec<String>,
    /// Learned table, `[parent_0..k, self]` layout (last axis fastest).
    pub table: Vec<f64>,
    /// Learned experience (counting updates it alongside the CPT).
    pub experience: Option<Vec<f64>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StructurePatch {
    pub nodes: Vec<NodePatch>,
}

/// Diff `after` against `before`. Both must share NodeIds (`after` is a
/// mutated clone of `before` — exactly what `run_structure_job` produces).
pub fn structure_patch(before: &Network, after: &Network) -> StructurePatch {
    let nodes = after
        .topo_order()
        .into_iter()
        .filter_map(|id| {
            let b = before.get(id)?;
            let a = after.node(id);
            if b.parents == a.parents && b.table == a.table && b.experience == a.experience {
                return None;
            }
            Some(NodePatch {
                name: a.name.clone(),
                parents: a.parents.iter().map(|&p| after.node(p).name.clone()).collect(),
                table: a.table.data.clone(),
                experience: a.experience.clone(),
            })
        })
        .collect();
    StructurePatch { nodes }
}

/// Apply a job's patch to the live session — the serialization-safe analogue
/// of `apply_structure_outcome`. Always clears the busy slot.
pub fn apply_structure_patch(
    s: &mut Session,
    patch: &StructurePatch,
    report: String,
    summary: String,
    warnings: Vec<String>,
    started_seq: u64,
) -> Result<StructureLearnResult, CmdError> {
    s.job_cancel = None;
    if s.doc.change_seq != started_seq {
        return Err(CmdError::Stale(
            "network was edited while learning ran — result discarded; run again".into(),
        ));
    }
    s.doc.begin_change();
    match apply_nodes(&mut s.doc.net, patch) {
        Ok(()) => {
            s.doc.ensure_visuals();
            s.finish(Dirt::Structure);
            Ok(StructureLearnResult { report, summary, warnings })
        }
        Err(e) => {
            s.doc.undo();
            Err(e)
        }
    }
}

fn apply_nodes(net: &mut Network, patch: &StructurePatch) -> Result<(), CmdError> {
    let find = |net: &Network, name: &str| {
        net.find_by_name(name)
            .ok_or_else(|| CmdError::BadRequest(format!("node `{name}` no longer exists")))
    };
    // Pass 1: detach every patched node from its old parents. Doing all
    // removals before any additions keeps the graph a subgraph of the final
    // (acyclic) DAG throughout, so no add can hit a spurious cycle error.
    for np in &patch.nodes {
        let id = find(net, &np.name)?;
        for p in net.node(id).parents.clone() {
            net.remove_edge(p, id)?;
        }
    }
    // Pass 2: re-add parents in learned order (add_edge appends, preserving
    // the CPT axis order), then overwrite the auto-reshaped table.
    for np in &patch.nodes {
        let id = find(net, &np.name)?;
        for pname in &np.parents {
            let pid = find(net, pname)?;
            net.add_edge(pid, id)?;
        }
        net.set_table(id, Table { data: np.table.clone() })?;
        net.set_experience(id, np.experience.clone())?;
    }
    Ok(())
}
