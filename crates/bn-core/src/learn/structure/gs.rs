//! Grow-Shrink Markov-blanket discovery (Margaritis & Thrun 1999), used as
//! a hybrid: the symmetrized blankets constrain a score-based hill climb
//! (MMHC-style, Tsamardinos 2006). Orientation by CI tests alone is covered
//! by PC-stable; the hybrid is both simpler and empirically stronger.

use crate::error::LearnError;
use crate::learn::cases::CaseSet;
use crate::learn::structure::ci::{CiOptions, CiTester};
use crate::learn::structure::dag::WorkDag;
use crate::learn::structure::hill_climb::{search, HillClimbOptions, HillClimbReport};
use crate::learn::structure::score::FamilyScorer;
use crate::learn::structure::stats::DataView;
use crate::learn::structure::{apply_edges, check_targets, EdgeConstraints, Progress, SearchCtrl};
use crate::model::{Network, NodeId};

#[derive(Clone, Debug)]
pub struct GsOptions {
    pub ci: CiOptions,
    pub hill: HillClimbOptions,
    /// Hard structural constraints (required / forbidden edges).
    pub constraints: EdgeConstraints,
}

impl Default for GsOptions {
    fn default() -> Self {
        GsOptions {
            // Blankets can legitimately be larger than PC's conditioning
            // sets (a node's parents + children + spouses), so allow more.
            ci: CiOptions { max_cond: 8, ..CiOptions::default() },
            hill: HillClimbOptions::default(),
            constraints: EdgeConstraints::default(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GsReport {
    /// Markov blanket found for each target (before symmetrization).
    pub blankets: Vec<(NodeId, Vec<NodeId>)>,
    pub ci_tests: usize,
    pub hill: HillClimbReport,
}

/// Markov blankets only (no network mutation) — for display/analysis.
pub fn grow_shrink_blankets(
    net: &Network,
    cases: &CaseSet,
    targets: &[NodeId],
    ci: &CiOptions,
    ctrl: &SearchCtrl,
) -> Result<Vec<(NodeId, Vec<NodeId>)>, LearnError> {
    let targets = check_targets(targets, 2)?;
    let view = DataView::new(net, cases, &targets)?;
    let mut tester = CiTester::new(&view, ci.clone());
    let blankets = blankets_ix(&view, &mut tester, ctrl)?;
    Ok(blankets
        .into_iter()
        .enumerate()
        .map(|(i, b)| {
            (targets[i], b.into_iter().map(|j| targets[j as usize]).collect())
        })
        .collect())
}

/// Grow-Shrink hybrid: blanket discovery, AND-rule symmetrization, then a
/// hill climb restricted to blanket members as candidate parents.
pub fn learn_gs(
    net: &mut Network,
    cases: &CaseSet,
    targets: &[NodeId],
    opts: &GsOptions,
    ctrl: &SearchCtrl,
) -> Result<GsReport, LearnError> {
    opts.constraints.validate(net)?;
    let targets = check_targets(targets, 2)?;
    let (req, forb) = opts.constraints.to_ix(&targets);
    let mut view = DataView::new(net, cases, &targets)?;
    let mut report = GsReport::default();

    let blankets = {
        let mut tester = CiTester::new(&view, opts.ci.clone());
        let b = blankets_ix(&view, &mut tester, ctrl)?;
        report.ci_tests = tester.tests_run;
        b
    };
    report.blankets = blankets
        .iter()
        .enumerate()
        .map(|(i, b)| {
            (targets[i], b.iter().map(|&j| targets[j as usize]).collect())
        })
        .collect();
    // OR-rule symmetrization: i may parent j when either is in the other's
    // blanket. The AND-rule loses true edges around near-deterministic
    // nodes (e.g. an OR gate is independent of its children given its
    // parents, so they drop out of its blanket); with a score-based climb
    // deciding final edges, extra candidates are cheap and missing ones
    // are unrecoverable.
    let n = targets.len();
    let mut allowed: Vec<Vec<u32>> = (0..n)
        .map(|j| {
            (0..n as u32)
                .filter(|&i| {
                    i as usize != j
                        && (blankets[j].contains(&i)
                            || blankets[i as usize].contains(&(j as u32)))
                })
                .collect()
        })
        .collect();
    // Apply constraints to the allowed list: forbidden pairs are removed,
    // required pairs are added (bypassing the blanket filter).
    for (j, allow_j) in allowed.iter_mut().enumerate() {
        allow_j.retain(|&i| !forb.contains(&(i, j as u32)));
        for &(p, c) in &req {
            if c as usize == j && !allow_j.contains(&p) {
                allow_j.push(p);
            }
        }
    }

    let template = WorkDag::from_network(net, &targets);
    let mut scorer = FamilyScorer::new(&mut view, opts.hill.score);
    let start = if opts.hill.start_from_current {
        template.clone()
    } else {
        template.cleared()
    };
    let dag = search(
        start,
        &mut scorer,
        &opts.hill,
        Some(&allowed),
        &req,
        &forb,
        ctrl,
        &mut report.hill,
    )?;
    apply_edges(net, &targets, &dag.edges())?;
    report.hill.edges = dag
        .edges()
        .iter()
        .map(|&(p, c)| (targets[p as usize], targets[c as usize]))
        .collect();
    Ok(report)
}

/// Grow-Shrink blanket per variable, in index space.
fn blankets_ix(
    view: &DataView,
    tester: &mut CiTester,
    ctrl: &SearchCtrl,
) -> Result<Vec<Vec<u32>>, LearnError> {
    let n = view.n_vars();
    let mut out = Vec::with_capacity(n);
    for t in 0..n as u32 {
        ctrl.check()?;
        ctrl.tick(Progress { phase: "blankets", done: t as usize, total: n, score: None });
        // Candidates ordered by descending pairwise MI with t: stabilizes
        // the grow phase and admits strong neighbors first.
        let mut cands: Vec<(f64, u32)> = Vec::with_capacity(n - 1);
        for x in 0..n as u32 {
            if x != t {
                let (mi, _) = tester.cmi(t, x, &[])?;
                cands.push((mi, x));
            }
        }
        cands.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let cands: Vec<u32> = cands.into_iter().map(|(_, x)| x).collect();

        // Grow: add X when it is dependent on t given the current blanket;
        // sweep until a full pass adds nothing (or the size cap binds).
        let mut blanket: Vec<u32> = vec![];
        loop {
            ctrl.check()?;
            let mut grew = false;
            for &x in &cands {
                if blanket.contains(&x) || blanket.len() >= tester.opts.max_cond {
                    continue;
                }
                let (ind, _) = tester.independent(t, x, &blanket)?;
                if !ind {
                    blanket.push(x);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        // Shrink: drop X when it is independent of t given the rest;
        // repeat until stable.
        loop {
            let mut shrunk = false;
            let members = blanket.clone();
            for &x in &members {
                let rest: Vec<u32> =
                    blanket.iter().copied().filter(|&y| y != x).collect();
                let (ind, _) = tester.independent(t, x, &rest)?;
                if ind {
                    blanket.retain(|&y| y != x);
                    shrunk = true;
                }
            }
            if !shrunk {
                break;
            }
        }
        blanket.sort_unstable();
        out.push(blanket);
    }
    Ok(out)
}
