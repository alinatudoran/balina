//! Structure learning: search over DAGs among a fixed set of existing
//! chance nodes ("targets"), driven by case data.
//!
//! Learners own only the edges *between* targets — edges touching any
//! non-target node are preserved, and cycle checks account for paths
//! through them. None of the learners fit parameters: run
//! [`learn_counting`](crate::learn::learn_counting) or
//! [`learn_em`](crate::learn::learn_em) afterwards (Structural EM is the
//! exception — it fits parameters as part of its own loop).
//!
//! All entry points mutate the network only after the search has fully
//! succeeded, so cancellation ([`SearchCtrl`]) never leaves a half-edit.

pub mod ci;
pub mod dag;
pub mod gs;
pub mod hill_climb;
mod math;
pub mod mmhc;
pub mod nb_tan;
pub mod pc;
pub mod score;
pub mod sem;
mod stats;

pub use ci::CiOptions;
pub use gs::{grow_shrink_blankets, learn_gs, GsOptions, GsReport};
pub use hill_climb::{learn_hill_climb, HillClimbOptions, HillClimbReport};
pub use mmhc::{learn_mmhc, MmhcOptions, MmhcReport};
pub use nb_tan::{learn_naive_bayes, learn_tan, TanOptions, TanReport};
pub use pc::{learn_pc, Cpdag, PcOptions, PcReport};
pub use score::ScoreKind;
pub use sem::{learn_structural_em, SemOptions, SemReport};

use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::LearnError;
use crate::model::{Network, NodeId};

/// Background-knowledge constraints on the learned structure.
///
/// `required` edges must appear in the output DAG; `forbidden` edges must not.
/// Pairs where either node is not in the target set are silently ignored.
/// Call [`EdgeConstraints::validate`] before starting a search to catch
/// contradictions early.
#[derive(Clone, Debug, Default)]
pub struct EdgeConstraints {
    /// Edges that must be present in the learned graph (parent → child).
    pub required: Vec<(NodeId, NodeId)>,
    /// Edges that must not be present in the learned graph (parent → child).
    pub forbidden: Vec<(NodeId, NodeId)>,
}

impl EdgeConstraints {
    /// Check for self-loops, contradictory pairs, and cycles among required edges.
    pub fn validate(&self, net: &Network) -> Result<(), LearnError> {
        for &(p, c) in &self.required {
            if p == c {
                return Err(LearnError::BadConstraint(format!(
                    "self-loop on node `{}`",
                    net.node(p).name
                )));
            }
            if self.forbidden.contains(&(p, c)) {
                return Err(LearnError::BadConstraint(format!(
                    "`{}` → `{}` is both required and forbidden",
                    net.node(p).name,
                    net.node(c).name
                )));
            }
        }
        // Cycle check among required edges via Kahn's algorithm.
        let mut nodes: Vec<NodeId> =
            self.required.iter().flat_map(|&(p, c)| [p, c]).collect();
        nodes.sort();
        nodes.dedup();
        let n = nodes.len();
        let idx = |id: NodeId| nodes.iter().position(|&x| x == id).unwrap();
        let mut adj = vec![vec![false; n]; n];
        for &(p, c) in &self.required {
            adj[idx(p)][idx(c)] = true;
        }
        let mut indegree: Vec<usize> =
            (0..n).map(|i| (0..n).filter(|&j| adj[j][i]).count()).collect();
        let mut queue: Vec<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
        let mut processed = 0;
        while let Some(u) = queue.pop() {
            processed += 1;
            for v in 0..n {
                if adj[u][v] {
                    indegree[v] -= 1;
                    if indegree[v] == 0 {
                        queue.push(v);
                    }
                }
            }
        }
        if processed < n {
            return Err(LearnError::BadConstraint(
                "required edges contain a directed cycle".into(),
            ));
        }
        Ok(())
    }

    /// Project to index-space pairs over `targets`.
    /// Any pair whose parent or child is not in `targets` is dropped.
    pub(crate) fn to_ix(&self, targets: &[NodeId]) -> (Vec<(u32, u32)>, Vec<(u32, u32)>) {
        let ix = |id: NodeId| -> Option<u32> {
            targets.iter().position(|&t| t == id).map(|i| i as u32)
        };
        let project = |v: &[(NodeId, NodeId)]| -> Vec<(u32, u32)> {
            v.iter().filter_map(|&(p, c)| Some((ix(p)?, ix(c)?))).collect()
        };
        (project(&self.required), project(&self.forbidden))
    }
}

/// A progress tick from a long-running learner.
#[derive(Clone, Debug)]
pub struct Progress {
    pub phase: &'static str,
    pub done: usize,
    /// 0 = unknown.
    pub total: usize,
    pub score: Option<f64>,
}

/// GUI-free control handle: an optional progress callback plus an optional
/// cancel flag. The default (`SearchCtrl::default()`) reports nothing and
/// never cancels.
#[derive(Default, Clone, Copy)]
pub struct SearchCtrl<'a> {
    pub progress: Option<&'a dyn Fn(Progress)>,
    pub cancel: Option<&'a AtomicBool>,
}

impl SearchCtrl<'_> {
    pub(crate) fn tick(&self, p: Progress) {
        if let Some(f) = self.progress {
            f(p);
        }
    }
    pub(crate) fn check(&self) -> Result<(), LearnError> {
        match self.cancel {
            Some(c) if c.load(Ordering::Relaxed) => Err(LearnError::Cancelled),
            _ => Ok(()),
        }
    }
}

/// Validate a target list and return it (deduped, order preserved).
pub(crate) fn check_targets(
    targets: &[NodeId],
    min: usize,
) -> Result<Vec<NodeId>, LearnError> {
    let mut out: Vec<NodeId> = Vec::with_capacity(targets.len());
    for &t in targets {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    if out.len() < min {
        return Err(LearnError::TooFewVariables { min, got: out.len() });
    }
    Ok(out)
}

/// Replace all target-target edges with `edges` (indices into `targets`).
/// Goes through `Network` methods so CPTs auto-reshape and acyclicity
/// against the rest of the graph is rechecked.
pub(crate) fn apply_edges(
    net: &mut Network,
    targets: &[NodeId],
    edges: &[(u32, u32)],
) -> Result<(), LearnError> {
    for (p, c) in net.edges() {
        if targets.contains(&p) && targets.contains(&c) {
            net.remove_edge(p, c)?;
        }
    }
    let mut sorted = edges.to_vec();
    sorted.sort_unstable();
    for &(p, c) in &sorted {
        net.add_edge(targets[p as usize], targets[c as usize])?;
    }
    Ok(())
}

/// All chance nodes of a network, in name order — the default target set.
pub fn default_targets(net: &Network) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = net
        .nodes()
        .filter(|(_, n)| n.kind == crate::model::NodeKind::Chance)
        .map(|(id, _)| id)
        .collect();
    out.sort_by(|&a, &b| net.node(a).name.cmp(&net.node(b).name));
    out
}
