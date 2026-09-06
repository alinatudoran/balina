//! Conditional-independence testing (G² / conditional mutual information)
//! on weighted contingency counts.

use crate::error::LearnError;
use crate::learn::structure::math::chi2_sf;
use crate::learn::structure::stats::DataView;

#[derive(Clone, Debug)]
pub struct CiOptions {
    /// p-value threshold: independent iff p > alpha.
    pub alpha: f64,
    /// Max conditioning-set size.
    pub max_cond: usize,
    /// Declare independence untested when the effective sample size is below
    /// this many observations per contingency cell (unreliable test).
    /// 0 = always test.
    pub min_obs_per_cell: f64,
}

impl Default for CiOptions {
    fn default() -> Self {
        CiOptions { alpha: 0.05, max_cond: 4, min_obs_per_cell: 0.0 }
    }
}

pub(crate) struct CiTester<'a> {
    pub view: &'a DataView,
    pub opts: CiOptions,
    pub tests_run: usize,
}

impl<'a> CiTester<'a> {
    pub fn new(view: &'a DataView, opts: CiOptions) -> Self {
        CiTester { view, opts, tests_run: 0 }
    }

    /// G² test of X ⊥ Y | Z. Returns (independent, p_value). Degenerate
    /// tables (zero adjusted df, or too few observations per cell) cannot
    /// reject, so they count as independent with p = 1.
    pub fn independent(&mut self, x: u32, y: u32, z: &[u32]) -> Result<(bool, f64), LearnError> {
        self.tests_run += 1;
        let (g2, df, n, cells) = self.g2(x, y, z)?;
        if self.opts.min_obs_per_cell > 0.0 && n < self.opts.min_obs_per_cell * cells as f64 {
            return Ok((true, 1.0));
        }
        if df == 0 {
            return Ok((true, 1.0));
        }
        let p = chi2_sf(g2, df as f64);
        Ok((p > self.opts.alpha, p))
    }

    /// Conditional mutual information I(X; Y | Z) in nats over complete
    /// cases, plus the effective sample weight. (G² = 2·N·CMI.)
    pub fn cmi(&mut self, x: u32, y: u32, z: &[u32]) -> Result<(f64, f64), LearnError> {
        let (g2, _, n, _) = self.g2(x, y, z)?;
        Ok((if n > 0.0 { g2 / (2.0 * n) } else { 0.0 }, n))
    }

    /// Shared accumulation: returns (G², adjusted df, effective N, cells).
    /// Counts are laid out [z..., x, y] so each contiguous rx·ry block is
    /// one z-configuration. df is adjusted bnlearn-style: per z-slice only
    /// rows/columns with nonzero marginals contribute, and all-degenerate
    /// slices contribute nothing.
    fn g2(&self, x: u32, y: u32, z: &[u32]) -> Result<(f64, usize, f64, usize), LearnError> {
        let mut vars = z.to_vec();
        vars.push(x);
        vars.push(y);
        let (counts, n) = self.view.counts(&vars)?;
        let rx = self.view.cards[x as usize];
        let ry = self.view.cards[y as usize];
        let mut g2 = 0.0;
        let mut df = 0usize;
        let mut nx = vec![0.0; rx];
        let mut ny = vec![0.0; ry];
        for slice in counts.chunks(rx * ry) {
            let nz: f64 = slice.iter().sum();
            if nz <= 0.0 {
                continue;
            }
            nx.fill(0.0);
            ny.fill(0.0);
            for (i, &c) in slice.iter().enumerate() {
                nx[i / ry] += c;
                ny[i % ry] += c;
            }
            let a = nx.iter().filter(|&&v| v > 0.0).count();
            let b = ny.iter().filter(|&&v| v > 0.0).count();
            if a < 2 || b < 2 {
                continue;
            }
            df += (a - 1) * (b - 1);
            for (i, &c) in slice.iter().enumerate() {
                if c > 0.0 {
                    g2 += 2.0 * c * (c * nz / (nx[i / ry] * ny[i % ry])).ln();
                }
            }
        }
        Ok((g2.max(0.0), df, n, counts.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::cases::CaseSet;
    use crate::model::{Network, NodeId, NodeKind, State};
    use rand::rngs::StdRng;
    use rand::{RngExt, SeedableRng};

    fn three_vars() -> (Network, Vec<NodeId>) {
        let mut net = Network::new("t");
        let two = || vec![State::new("a"), State::new("b")];
        let x = net.add_node("X", NodeKind::Chance, two()).unwrap();
        let y = net.add_node("Y", NodeKind::Chance, two()).unwrap();
        let z = net.add_node("Z", NodeKind::Chance, two()).unwrap();
        (net, vec![x, y, z])
    }

    #[test]
    fn detects_dependence_and_independence() {
        let (net, ids) = three_vars();
        let mut rng = StdRng::seed_from_u64(2);
        // Z ~ coin; X = Z with noise; Y independent coin.
        let mut rows = vec![];
        for _ in 0..4000 {
            let z = rng.random_range(0..2usize);
            let x = if rng.random::<f64>() < 0.9 { z } else { 1 - z };
            let y = rng.random_range(0..2usize);
            rows.push(vec![Some(x), Some(y), Some(z)]);
        }
        let cases =
            CaseSet { nodes: ids.clone(), rows, weights: vec![1.0; 4000] };
        let view = DataView::new(&net, &cases, &ids).unwrap();
        let mut t = CiTester::new(&view, CiOptions::default());
        let (ind, p) = t.independent(0, 2, &[]).unwrap();
        assert!(!ind && p < 1e-6, "X and Z strongly dependent, p = {p}");
        let (ind, p) = t.independent(0, 1, &[]).unwrap();
        assert!(ind, "X and Y independent, p = {p}");
        let (ind, _) = t.independent(0, 1, &[2]).unwrap();
        assert!(ind, "X and Y independent given Z");
        assert_eq!(t.tests_run, 3);
    }

    #[test]
    fn degenerate_slices_cannot_reject() {
        let (net, ids) = three_vars();
        // Y is constant: marginal has one nonzero level → df = 0 → independent.
        let rows = vec![
            vec![Some(0), Some(0), Some(0)],
            vec![Some(1), Some(0), Some(1)],
            vec![Some(0), Some(0), Some(1)],
        ];
        let cases = CaseSet { nodes: ids.clone(), rows, weights: vec![1.0; 3] };
        let view = DataView::new(&net, &cases, &ids).unwrap();
        let mut t = CiTester::new(&view, CiOptions::default());
        let (ind, p) = t.independent(0, 1, &[]).unwrap();
        assert!(ind && p == 1.0);
    }

    #[test]
    fn cmi_matches_g2_relation() {
        let (net, ids) = three_vars();
        let rows = vec![
            vec![Some(0), Some(0), Some(0)],
            vec![Some(0), Some(1), Some(0)],
            vec![Some(1), Some(0), Some(1)],
            vec![Some(1), Some(1), Some(1)],
        ];
        let cases = CaseSet { nodes: ids.clone(), rows, weights: vec![3.0, 1.0, 1.0, 3.0] };
        let view = DataView::new(&net, &cases, &ids).unwrap();
        let mut t = CiTester::new(&view, CiOptions::default());
        let (cmi, n) = t.cmi(0, 1, &[]).unwrap();
        assert_eq!(n, 8.0);
        // Hand computation: I(X;Y) with joint [3,1,1,3]/8.
        let expect = 2.0 * (3.0 / 8.0 * (3.0f64 / 8.0 / (0.5 * 0.5)).ln())
            + 2.0 * (1.0 / 8.0 * (1.0f64 / 8.0 / (0.5 * 0.5)).ln());
        assert!((cmi - expect).abs() < 1e-12);
    }
}
