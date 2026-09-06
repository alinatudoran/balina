//! Decomposable network scores (BIC, BDeu) with a per-family cache.
//!
//! Counts come from a [`CountSource`] so the same scorer drives both plain
//! search (observed counts from [`DataView`]) and Structural EM (expected
//! counts from the junction tree).

use std::collections::HashMap;

use crate::error::LearnError;
use crate::learn::structure::math::ln_gamma;
use crate::learn::structure::stats::DataView;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScoreKind {
    Bic,
    Bdeu { ess: f64 },
}

impl Default for ScoreKind {
    fn default() -> Self {
        ScoreKind::Bic
    }
}

/// Where family counts come from. `family_counts` returns counts in
/// `[parents..., child]` layout (last axis fastest) plus the effective
/// sample weight behind them.
pub(crate) trait CountSource {
    fn cards(&self) -> &[usize];
    fn family_counts(&mut self, child: u32, parents: &[u32])
        -> Result<(Vec<f64>, f64), LearnError>;
}

impl CountSource for DataView {
    fn cards(&self) -> &[usize] {
        &self.cards
    }
    fn family_counts(
        &mut self,
        child: u32,
        parents: &[u32],
    ) -> Result<(Vec<f64>, f64), LearnError> {
        let mut vars = parents.to_vec();
        vars.push(child);
        self.counts(&vars)
    }
}

/// Caches local scores keyed by (child, sorted parent set).
pub(crate) struct FamilyScorer<'a, S: CountSource> {
    src: &'a mut S,
    kind: ScoreKind,
    cache: HashMap<(u32, Vec<u32>), f64>,
}

impl<'a, S: CountSource> FamilyScorer<'a, S> {
    pub fn new(src: &'a mut S, kind: ScoreKind) -> Self {
        FamilyScorer { src, kind, cache: HashMap::new() }
    }

    /// Local score of `child` given `parents` (order-insensitive).
    pub fn score(&mut self, child: u32, parents: &[u32]) -> Result<f64, LearnError> {
        let mut key_parents = parents.to_vec();
        key_parents.sort_unstable();
        if let Some(&s) = self.cache.get(&(child, key_parents.clone())) {
            return Ok(s);
        }
        let (counts, n) = self.src.family_counts(child, &key_parents)?;
        let r = self.src.cards()[child as usize];
        let q = counts.len() / r;
        let s = match self.kind {
            ScoreKind::Bic => bic(&counts, n, q, r),
            ScoreKind::Bdeu { ess } => bdeu(&counts, q, r, ess),
        };
        self.cache.insert((child, key_parents), s);
        Ok(s)
    }

    /// Σ of local scores over all variables under the given parent sets.
    pub fn total(&mut self, parents: &[Vec<u32>]) -> Result<f64, LearnError> {
        let mut sum = 0.0;
        for (child, pa) in parents.iter().enumerate() {
            sum += self.score(child as u32, pa)?;
        }
        Ok(sum)
    }
}

/// BIC = Σ_{j,k} N_ijk · ln(N_ijk / N_ij)  −  (ln N / 2) · q · (r − 1).
fn bic(counts: &[f64], n: f64, q: usize, r: usize) -> f64 {
    let mut ll = 0.0;
    for row in counts.chunks(r) {
        let nj: f64 = row.iter().sum();
        if nj <= 0.0 {
            continue;
        }
        for &c in row {
            if c > 0.0 {
                ll += c * (c / nj).ln();
            }
        }
    }
    ll - 0.5 * n.max(1.0).ln() * (q * (r - 1)) as f64
}

/// BDeu with equivalent sample size `ess`:
/// Σ_j [ lnΓ(α_j) − lnΓ(α_j + N_ij) + Σ_k ( lnΓ(α_jk + N_ijk) − lnΓ(α_jk) ) ].
fn bdeu(counts: &[f64], q: usize, r: usize, ess: f64) -> f64 {
    let a_j = ess / q as f64;
    let a_jk = ess / (q * r) as f64;
    let mut s = 0.0;
    for row in counts.chunks(r) {
        let nj: f64 = row.iter().sum();
        if nj <= 0.0 {
            continue; // empty parent configuration contributes exactly 0
        }
        s += ln_gamma(a_j) - ln_gamma(a_j + nj);
        for &c in row {
            if c > 0.0 {
                s += ln_gamma(a_jk + c) - ln_gamma(a_jk);
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::cases::CaseSet;
    use crate::model::{Network, NodeKind, State};

    fn xy_data() -> (Network, Vec<crate::model::NodeId>, CaseSet) {
        let mut net = Network::new("t");
        let two = || vec![State::new("a"), State::new("b")];
        let x = net.add_node("X", NodeKind::Chance, two()).unwrap();
        let y = net.add_node("Y", NodeKind::Chance, two()).unwrap();
        // Strongly correlated X, Y.
        let mut rows = vec![];
        for _ in 0..40 {
            rows.push(vec![Some(0), Some(0)]);
            rows.push(vec![Some(1), Some(1)]);
        }
        for _ in 0..5 {
            rows.push(vec![Some(0), Some(1)]);
            rows.push(vec![Some(1), Some(0)]);
        }
        let n = rows.len();
        (net.clone(), vec![x, y], CaseSet { nodes: vec![x, y], rows, weights: vec![1.0; n] })
    }

    #[test]
    fn bic_hand_computed() {
        // counts [6, 2] for a root node with 2 states, N = 8, q = 1:
        // LL = 6·ln(6/8) + 2·ln(2/8); penalty = ln(8)/2 · 1 · 1.
        let ll = 6.0 * (0.75f64).ln() + 2.0 * (0.25f64).ln();
        let expect = ll - 0.5 * 8.0f64.ln();
        assert!((bic(&[6.0, 2.0], 8.0, 1, 2) - expect).abs() < 1e-12);
    }

    #[test]
    fn bdeu_score_equivalence() {
        // Markov-equivalent X→Y and Y→X must score identically under BDeu.
        let (net, targets, cases) = xy_data();
        let mut view = DataView::new(&net, &cases, &targets).unwrap();
        let mut sc = FamilyScorer::new(&mut view, ScoreKind::Bdeu { ess: 1.0 });
        let xy = sc.score(0, &[]).unwrap() + sc.score(1, &[0]).unwrap();
        let yx = sc.score(1, &[]).unwrap() + sc.score(0, &[1]).unwrap();
        assert!((xy - yx).abs() < 1e-9, "BDeu not score-equivalent: {xy} vs {yx}");
    }

    #[test]
    fn dependent_edge_beats_empty() {
        let (net, targets, cases) = xy_data();
        let mut view = DataView::new(&net, &cases, &targets).unwrap();
        for kind in [ScoreKind::Bic, ScoreKind::Bdeu { ess: 1.0 }] {
            let mut sc = FamilyScorer::new(&mut view, kind);
            let with_edge = sc.score(0, &[]).unwrap() + sc.score(1, &[0]).unwrap();
            let empty = sc.score(0, &[]).unwrap() + sc.score(1, &[]).unwrap();
            assert!(with_edge > empty, "{kind:?}: edge should win on dependent data");
        }
    }
}
