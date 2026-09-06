//! Counting learning with experience (Dirichlet pseudo-counts):
//! each parent configuration carries an experience count ε; the update is
//! `new_row = (ε · old_row + counts) / (ε + n)`, `new_ε = ε + n`.

use crate::error::LearnError;
use crate::factor::encode_index;
use crate::learn::cases::CaseSet;
use crate::model::{Network, NodeKind};

#[derive(Clone, Debug)]
pub struct CountingOptions {
    /// Treat the current CPT (weighted by experience) as a prior. When off,
    /// counts replace the CPT outright (rows with no data keep their values).
    pub use_experience: bool,
}

impl Default for CountingOptions {
    fn default() -> Self {
        CountingOptions { use_experience: true }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LearnReport {
    pub nodes_updated: usize,
    pub cases_used: f64,
}

pub fn learn_counting(
    net: &mut Network,
    cases: &CaseSet,
    opts: &CountingOptions,
) -> Result<LearnReport, LearnError> {
    let col_of = |id| cases.nodes.iter().position(|&n| n == id);
    let mut report = LearnReport { cases_used: cases.total_weight(), ..Default::default() };
    let targets: Vec<_> = cases
        .nodes
        .iter()
        .copied()
        .filter(|&id| net.node(id).kind == NodeKind::Chance)
        .collect();
    if targets.is_empty() {
        return Err(LearnError::NothingToLearn);
    }
    for id in targets {
        let node_col = col_of(id).unwrap();
        let parent_cols: Option<Vec<usize>> =
            net.node(id).parents.iter().map(|&p| col_of(p)).collect();
        let Some(parent_cols) = parent_cols else {
            continue; // a parent has no data column: cannot count this family
        };
        let cards = net.parent_cards(id);
        let out = net.node(id).out_card();
        let rows = net.row_count(id);
        let mut counts = vec![0.0; rows * out];
        for (row, &w) in cases.rows.iter().zip(&cases.weights) {
            let Some(s) = row[node_col] else { continue };
            let pstates: Option<Vec<usize>> =
                parent_cols.iter().map(|&c| row[c]).collect();
            let Some(pstates) = pstates else { continue };
            let r = encode_index(&pstates, &cards);
            counts[r * out + s] += w;
        }
        let old_exp = net.node(id).experience.clone();
        let mut new_table = net.node(id).table.data.clone();
        let mut new_exp = Vec::with_capacity(rows);
        for r in 0..rows {
            let n: f64 = counts[r * out..(r + 1) * out].iter().sum();
            let eps = if opts.use_experience {
                old_exp.as_ref().map(|e| e[r]).unwrap_or(0.0)
            } else {
                0.0
            };
            if eps + n > 0.0 {
                for s in 0..out {
                    let old = new_table[r * out + s];
                    new_table[r * out + s] = (eps * old + counts[r * out + s]) / (eps + n);
                }
            }
            new_exp.push(eps + n);
        }
        net.set_table(id, crate::model::Table { data: new_table }).unwrap();
        net.set_experience(id, Some(new_exp)).unwrap();
        report.nodes_updated += 1;
    }
    Ok(report)
}
