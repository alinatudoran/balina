//! Structural EM (Friedman 1998): structure search with missing data.
//!
//! Outer loop: fit parameters by EM on the current structure (working
//! clone), then hill-climb on the *expected* score, where family counts for
//! candidate structures come from the junction tree's joint posteriors
//! under the current model. The expected score is non-decreasing across
//! outer iterations when the E-step is exact.

use std::collections::HashMap;

use crate::error::{InferenceError, LearnError};
use crate::factor::{decode_index, encode_index};
use crate::inference::{Engine, Evidence, Finding};
use crate::learn::cases::CaseSet;
use crate::learn::em::{learn_em, EmOptions};
use crate::learn::structure::dag::WorkDag;
use crate::learn::structure::hill_climb::{climb, seed_required, HillClimbOptions, HillClimbReport};
use crate::learn::structure::score::{CountSource, FamilyScorer};
use crate::learn::structure::stats::{DataView, MAX_CONTINGENCY_CELLS};
use crate::learn::structure::{apply_edges, check_targets, EdgeConstraints, Progress, SearchCtrl};
use crate::model::{Network, NodeId};

#[derive(Clone, Debug)]
pub struct SemOptions {
    /// Inner structure search. Restarts default to 0 here: each outer
    /// iteration warm-starts from the current structure instead.
    pub hill: HillClimbOptions,
    /// Inner parametric EM (also used for the final fit).
    pub em: EmOptions,
    pub max_outer_iters: usize,
    /// Stop when the expected score improves by less than this.
    pub tol: f64,
    /// Hard structural constraints (required / forbidden edges).
    pub constraints: EdgeConstraints,
}

impl Default for SemOptions {
    fn default() -> Self {
        SemOptions {
            hill: HillClimbOptions {
                // Expected counts for a candidate family need a joint
                // posterior over the whole family: keep families small.
                max_parents: 3,
                random_restarts: 0,
                ..HillClimbOptions::default()
            },
            em: EmOptions::default(),
            max_outer_iters: 10,
            tol: 1e-3,
            constraints: EdgeConstraints::default(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SemReport {
    pub outer_iterations: usize,
    /// Best expected score after each outer iteration's climb.
    pub expected_score_trace: Vec<f64>,
    /// Observed-data log-likelihood of the final fitted model.
    pub log_likelihood: f64,
    /// Cases with zero probability under the final model.
    pub conflicting_cases: usize,
    pub edges: Vec<(NodeId, NodeId)>,
}

/// Learn structure from incomplete data by Structural EM, then fit the
/// final parameters by EM on the caller's network.
pub fn learn_structural_em(
    net: &mut Network,
    cases: &CaseSet,
    targets: &[NodeId],
    opts: &SemOptions,
    ctrl: &SearchCtrl,
) -> Result<SemReport, LearnError> {
    opts.constraints.validate(net)?;
    let targets = check_targets(targets, 2)?;
    let (req, forb) = opts.constraints.to_ix(&targets);
    // Validates targets and provides cards; also detects the complete-data
    // case, where the expected counts reduce to observed counts.
    let view = DataView::new(net, cases, &targets)?;

    let template = WorkDag::from_network(net, &targets);
    let mut dag =
        if opts.hill.start_from_current { template.clone() } else { template.cleared() };
    seed_required(&mut dag, &req);
    let mut report = SemReport::default();
    let mut prev_score = f64::NEG_INFINITY;

    for iter in 0..opts.max_outer_iters {
        ctrl.check()?;
        ctrl.tick(Progress {
            phase: "structural EM",
            done: iter,
            total: opts.max_outer_iters,
            score: (iter > 0).then_some(prev_score),
        });
        // M-step (parameters): EM on a working clone with the current edges.
        let mut working = net.clone();
        apply_edges(&mut working, &targets, &dag.edges())?;
        learn_em(&mut working, cases, &opts.em)?;

        // E-step provider: expected family counts under the fitted model.
        let mut counts = PosteriorCounts::new(&working, cases, &targets, &view)?;
        let mut scorer = FamilyScorer::new(&mut counts, opts.hill.score);

        // M-step (structure): climb on the expected score from the current
        // structure.
        let mut hill_report = HillClimbReport::default();
        let mut new_dag = dag.clone();
        let score = climb(
            &mut new_dag,
            &mut scorer,
            &opts.hill,
            None,
            &req,
            &forb,
            ctrl,
            &mut hill_report,
        )?;
        report.outer_iterations = iter + 1;
        report.expected_score_trace.push(score);
        let unchanged = new_dag.edges() == dag.edges();
        dag = new_dag;
        if unchanged || score - prev_score < opts.tol {
            break;
        }
        prev_score = score;
    }

    apply_edges(net, &targets, &dag.edges())?;
    let em_report = learn_em(net, cases, &opts.em)?;
    report.log_likelihood =
        em_report.log_likelihood_trace.last().copied().unwrap_or(f64::NAN);
    report.conflicting_cases = em_report.conflicting_cases;
    report.edges = dag
        .edges()
        .iter()
        .map(|&(p, c)| (targets[p as usize], targets[c as usize]))
        .collect();
    Ok(report)
}

/// Expected sufficient statistics under a fitted model: rows fully observed
/// over the targets are counted directly; the rest go through the junction
/// tree's joint posterior over the candidate family. Memoized per instance
/// (one instance per outer SEM iteration).
struct PosteriorCounts<'a> {
    engine: Engine,
    targets: &'a [NodeId],
    cards: Vec<usize>,
    complete: DataView,
    /// Deduped incomplete patterns over the case set's columns, sorted for
    /// reproducible float summation.
    incomplete: Vec<(Vec<Option<usize>>, f64)>,
    case_nodes: &'a [NodeId],
    memo: HashMap<(u32, Vec<u32>), (Vec<f64>, f64)>,
}

impl<'a> PosteriorCounts<'a> {
    fn new(
        working: &Network,
        cases: &'a CaseSet,
        targets: &'a [NodeId],
        view: &DataView,
    ) -> Result<PosteriorCounts<'a>, LearnError> {
        let col_of: Vec<usize> = targets
            .iter()
            .map(|&t| cases.nodes.iter().position(|&n| n == t).unwrap())
            .collect();
        let mut complete_rows = Vec::new();
        let mut complete_weights = Vec::new();
        let mut unique: HashMap<Vec<Option<usize>>, f64> = HashMap::new();
        for (row, &w) in cases.rows.iter().zip(&cases.weights) {
            if col_of.iter().all(|&c| row[c].is_some()) {
                complete_rows.push(row.clone());
                complete_weights.push(w);
            } else {
                *unique.entry(row.clone()).or_insert(0.0) += w;
            }
        }
        let mut incomplete: Vec<(Vec<Option<usize>>, f64)> = unique.into_iter().collect();
        incomplete.sort_by(|a, b| a.0.cmp(&b.0));
        let complete = DataView::new(
            working,
            &CaseSet {
                nodes: cases.nodes.clone(),
                rows: complete_rows,
                weights: complete_weights,
            },
            targets,
        )?;
        Ok(PosteriorCounts {
            engine: Engine::compile(working),
            targets,
            cards: view.cards.clone(),
            complete,
            incomplete,
            case_nodes: &cases.nodes,
            memo: HashMap::new(),
        })
    }
}

impl CountSource for PosteriorCounts<'_> {
    fn cards(&self) -> &[usize] {
        &self.cards
    }

    /// Counts in `[parents..., child]` layout. The effective N is the same
    /// for every candidate family (total non-conflicting case weight), so
    /// the expected score has no available-case bias.
    fn family_counts(
        &mut self,
        child: u32,
        parents: &[u32],
    ) -> Result<(Vec<f64>, f64), LearnError> {
        let key = (child, parents.to_vec());
        if let Some((c, n)) = self.memo.get(&key) {
            return Ok((c.clone(), *n));
        }
        let mut vars = parents.to_vec();
        vars.push(child);
        let axis_cards: Vec<usize> =
            vars.iter().map(|&v| self.cards[v as usize]).collect();
        let cells: usize = axis_cards.iter().product();
        if cells > MAX_CONTINGENCY_CELLS {
            return Err(LearnError::TableTooLarge { cells, max: MAX_CONTINGENCY_CELLS });
        }
        // Fully observed rows: plain weighted counts.
        let (mut counts, mut n) = self.complete.counts(&vars)?;
        // Incomplete rows: fold w · P(family | case evidence) per pattern.
        let family_ids: Vec<NodeId> =
            vars.iter().map(|&v| self.targets[v as usize]).collect();
        for (row, w) in &self.incomplete {
            let mut ev = Evidence::new();
            for (col, v) in row.iter().enumerate() {
                if let Some(s) = v {
                    ev.set(self.case_nodes[col], Finding::Hard(*s));
                }
            }
            self.engine.set_evidence(ev);
            let post = match self.engine.joint_posterior(&family_ids) {
                Ok(p) => p,
                Err(InferenceError::ConflictingEvidence) => continue,
                Err(e) => return Err(e.into()),
            };
            // Map each table axis to its position in the (sorted-VarId)
            // posterior factor.
            let cn = self.engine.compiled();
            let pos: Vec<usize> = family_ids
                .iter()
                .map(|&id| {
                    let v = cn.var_of[id];
                    post.vars.iter().position(|&pv| pv == v).unwrap()
                })
                .collect();
            for (i, &p) in post.data.iter().enumerate() {
                if p == 0.0 {
                    continue;
                }
                let a = decode_index(i, &post.cards);
                let t: Vec<usize> = pos.iter().map(|&j| a[j]).collect();
                counts[encode_index(&t, &axis_cards)] += w * p;
            }
            n += w;
        }
        self.memo.insert(key, (counts.clone(), n));
        Ok((counts, n))
    }
}
