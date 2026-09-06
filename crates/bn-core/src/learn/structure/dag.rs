//! Scratch DAG used during search, in the compact target index space.
//!
//! Besides the learned edges it carries "external" arcs: target→target
//! reachability through non-target nodes in the surrounding network, so a
//! learned edge can never close a cycle through parts of the graph the
//! learner does not own.

use crate::model::{Network, NodeId};

#[derive(Clone, PartialEq)]
pub(crate) struct WorkDag {
    pub parents: Vec<Vec<u32>>, // sorted ascending
    pub children: Vec<Vec<u32>>,
    /// ext[i] contains j when target i reaches target j through ≥1
    /// non-target node (direct target-target edges excluded).
    ext: Vec<Vec<u32>>,
}

impl WorkDag {
    /// Build from the network's current target-target edges, with external
    /// arcs precomputed.
    pub fn from_network(net: &Network, targets: &[NodeId]) -> WorkDag {
        let n = targets.len();
        let idx_of = |id: NodeId| targets.iter().position(|&t| t == id);
        let mut dag = WorkDag {
            parents: vec![vec![]; n],
            children: vec![vec![]; n],
            ext: vec![vec![]; n],
        };
        for (ci, &c) in targets.iter().enumerate() {
            for &p in &net.node(c).parents {
                if let Some(pi) = idx_of(p) {
                    dag.parents[ci].push(pi as u32);
                    dag.children[pi].push(ci as u32);
                }
            }
            dag.parents[ci].sort_unstable();
        }
        // External reachability: DFS from each target over the network with
        // direct target-target edges removed; record reached targets.
        for (i, &start) in targets.iter().enumerate() {
            let mut stack: Vec<NodeId> = net
                .children(start)
                .iter()
                .copied()
                .filter(|&c| idx_of(c).is_none())
                .collect();
            let mut seen: Vec<NodeId> = stack.clone();
            while let Some(node) = stack.pop() {
                if let Some(j) = idx_of(node) {
                    if j != i && !dag.ext[i].contains(&(j as u32)) {
                        dag.ext[i].push(j as u32);
                    }
                    continue; // stop at targets: onward paths are their own arcs
                }
                for &c in net.children(node) {
                    if !seen.contains(&c) {
                        seen.push(c);
                        stack.push(c);
                    }
                }
            }
            dag.ext[i].sort_unstable();
        }
        dag
    }

    /// Same external arcs, no learned edges.
    pub fn cleared(&self) -> WorkDag {
        WorkDag {
            parents: vec![vec![]; self.parents.len()],
            children: vec![vec![]; self.parents.len()],
            ext: self.ext.clone(),
        }
    }

    pub fn n(&self) -> usize {
        self.parents.len()
    }

    pub fn has_edge(&self, p: u32, c: u32) -> bool {
        self.parents[c as usize].binary_search(&p).is_ok()
    }

    pub fn add(&mut self, p: u32, c: u32) {
        debug_assert!(!self.has_edge(p, c) && !self.creates_cycle(p, c));
        let pa = &mut self.parents[c as usize];
        pa.insert(pa.binary_search(&p).unwrap_err(), p);
        self.children[p as usize].push(c);
    }

    pub fn remove(&mut self, p: u32, c: u32) {
        let pa = &mut self.parents[c as usize];
        let pos = pa.binary_search(&p).expect("edge must exist");
        pa.remove(pos);
        self.children[p as usize].retain(|&x| x != c);
    }

    /// Would adding p→c close a cycle through learned edges ∪ external arcs?
    pub fn creates_cycle(&self, p: u32, c: u32) -> bool {
        if p == c {
            return true;
        }
        // DFS from c looking for p.
        let mut stack = vec![c];
        let mut seen = vec![false; self.n()];
        seen[c as usize] = true;
        while let Some(x) = stack.pop() {
            if x == p {
                return true;
            }
            for &y in self.children[x as usize].iter().chain(&self.ext[x as usize]) {
                if !seen[y as usize] {
                    seen[y as usize] = true;
                    stack.push(y);
                }
            }
        }
        false
    }

    /// All learned edges, sorted (parent, child).
    pub fn edges(&self) -> Vec<(u32, u32)> {
        let mut out = vec![];
        for (c, pa) in self.parents.iter().enumerate() {
            for &p in pa {
                out.push((p, c as u32));
            }
        }
        out.sort_unstable();
        out
    }
}

/// The CPDAG (compelled + reversible edges) of a set of directed edges over
/// `n` variables: skeleton + v-structures, closed under Meek rules R1–R3.
/// Used to compare learned structures up to Markov equivalence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpdagIx {
    /// Compelled edges (directed), sorted.
    pub directed: Vec<(u32, u32)>,
    /// Reversible edges as (min, max), sorted.
    pub undirected: Vec<(u32, u32)>,
}

pub fn dag_to_cpdag(edges: &[(u32, u32)], n: usize) -> CpdagIx {
    let mut parents = vec![Vec::<u32>::new(); n];
    let mut adj = vec![vec![false; n]; n];
    for &(p, c) in edges {
        parents[c as usize].push(p);
        adj[p as usize][c as usize] = true;
        adj[c as usize][p as usize] = true;
    }
    // Start with everything undirected, then compel v-structures: for each
    // unmarried parent pair of a common child, both incoming edges.
    let mut dir = vec![vec![false; n]; n]; // dir[a][b]: compelled a→b
    for c in 0..n {
        let pa = &parents[c];
        for i in 0..pa.len() {
            for j in i + 1..pa.len() {
                let (a, b) = (pa[i] as usize, pa[j] as usize);
                if !adj[a][b] {
                    dir[a][c] = true;
                    dir[b][c] = true;
                }
            }
        }
    }
    meek_closure(&adj, &mut dir);
    let mut directed = vec![];
    let mut undirected = vec![];
    for a in 0..n {
        for b in 0..n {
            if a < b && adj[a][b] {
                match (dir[a][b], dir[b][a]) {
                    (true, false) => directed.push((a as u32, b as u32)),
                    (false, true) => directed.push((b as u32, a as u32)),
                    _ => undirected.push((a as u32, b as u32)),
                }
            }
        }
    }
    directed.sort_unstable();
    undirected.sort_unstable();
    CpdagIx { directed, undirected }
}

/// Meek orientation rules R1–R3 applied to fixpoint. `adj` is the symmetric
/// skeleton; `dir[a][b]` marks a compelled a→b (an edge with neither
/// direction set is undirected). R4 is only needed with background-knowledge
/// orientations (Meek 1995), which we never have.
pub(crate) fn meek_closure(adj: &[Vec<bool>], dir: &mut [Vec<bool>]) {
    let n = adj.len();
    let undirected =
        |dir: &[Vec<bool>], a: usize, b: usize| adj[a][b] && !dir[a][b] && !dir[b][a];
    loop {
        let mut changed = false;
        for b in 0..n {
            for c in 0..n {
                if !undirected(dir, b, c) {
                    continue;
                }
                // R1: a→b, b−c, a and c nonadjacent  ⇒  b→c.
                let r1 = (0..n).any(|a| dir[a][b] && !dir[b][a] && !adj[a][c] && a != c);
                // R2: b→a→c and b−c  ⇒  b→c.
                let r2 = (0..n).any(|a| dir[b][a] && dir[a][c]);
                // R3: b−a, b−d, a→c, d→c, b−c, a and d nonadjacent  ⇒  b→c.
                let r3 = (0..n).any(|a| {
                    undirected(dir, b, a)
                        && dir[a][c]
                        && (0..n).any(|d| {
                            d != a && undirected(dir, b, d) && dir[d][c] && !adj[a][d]
                        })
                });
                if r1 || r2 || r3 {
                    dir[b][c] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Network, NodeKind, State};

    fn two() -> Vec<State> {
        vec![State::new("a"), State::new("b")]
    }

    #[test]
    fn external_path_blocks_cycle() {
        // A → M → B with M not a target: learned B→A would close a cycle.
        let mut net = Network::new("t");
        let a = net.add_node("A", NodeKind::Chance, two()).unwrap();
        let m = net.add_node("M", NodeKind::Chance, two()).unwrap();
        let b = net.add_node("B", NodeKind::Chance, two()).unwrap();
        net.add_edge(a, m).unwrap();
        net.add_edge(m, b).unwrap();
        let dag = WorkDag::from_network(&net, &[a, b]);
        assert!(dag.edges().is_empty()); // no direct target-target edge
        assert!(dag.creates_cycle(1, 0)); // B→A forbidden via M
        assert!(!dag.creates_cycle(0, 1)); // A→B fine
    }

    #[test]
    fn cpdag_of_sprinkler_shape() {
        // C→S, C→R, S→W, R→W: the v-structure at W is compelled; the C
        // edges are reversible.
        let edges = [(0u32, 1u32), (0, 2), (1, 3), (2, 3)];
        let c = dag_to_cpdag(&edges, 4);
        assert_eq!(c.directed, vec![(1, 3), (2, 3)]);
        assert_eq!(c.undirected, vec![(0, 1), (0, 2)]);
    }

    #[test]
    fn cpdag_chain_fully_reversible() {
        // A→B→C has no v-structure: everything reversible.
        let c = dag_to_cpdag(&[(0, 1), (1, 2)], 3);
        assert!(c.directed.is_empty());
        assert_eq!(c.undirected, vec![(0, 1), (1, 2)]);
    }
}
