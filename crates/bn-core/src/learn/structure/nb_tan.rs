//! Classifier structures: Naive Bayes and Tree-Augmented Naive Bayes
//! (Friedman, Geiger & Goldszmidt 1997). Both take a designated class node;
//! parameters are fitted separately (counting/EM).

use crate::error::LearnError;
use crate::learn::cases::CaseSet;
use crate::learn::structure::ci::{CiOptions, CiTester};
use crate::learn::structure::stats::DataView;
use crate::learn::structure::{apply_edges, check_targets, Progress, SearchCtrl};
use crate::model::{Network, NodeId};

/// Make every non-class target a child of `class` (star graph). Needs no
/// case data.
pub fn learn_naive_bayes(
    net: &mut Network,
    targets: &[NodeId],
    class: NodeId,
) -> Result<(), LearnError> {
    let targets = check_targets(targets, 2)?;
    let ci = class_index(net, &targets, class)?;
    let edges: Vec<(u32, u32)> = (0..targets.len() as u32).filter(|&i| i != ci).map(|i| (ci, i)).collect();
    apply_edges(net, &targets, &edges)
}

#[derive(Clone, Debug, Default)]
pub struct TanOptions {
    /// Root of the feature tree; default: the first non-class target.
    pub root: Option<NodeId>,
}

#[derive(Clone, Debug, Default)]
pub struct TanReport {
    /// Directed feature-tree edges (excludes the class→feature edges).
    pub tree_edges: Vec<(NodeId, NodeId)>,
    /// Conditional mutual information of each tree edge given the class.
    pub cmi: Vec<f64>,
}

/// Tree-Augmented Naive Bayes: class parent of every feature, plus a
/// maximum-spanning-tree over pairwise I(Xi; Xj | class).
pub fn learn_tan(
    net: &mut Network,
    cases: &CaseSet,
    targets: &[NodeId],
    class: NodeId,
    opts: &TanOptions,
    ctrl: &SearchCtrl,
) -> Result<TanReport, LearnError> {
    let targets = check_targets(targets, 2)?;
    let ci = class_index(net, &targets, class)?;
    let view = DataView::new(net, cases, &targets)?;
    let features: Vec<u32> = (0..targets.len() as u32).filter(|&i| i != ci).collect();

    // Pairwise conditional MI given the class.
    let mut tester = CiTester::new(&view, CiOptions::default());
    let n_pairs = features.len() * (features.len().saturating_sub(1)) / 2;
    let mut weighted: Vec<(f64, u32, u32)> = Vec::with_capacity(n_pairs);
    let mut done = 0;
    for (a, &i) in features.iter().enumerate() {
        for &j in &features[a + 1..] {
            ctrl.check()?;
            let (w, _) = tester.cmi(i, j, &[ci])?;
            weighted.push((w, i, j));
            done += 1;
            ctrl.tick(Progress { phase: "pairwise CMI", done, total: n_pairs, score: None });
        }
    }
    // Kruskal maximum spanning tree; deterministic tie-break on indices.
    weighted.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let n = targets.len();
    let mut comp: Vec<usize> = (0..n).collect();
    fn find(comp: &mut Vec<usize>, x: usize) -> usize {
        if comp[x] != x {
            let r = find(comp, comp[x]);
            comp[x] = r;
        }
        comp[x]
    }
    let mut mst: Vec<(u32, u32, f64)> = vec![];
    for &(w, i, j) in &weighted {
        let (ri, rj) = (find(&mut comp, i as usize), find(&mut comp, j as usize));
        if ri != rj {
            comp[ri] = rj;
            mst.push((i, j, w));
        }
    }
    // Direct edges away from the root by BFS over the undirected tree.
    // Root of the feature tree; the class (or an unknown node) falls back
    // to the first feature.
    let root = opts
        .root
        .and_then(|r| targets.iter().position(|&t| t == r))
        .map(|i| i as u32)
        .filter(|&i| i != ci)
        .unwrap_or(features[0]);
    let mut edges: Vec<(u32, u32)> = features.iter().map(|&f| (ci, f)).collect();
    let mut report = TanReport::default();
    let mut visited = vec![false; n];
    visited[root as usize] = true;
    let mut queue = vec![root];
    while let Some(u) = queue.pop() {
        for &(i, j, w) in &mst {
            let v = if i == u && !visited[j as usize] {
                j
            } else if j == u && !visited[i as usize] {
                i
            } else {
                continue;
            };
            visited[v as usize] = true;
            edges.push((u, v));
            report.tree_edges.push((targets[u as usize], targets[v as usize]));
            report.cmi.push(w);
            queue.push(v);
        }
    }
    apply_edges(net, &targets, &edges)?;
    Ok(report)
}

/// Index of `class` within targets (validated present).
fn class_index(net: &Network, targets: &[NodeId], class: NodeId) -> Result<u32, LearnError> {
    targets
        .iter()
        .position(|&t| t == class)
        .map(|i| i as u32)
        .ok_or_else(|| LearnError::NoDataColumn(net.node(class).name.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NodeKind, State};

    #[test]
    fn naive_bayes_star_shape() {
        let mut net = Network::new("t");
        let two = || vec![State::new("a"), State::new("b")];
        let c = net.add_node("Class", NodeKind::Chance, two()).unwrap();
        let f1 = net.add_node("F1", NodeKind::Chance, two()).unwrap();
        let f2 = net.add_node("F2", NodeKind::Chance, two()).unwrap();
        let f3 = net.add_node("F3", NodeKind::Chance, two()).unwrap();
        // Pre-existing wrong edge gets rewired.
        net.add_edge(f1, f2).unwrap();
        learn_naive_bayes(&mut net, &[c, f1, f2, f3], c).unwrap();
        for &f in &[f1, f2, f3] {
            assert_eq!(net.node(f).parents, vec![c]);
        }
        assert!(net.node(c).parents.is_empty());
    }
}
