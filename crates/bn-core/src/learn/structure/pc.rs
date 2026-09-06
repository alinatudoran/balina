//! PC-stable (Colombo & Maathuis 2014): constraint-based causal discovery.
//! Skeleton via level-wise CI tests with per-level neighborhood snapshots
//! (order-independent), v-structure orientation, Meek rules R1–R3, and a
//! Dor–Tarsi (1992) consistent extension to pick the DAG written into the
//! network. The report keeps the CPDAG so callers can tell data-identified
//! directions from arbitrary ones.

use std::collections::HashMap;

use crate::error::LearnError;
use crate::learn::cases::CaseSet;
use crate::learn::structure::ci::{CiOptions, CiTester};
use crate::learn::structure::dag::meek_closure;
use crate::learn::structure::stats::DataView;
use crate::learn::structure::{apply_edges, check_targets, EdgeConstraints, Progress, SearchCtrl};
use crate::model::{Network, NodeId};

#[derive(Clone, Debug, Default)]
pub struct PcOptions {
    pub ci: CiOptions,
    /// Hard structural constraints (required / forbidden edges).
    pub constraints: EdgeConstraints,
}

/// A partially directed graph up to Markov equivalence, in `NodeId` terms.
#[derive(Clone, Debug, Default)]
pub struct Cpdag {
    /// Compelled (data-identified) directed edges.
    pub directed: Vec<(NodeId, NodeId)>,
    /// Reversible edges — direction chosen arbitrarily in the network.
    pub undirected: Vec<(NodeId, NodeId)>,
}

#[derive(Clone, Debug, Default)]
pub struct PcReport {
    pub cpdag: Cpdag,
    pub ci_tests: usize,
    /// Edges the consistent extension had to orient by fallback because the
    /// CPDAG was not extendable (> 0 signals CI-test inconsistencies).
    pub forced_orientations: usize,
    /// Conflicting v-structure orientations left as first-come (same signal).
    pub v_structure_conflicts: usize,
}

pub fn learn_pc(
    net: &mut Network,
    cases: &CaseSet,
    targets: &[NodeId],
    opts: &PcOptions,
    ctrl: &SearchCtrl,
) -> Result<PcReport, LearnError> {
    opts.constraints.validate(net)?;
    let targets = check_targets(targets, 2)?;
    let (req, forb) = opts.constraints.to_ix(&targets);
    let view = DataView::new(net, cases, &targets)?;
    let n = targets.len();
    let mut tester = CiTester::new(&view, opts.ci.clone());
    let mut report = PcReport::default();

    // ---- skeleton (stable) ------------------------------------------------
    let mut adj = vec![vec![false; n]; n];
    for i in 0..n {
        for j in 0..n {
            adj[i][j] = i != j;
        }
    }
    // Apply blacklist before skeleton: forbidden pairs cannot be adjacent.
    for &(p, c) in &forb {
        adj[p as usize][c as usize] = false;
        adj[c as usize][p as usize] = false;
    }
    let mut sepset: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for level in 0..=opts.ci.max_cond {
        ctrl.check()?;
        ctrl.tick(Progress { phase: "skeleton", done: level, total: opts.ci.max_cond + 1, score: None });
        let snapshot = adj.clone(); // all level-l tests see the same graph
        let mut any_candidate = false;
        for x in 0..n {
            for y in x + 1..n {
                if !adj[x][y] {
                    continue;
                }
                // Neighbors of x (in the snapshot) excluding y, then of y
                // excluding x — PC tests conditioning sets from both sides.
                let mut separated = false;
                for (from, other) in [(x, y), (y, x)] {
                    let nbrs: Vec<u32> = (0..n as u32)
                        .filter(|&k| k as usize != other && snapshot[from][k as usize])
                        .collect();
                    if nbrs.len() < level {
                        continue;
                    }
                    any_candidate = true;
                    let mut subsets = KSubsets::new(nbrs.len(), level);
                    while let Some(pick) = subsets.next() {
                        ctrl.check()?;
                        let cond: Vec<u32> = pick.iter().map(|&i| nbrs[i]).collect();
                        let (ind, _) = tester.independent(x as u32, y as u32, &cond)?;
                        if ind {
                            sepset.insert((x as u32, y as u32), cond);
                            separated = true;
                            break;
                        }
                    }
                    if separated {
                        break;
                    }
                }
                if separated {
                    adj[x][y] = false;
                    adj[y][x] = false;
                }
            }
        }
        if !any_candidate {
            break; // no pair has enough neighbors for larger sets
        }
    }

    // Apply whitelist after skeleton: required pairs must be adjacent, and
    // they are protected from CI-based removal. Remove any stale sepset entry
    // so the v-structure phase treats required pairs as connected.
    for &(p, c) in &req {
        adj[p as usize][c as usize] = true;
        adj[c as usize][p as usize] = true;
        sepset.remove(&(p, c));
        sepset.remove(&(c, p));
    }

    // ---- v-structures -----------------------------------------------------
    // For nonadjacent x, y with common neighbor z: orient x→z←y when z is
    // not in sepset(x, y). Conflicts with an earlier orientation are counted
    // and left as-is (scan order is deterministic).
    let mut dir = vec![vec![false; n]; n];
    for x in 0..n {
        for y in x + 1..n {
            if adj[x][y] {
                continue;
            }
            let empty = vec![];
            let sep = sepset.get(&(x as u32, y as u32)).unwrap_or(&empty);
            for z in 0..n {
                if z == x || z == y || !adj[x][z] || !adj[y][z] || sep.contains(&(z as u32)) {
                    continue;
                }
                for a in [x, y] {
                    if dir[z][a] {
                        report.v_structure_conflicts += 1;
                    } else {
                        dir[a][z] = true;
                    }
                }
            }
        }
    }

    // Force required edge directions (override any CI-derived orientation).
    for &(p, c) in &req {
        dir[p as usize][c as usize] = true;
        dir[c as usize][p as usize] = false;
    }

    // ---- Meek rules + record the CPDAG -------------------------------------
    meek_closure(&adj, &mut dir);
    for a in 0..n {
        for b in a + 1..n {
            if !adj[a][b] {
                continue;
            }
            match (dir[a][b], dir[b][a]) {
                (true, false) => report.cpdag.directed.push((targets[a], targets[b])),
                (false, true) => report.cpdag.directed.push((targets[b], targets[a])),
                _ => report.cpdag.undirected.push((targets[a], targets[b])),
            }
        }
    }
    report.ci_tests = tester.tests_run;

    // ---- consistent extension (Dor & Tarsi 1992) ----------------------------
    ctrl.tick(Progress { phase: "orienting", done: 0, total: 0, score: None });
    let g_adj = adj.clone();
    let mut g_dir = dir.clone();
    let mut alive: Vec<bool> = vec![true; n];
    let mut remaining = n;
    while remaining > 0 {
        let mut sink = None;
        'search: for x in 0..n {
            if !alive[x] {
                continue;
            }
            // x must have no outgoing directed edge among alive nodes…
            for y in 0..n {
                if alive[y] && g_dir[x][y] && !g_dir[y][x] {
                    continue 'search;
                }
            }
            // …and every undirected neighbor of x must be adjacent to all
            // other neighbors of x.
            let nbrs: Vec<usize> =
                (0..n).filter(|&y| alive[y] && g_adj[x][y]).collect();
            for &y in &nbrs {
                if g_dir[x][y] || g_dir[y][x] {
                    continue; // only undirected neighbors need the clique check
                }
                for &w in &nbrs {
                    if w != y && !g_adj[y][w] {
                        continue 'search;
                    }
                }
            }
            sink = Some(x);
            break;
        }
        match sink {
            Some(x) => {
                for y in 0..n {
                    if alive[y] && g_adj[x][y] && !g_dir[x][y] && !g_dir[y][x] {
                        g_dir[y][x] = true;
                        dir[y][x] = true; // adopt the orientation in the output
                    }
                }
                alive[x] = false;
                remaining -= 1;
            }
            None => {
                // Not extendable (inconsistent CI answers): force one
                // undirected edge low-index → high-index unless that closes
                // a directed cycle, then resume.
                let mut forced = false;
                'force: for a in 0..n {
                    for b in a + 1..n {
                        if alive[a] && alive[b] && g_adj[a][b] && !g_dir[a][b] && !g_dir[b][a] {
                            let (p, c) = if directed_reaches(&g_dir, b, a, n) {
                                (b, a) // a→b would close a cycle
                            } else {
                                (a, b)
                            };
                            g_dir[p][c] = true;
                            dir[p][c] = true;
                            report.forced_orientations += 1;
                            forced = true;
                            break 'force;
                        }
                    }
                }
                if !forced {
                    // Only directed edges left yet no sink: directed cycle
                    // from inconsistent tests. Drop orientation of one edge.
                    debug_assert!(false, "directed cycle in extension");
                    break;
                }
            }
        }
    }

    // ---- write the DAG into the network -------------------------------------
    let mut edges: Vec<(u32, u32)> = vec![];
    for a in 0..n {
        for b in 0..n {
            if adj[a][b] && dir[a][b] && !dir[b][a] {
                edges.push((a as u32, b as u32));
            }
        }
    }
    apply_edges(net, &targets, &edges)?;
    Ok(report)
}

fn directed_reaches(dir: &[Vec<bool>], from: usize, to: usize, n: usize) -> bool {
    let mut stack = vec![from];
    let mut seen = vec![false; n];
    seen[from] = true;
    while let Some(x) = stack.pop() {
        if x == to {
            return true;
        }
        for y in 0..n {
            if dir[x][y] && !dir[y][x] && !seen[y] {
                seen[y] = true;
                stack.push(y);
            }
        }
    }
    false
}

/// Lexicographic k-subset iterator over 0..n (indices into a slice).
struct KSubsets {
    n: usize,
    k: usize,
    cur: Vec<usize>,
    started: bool,
}

impl KSubsets {
    fn new(n: usize, k: usize) -> Self {
        KSubsets { n, k, cur: (0..k).collect(), started: false }
    }
    fn next(&mut self) -> Option<&[usize]> {
        if self.k > self.n {
            return None;
        }
        if !self.started {
            self.started = true;
            return Some(&self.cur);
        }
        if self.k == 0 {
            return None; // the single empty subset was already served
        }
        // Advance to the next lexicographic combination.
        let mut i = self.k;
        loop {
            if i == 0 {
                return None;
            }
            i -= 1;
            if self.cur[i] != i + self.n - self.k {
                break;
            }
        }
        self.cur[i] += 1;
        for j in i + 1..self.k {
            self.cur[j] = self.cur[j - 1] + 1;
        }
        Some(&self.cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ksubsets_enumerates_lexicographically() {
        let mut it = KSubsets::new(4, 2);
        let mut all = vec![];
        while let Some(s) = it.next() {
            all.push(s.to_vec());
        }
        assert_eq!(
            all,
            vec![vec![0, 1], vec![0, 2], vec![0, 3], vec![1, 2], vec![1, 3], vec![2, 3]]
        );
        let mut empty = KSubsets::new(3, 0);
        assert_eq!(empty.next(), Some(&[][..]));
        assert!(empty.next().is_none());
        let mut too_big = KSubsets::new(2, 3);
        assert!(too_big.next().is_none());
    }

    /// Build a strongly dependent (X,Y) case set for PC tests.
    fn dependent_cases(
        x: crate::model::NodeId,
        y: crate::model::NodeId,
    ) -> crate::learn::cases::CaseSet {
        let mut rows = vec![];
        for _ in 0..100 {
            rows.push(vec![Some(0usize), Some(0usize)]);
            rows.push(vec![Some(1), Some(1)]);
        }
        for _ in 0..5 {
            rows.push(vec![Some(0), Some(1)]);
        }
        let n = rows.len();
        crate::learn::cases::CaseSet { nodes: vec![x, y], rows, weights: vec![1.0; n] }
    }

    #[test]
    fn pc_blacklist_removes_forbidden_edge() {
        use crate::learn::structure::EdgeConstraints;
        let mut net = crate::model::Network::new("t");
        let two = || vec![crate::model::State::new("a"), crate::model::State::new("b")];
        let x = net.add_node("X", crate::model::NodeKind::Chance, two()).unwrap();
        let y = net.add_node("Y", crate::model::NodeKind::Chance, two()).unwrap();
        let cases = dependent_cases(x, y);
        // Data implies X—Y adjacency, but both directions forbidden.
        let opts = PcOptions {
            ci: CiOptions::default(),
            constraints: EdgeConstraints {
                forbidden: vec![(x, y), (y, x)],
                ..Default::default()
            },
        };
        let _report =
            learn_pc(&mut net, &cases, &[x, y], &opts, &SearchCtrl::default()).unwrap();
        assert!(net.edges().is_empty(), "blacklisted edge must not appear");
    }

    #[test]
    fn pc_whitelist_forces_edge_and_direction() {
        use crate::learn::structure::EdgeConstraints;
        let mut net = crate::model::Network::new("t");
        let two = || vec![crate::model::State::new("a"), crate::model::State::new("b")];
        // A and B are independent (uniform data).
        let a = net.add_node("A", crate::model::NodeKind::Chance, two()).unwrap();
        let b = net.add_node("B", crate::model::NodeKind::Chance, two()).unwrap();
        let rows: Vec<Vec<Option<usize>>> =
            (0..100).map(|i| vec![Some(i % 2), Some((i / 2) % 2)]).collect();
        let n = rows.len();
        let cases =
            crate::learn::cases::CaseSet { nodes: vec![a, b], rows, weights: vec![1.0; n] };
        // Require A→B — PC would normally see independence and remove the edge.
        let opts = PcOptions {
            ci: CiOptions::default(),
            constraints: EdgeConstraints {
                required: vec![(a, b)],
                ..Default::default()
            },
        };
        let _report =
            learn_pc(&mut net, &cases, &[a, b], &opts, &SearchCtrl::default()).unwrap();
        let edges = net.edges();
        assert!(
            edges.contains(&(a, b)),
            "required edge A→B must appear in the output"
        );
    }
}
