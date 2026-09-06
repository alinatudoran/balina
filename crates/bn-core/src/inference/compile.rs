//! Network compilation: variable indexing, moralization, min-fill
//! triangulation, and junction tree construction.

use slotmap::SecondaryMap;
use std::collections::HashSet;

use crate::factor::{Factor, VarId};
use crate::model::{Network, NodeId, NodeKind};

/// The network lowered onto dense variable indices (chance + decision nodes;
/// utility nodes carry no variable).
pub struct CompiledNet {
    /// Topological order; `VarId(i)` is `order[i]`.
    pub order: Vec<NodeId>,
    pub var_of: SecondaryMap<NodeId, VarId>,
    pub cards: Vec<usize>,
    /// Per variable: its CPT as a factor over `[parents..., self]`
    /// (decision nodes: uniform over `[self]`).
    pub families: Vec<Factor>,
    /// Per variable: the family variable set (self + chance-relevant parents).
    pub family_vars: Vec<Vec<VarId>>,
}

impl CompiledNet {
    pub fn compile(net: &Network) -> CompiledNet {
        let order: Vec<NodeId> = net
            .topo_order()
            .into_iter()
            .filter(|&id| net.node(id).kind != NodeKind::Utility)
            .collect();
        let mut var_of = SecondaryMap::new();
        for (i, &id) in order.iter().enumerate() {
            var_of.insert(id, VarId(i as u32));
        }
        let cards: Vec<usize> = order.iter().map(|&id| net.node(id).n_states()).collect();
        let mut families = Vec::with_capacity(order.len());
        let mut family_vars = Vec::with_capacity(order.len());
        for &id in &order {
            let node = net.node(id);
            let me = var_of[id];
            match node.kind {
                NodeKind::Chance => {
                    let mut axes: Vec<VarId> = node.parents.iter().map(|&p| var_of[p]).collect();
                    let mut axcards: Vec<usize> =
                        node.parents.iter().map(|&p| net.node(p).n_states()).collect();
                    axes.push(me);
                    axcards.push(node.n_states());
                    let f = Factor::from_axes(&axes, &axcards, &node.table.data);
                    let mut fam = axes.clone();
                    fam.sort();
                    families.push(f);
                    family_vars.push(fam);
                }
                NodeKind::Decision => {
                    let k = node.n_states();
                    families.push(Factor::new(vec![me], vec![k], vec![1.0 / k as f64; k]));
                    family_vars.push(vec![me]);
                }
                NodeKind::Utility => unreachable!(),
            }
        }
        CompiledNet { order, var_of, cards, families, family_vars }
    }

    pub fn n_vars(&self) -> usize {
        self.order.len()
    }
}

pub struct JunctionTree {
    /// Sorted variable lists of each clique.
    pub cliques: Vec<Vec<VarId>>,
    /// Edges of the tree: (clique_a, clique_b, sepset vars).
    pub edges: Vec<(usize, usize, Vec<VarId>)>,
    /// Adjacency: per clique, (neighbor clique, edge index).
    pub neighbors: Vec<Vec<(usize, usize)>>,
    /// Per variable: clique its family factor is assigned to.
    pub home_of_family: Vec<usize>,
    /// Per variable: smallest clique containing it (belief queries).
    pub belief_clique: Vec<usize>,
}

impl JunctionTree {
    pub fn build(net: &Network, cn: &CompiledNet) -> JunctionTree {
        let n = cn.n_vars();
        // ---- moral graph -------------------------------------------------
        let mut adj: Vec<HashSet<usize>> = vec![HashSet::new(); n];
        let connect = |adj: &mut Vec<HashSet<usize>>, a: usize, b: usize| {
            if a != b {
                adj[a].insert(b);
                adj[b].insert(a);
            }
        };
        for &id in &cn.order {
            let node = net.node(id);
            let me = cn.var_of[id].0 as usize;
            let pars: Vec<usize> =
                node.parents.iter().map(|&p| cn.var_of[p].0 as usize).collect();
            match node.kind {
                NodeKind::Chance => {
                    // Marry all family members pairwise.
                    let fam: Vec<usize> = pars.iter().copied().chain([me]).collect();
                    for i in 0..fam.len() {
                        for j in i + 1..fam.len() {
                            connect(&mut adj, fam[i], fam[j]);
                        }
                    }
                }
                NodeKind::Decision => {
                    // Informational links only; the decision's factor is over
                    // itself, so parents need not be married.
                    for &p in &pars {
                        connect(&mut adj, me, p);
                    }
                }
                NodeKind::Utility => {}
            }
        }
        // Marry the parents of every utility node so each utility family
        // lands inside one clique (needed for expected-utility queries).
        for (_id, node) in net.nodes() {
            if node.kind == NodeKind::Utility {
                let pars: Vec<usize> =
                    node.parents.iter().map(|&p| cn.var_of[p].0 as usize).collect();
                for i in 0..pars.len() {
                    for j in i + 1..pars.len() {
                        connect(&mut adj, pars[i], pars[j]);
                    }
                }
            }
        }

        // ---- min-fill triangulation ---------------------------------------
        let mut remaining: HashSet<usize> = (0..n).collect();
        let mut elim_cliques: Vec<Vec<usize>> = Vec::new();
        while !remaining.is_empty() {
            // Pick vertex with fewest fill-in edges; tie-break on clique weight.
            let mut best: Option<(usize, usize, f64)> = None; // (v, fill, weight)
            for &v in &remaining {
                let nbrs: Vec<usize> =
                    adj[v].iter().copied().filter(|u| remaining.contains(u)).collect();
                let mut fill = 0usize;
                for i in 0..nbrs.len() {
                    for j in i + 1..nbrs.len() {
                        if !adj[nbrs[i]].contains(&nbrs[j]) {
                            fill += 1;
                        }
                    }
                }
                let weight: f64 = nbrs
                    .iter()
                    .map(|&u| cn.cards[u] as f64)
                    .product::<f64>()
                    * cn.cards[v] as f64;
                let better = match best {
                    None => true,
                    Some((_, bf, bw)) => fill < bf || (fill == bf && weight < bw),
                };
                if better {
                    best = Some((v, fill, weight));
                }
            }
            let (v, _, _) = best.unwrap();
            let nbrs: Vec<usize> =
                adj[v].iter().copied().filter(|u| remaining.contains(u)).collect();
            for i in 0..nbrs.len() {
                for j in i + 1..nbrs.len() {
                    connect(&mut adj, nbrs[i], nbrs[j]);
                }
            }
            let mut clique = nbrs;
            clique.push(v);
            clique.sort_unstable();
            elim_cliques.push(clique);
            remaining.remove(&v);
        }

        // ---- maximal cliques ----------------------------------------------
        let mut cliques: Vec<Vec<usize>> = Vec::new();
        'outer: for c in elim_cliques {
            for other in &cliques {
                if c.iter().all(|v| other.contains(v)) {
                    continue 'outer;
                }
            }
            cliques.retain(|other| !other.iter().all(|v| c.contains(v)));
            cliques.push(c);
        }
        if cliques.is_empty() {
            cliques.push(vec![]); // degenerate empty network
        }

        // ---- maximum-weight spanning tree over sepset sizes ----------------
        let mut cand: Vec<(usize, usize, usize)> = Vec::new(); // (weight, i, j)
        for i in 0..cliques.len() {
            for j in i + 1..cliques.len() {
                let w = cliques[i].iter().filter(|v| cliques[j].contains(v)).count();
                cand.push((w, i, j));
            }
        }
        cand.sort_by(|a, b| b.0.cmp(&a.0));
        let mut uf: Vec<usize> = (0..cliques.len()).collect();
        fn find(uf: &mut Vec<usize>, x: usize) -> usize {
            if uf[x] != x {
                let r = find(uf, uf[x]);
                uf[x] = r;
            }
            uf[x]
        }
        let mut edges: Vec<(usize, usize, Vec<VarId>)> = Vec::new();
        let mut neighbors: Vec<Vec<(usize, usize)>> = vec![vec![]; cliques.len()];
        for (_, i, j) in cand {
            let (ri, rj) = (find(&mut uf, i), find(&mut uf, j));
            if ri != rj {
                uf[ri] = rj;
                let sep: Vec<VarId> = cliques[i]
                    .iter()
                    .filter(|v| cliques[j].contains(v))
                    .map(|&v| VarId(v as u32))
                    .collect();
                let e = edges.len();
                neighbors[i].push((j, e));
                neighbors[j].push((i, e));
                edges.push((i, j, sep));
            }
        }

        // ---- assignments ----------------------------------------------------
        let weight_of = |c: &[usize]| -> f64 { c.iter().map(|&v| cn.cards[v] as f64).product() };
        let smallest_containing = |vars: &[VarId]| -> usize {
            let mut best = None;
            for (ci, c) in cliques.iter().enumerate() {
                if vars.iter().all(|v| c.contains(&(v.0 as usize))) {
                    let w = weight_of(c);
                    if best.map_or(true, |(_, bw)| w < bw) {
                        best = Some((ci, w));
                    }
                }
            }
            best.expect("family must be contained in some clique").0
        };
        let home_of_family: Vec<usize> =
            (0..n).map(|v| smallest_containing(&cn.family_vars[v])).collect();
        let belief_clique: Vec<usize> =
            (0..n).map(|v| smallest_containing(&[VarId(v as u32)])).collect();

        JunctionTree {
            cliques: cliques
                .into_iter()
                .map(|c| c.into_iter().map(|v| VarId(v as u32)).collect())
                .collect(),
            edges,
            neighbors,
            home_of_family,
            belief_clique,
        }
    }

    /// Smallest clique containing all of `vars`, if any.
    pub fn clique_containing(&self, vars: &[VarId], cards: &[usize]) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for (ci, c) in self.cliques.iter().enumerate() {
            if vars.iter().all(|v| c.contains(v)) {
                let w: f64 = c.iter().map(|v| cards[v.0 as usize] as f64).product();
                if best.map_or(true, |(_, bw)| w < bw) {
                    best = Some((ci, w));
                }
            }
        }
        best.map(|(ci, _)| ci)
    }
}
