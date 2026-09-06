//! Column-major view of a [`CaseSet`] restricted to the learning targets,
//! plus weighted contingency counting — the sufficient-statistics layer
//! every structure-learning algorithm sits on.

use crate::error::LearnError;
use crate::factor::encode_index;
use crate::learn::cases::CaseSet;
use crate::model::{Network, NodeId, NodeKind};

/// Refuse contingency tables above this many cells (~32 MB of f64).
pub(crate) const MAX_CONTINGENCY_CELLS: usize = 1 << 22;

/// Case data transposed to columns over a fixed target list. Targets define
/// the compact variable index space (`u32`) used throughout the module.
pub(crate) struct DataView {
    pub cards: Vec<usize>,
    /// Column-major; -1 = missing.
    cols: Vec<Vec<i32>>,
    weights: Vec<f64>,
}

impl DataView {
    /// Validates that every target is a chance node with a case column,
    /// then transposes the matching columns.
    pub fn new(net: &Network, cases: &CaseSet, targets: &[NodeId]) -> Result<DataView, LearnError> {
        let mut cards = Vec::with_capacity(targets.len());
        let mut col_of = Vec::with_capacity(targets.len());
        for &id in targets {
            let node = net.node(id);
            if node.kind != NodeKind::Chance {
                return Err(LearnError::NotChance(node.name.clone()));
            }
            let col = cases
                .nodes
                .iter()
                .position(|&n| n == id)
                .ok_or_else(|| LearnError::NoDataColumn(node.name.clone()))?;
            cards.push(node.n_states());
            col_of.push(col);
        }
        let mut cols = vec![Vec::with_capacity(cases.rows.len()); targets.len()];
        for row in &cases.rows {
            for (t, &c) in col_of.iter().enumerate() {
                match row[c] {
                    Some(s) => cols[t].push(s as i32),
                    None => cols[t].push(-1),
                }
            }
        }
        Ok(DataView { cards, cols, weights: cases.weights.clone() })
    }

    pub fn n_vars(&self) -> usize {
        self.cards.len()
    }

    /// Weighted contingency table over `vars` in the given axis order
    /// (row-major, last axis fastest — the CPT convention). Complete-case
    /// per query: rows missing any of `vars` are skipped. Returns the
    /// counts and the total weight of contributing rows.
    pub fn counts(&self, vars: &[u32]) -> Result<(Vec<f64>, f64), LearnError> {
        let cards: Vec<usize> = vars.iter().map(|&v| self.cards[v as usize]).collect();
        let cells: usize = cards.iter().product();
        if cells > MAX_CONTINGENCY_CELLS {
            return Err(LearnError::TableTooLarge { cells, max: MAX_CONTINGENCY_CELLS });
        }
        let mut out = vec![0.0; cells];
        let mut n = 0.0;
        let mut assignment = vec![0usize; vars.len()];
        'rows: for (r, &w) in self.weights.iter().enumerate() {
            for (a, &v) in assignment.iter_mut().zip(vars) {
                let s = self.cols[v as usize][r];
                if s < 0 {
                    continue 'rows;
                }
                *a = s as usize;
            }
            out[encode_index(&assignment, &cards)] += w;
            n += w;
        }
        Ok((out, n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Network, State};

    fn tiny() -> (Network, Vec<NodeId>, CaseSet) {
        let mut net = Network::new("t");
        let two = || vec![State::new("a"), State::new("b")];
        let x = net.add_node("X", NodeKind::Chance, two()).unwrap();
        let y = net.add_node("Y", NodeKind::Chance, two()).unwrap();
        let cases = CaseSet {
            nodes: vec![x, y],
            rows: vec![
                vec![Some(0), Some(0)],
                vec![Some(0), Some(1)],
                vec![Some(1), None],
                vec![Some(1), Some(1)],
            ],
            weights: vec![1.0, 2.0, 1.0, 3.0],
        };
        (net, vec![x, y], cases)
    }

    #[test]
    fn counts_weighted_and_complete_case() {
        let (net, targets, cases) = tiny();
        let view = DataView::new(&net, &cases, &targets).unwrap();
        // Joint over (X, Y): the row with missing Y is skipped.
        let (c, n) = view.counts(&[0, 1]).unwrap();
        assert_eq!(c, vec![1.0, 2.0, 0.0, 3.0]);
        assert_eq!(n, 6.0);
        // Marginal over X alone: nothing skipped.
        let (c, n) = view.counts(&[0]).unwrap();
        assert_eq!(c, vec![3.0, 4.0]);
        assert_eq!(n, 7.0);
        // Axis order respected: (Y, X) transposes.
        let (c, _) = view.counts(&[1, 0]).unwrap();
        assert_eq!(c, vec![1.0, 0.0, 2.0, 3.0]);
    }

    #[test]
    fn rejects_bad_targets() {
        let (net, targets, cases) = tiny();
        let mut net2 = net.clone();
        let z = net2.add_node("Z", NodeKind::Chance, vec![State::new("a")]).unwrap();
        assert!(matches!(
            DataView::new(&net2, &cases, &[targets[0], z]),
            Err(LearnError::NoDataColumn(_))
        ));
    }
}
