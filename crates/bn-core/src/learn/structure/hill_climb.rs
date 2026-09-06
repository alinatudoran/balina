//! Greedy hill climbing over DAGs with a decomposable score (BIC/BDeu).
//! Operators: add, delete, reverse. Only the touched families are rescored
//! per move; the [`FamilyScorer`] cache makes repeated sweeps cheap.

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use crate::error::LearnError;
use crate::learn::cases::CaseSet;
use crate::learn::structure::dag::WorkDag;
use crate::learn::structure::score::{CountSource, FamilyScorer, ScoreKind};
use crate::learn::structure::stats::DataView;
use crate::learn::structure::{apply_edges, check_targets, EdgeConstraints, Progress, SearchCtrl};
use crate::model::{Network, NodeId};

#[derive(Clone, Debug)]
pub struct HillClimbOptions {
    pub score: ScoreKind,
    /// Cap on learned (target) parents per node; pre-existing external
    /// parents don't count against it.
    pub max_parents: usize,
    pub random_restarts: usize,
    /// Random edge perturbations applied per restart.
    pub perturbations: usize,
    pub seed: u64,
    /// Start from the network's current target-target edges instead of the
    /// empty graph.
    pub start_from_current: bool,
    /// Hard structural constraints: required edges are pre-seeded and
    /// protected; forbidden edges are never added.
    pub constraints: EdgeConstraints,
}

impl Default for HillClimbOptions {
    fn default() -> Self {
        HillClimbOptions {
            score: ScoreKind::Bic,
            max_parents: 4,
            // Plain greedy ascent gets stuck in wrong-direction local optima
            // even on textbook nets (sprinkler); a few seeded restarts fix it.
            random_restarts: 4,
            perturbations: 4,
            seed: 0,
            start_from_current: false,
            constraints: EdgeConstraints::default(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct HillClimbReport {
    pub score: f64,
    /// Total accepted moves across all restarts.
    pub iterations: usize,
    pub restarts: usize,
    /// Best total score after each accepted move.
    pub score_trace: Vec<f64>,
    pub edges: Vec<(NodeId, NodeId)>,
}

/// Pre-add required edges to the DAG. Silently skips any that would create a
/// cycle (which can happen if required edges conflict with external paths).
pub(crate) fn seed_required(dag: &mut WorkDag, required: &[(u32, u32)]) {
    for &(p, c) in required {
        if (p as usize) < dag.n() && (c as usize) < dag.n()
            && !dag.has_edge(p, c)
            && !dag.creates_cycle(p, c)
        {
            dag.add(p, c);
        }
    }
}

/// Learn edges among `targets` by greedy hill climbing, then rewrite the
/// network's target-target edges. Parameters are NOT fitted.
pub fn learn_hill_climb(
    net: &mut Network,
    cases: &CaseSet,
    targets: &[NodeId],
    opts: &HillClimbOptions,
    ctrl: &SearchCtrl,
) -> Result<HillClimbReport, LearnError> {
    opts.constraints.validate(net)?;
    let targets = check_targets(targets, 2)?;
    let (req, forb) = opts.constraints.to_ix(&targets);
    let mut view = DataView::new(net, cases, &targets)?;
    let mut scorer = FamilyScorer::new(&mut view, opts.score);
    let template = WorkDag::from_network(net, &targets);
    let start = if opts.start_from_current { template.clone() } else { template.cleared() };

    let mut report = HillClimbReport::default();
    let best_dag = search(start, &mut scorer, opts, None, &req, &forb, ctrl, &mut report)?;

    apply_edges(net, &targets, &best_dag.edges())?;
    report.edges = best_dag
        .edges()
        .iter()
        .map(|&(p, c)| (targets[p as usize], targets[c as usize]))
        .collect();
    Ok(report)
}

/// Climb from `start`, then run seeded random restarts from perturbed
/// copies of the best graph; returns the best DAG found and fills
/// `report.score/iterations/restarts/score_trace`. Shared by plain hill
/// climbing and the Grow-Shrink hybrid.
pub(crate) fn search<S: CountSource>(
    start: WorkDag,
    scorer: &mut FamilyScorer<S>,
    opts: &HillClimbOptions,
    allowed: Option<&[Vec<u32>]>,
    req: &[(u32, u32)],
    forb: &[(u32, u32)],
    ctrl: &SearchCtrl,
    report: &mut HillClimbReport,
) -> Result<WorkDag, LearnError> {
    let (mut best_score, mut best_dag) = {
        let mut dag = start;
        seed_required(&mut dag, req);
        let score = climb(&mut dag, scorer, opts, allowed, req, forb, ctrl, report)?;
        (score, dag)
    };
    for r in 0..opts.random_restarts {
        ctrl.check()?;
        report.restarts += 1;
        ctrl.tick(Progress {
            phase: "restart",
            done: r + 1,
            total: opts.random_restarts,
            score: Some(best_score),
        });
        let mut rng = StdRng::seed_from_u64(opts.seed ^ (r as u64 + 1));
        let mut dag = best_dag.clone();
        perturb(&mut dag, opts, allowed, req, forb, &mut rng);
        seed_required(&mut dag, req);
        let score = climb(&mut dag, scorer, opts, allowed, req, forb, ctrl, report)?;
        if score > best_score {
            best_score = score;
            best_dag = dag;
        }
    }
    report.score = best_score;
    Ok(best_dag)
}

/// Apply random legal add/delete moves (used between restarts).
fn perturb(
    dag: &mut WorkDag,
    opts: &HillClimbOptions,
    allowed: Option<&[Vec<u32>]>,
    req: &[(u32, u32)],
    forb: &[(u32, u32)],
    rng: &mut StdRng,
) {
    let n = dag.n() as u32;
    for _ in 0..opts.perturbations {
        let i = rng.random_range(0..n);
        let j = rng.random_range(0..n);
        if i == j {
            continue;
        }
        if dag.has_edge(i, j) {
            // Never remove a required edge during perturbation.
            if !req.contains(&(i, j)) {
                dag.remove(i, j);
            }
        } else if !dag.has_edge(j, i)
            && dag.parents[j as usize].len() < opts.max_parents
            && allowed.is_none_or(|a| a[j as usize].contains(&i))
            && !forb.contains(&(i, j))
            && !dag.creates_cycle(i, j)
        {
            dag.add(i, j);
        }
    }
}

/// Greedy ascent from `dag` to a local optimum; returns the total score.
/// `allowed` optionally whitelists parents per child (Grow-Shrink hybrid).
/// `req` / `forb` are the index-space required / forbidden edge pairs.
/// Shared by hill climbing, GS, and Structural EM.
pub(crate) fn climb<S: CountSource>(
    dag: &mut WorkDag,
    scorer: &mut FamilyScorer<S>,
    opts: &HillClimbOptions,
    allowed: Option<&[Vec<u32>]>,
    req: &[(u32, u32)],
    forb: &[(u32, u32)],
    ctrl: &SearchCtrl,
    report: &mut HillClimbReport,
) -> Result<f64, LearnError> {
    let n = dag.n() as u32;
    // An edge is permitted to add if it is not forbidden and passes the allowed filter.
    let permitted = |p: u32, c: u32| {
        !forb.contains(&(p, c)) && allowed.is_none_or(|a| a[c as usize].contains(&p))
    };
    // A required edge must not be deleted or used as the source of a reversal.
    let protected = |p: u32, c: u32| req.contains(&(p, c));
    let mut total = scorer.total(&dag.parents)?;
    loop {
        ctrl.check()?;
        // Best move this sweep: (delta, kind, i, j); kind 0=add 1=del 2=rev.
        let mut best: Option<(f64, u8, u32, u32)> = None;
        let consider = |cand: (f64, u8, u32, u32), best: &mut Option<(f64, u8, u32, u32)>| {
            if best.is_none_or(|b| cand.0 > b.0) {
                *best = Some(cand);
            }
        };
        for i in 0..n {
            for j in 0..n {
                if i == j {
                    continue;
                }
                let cur_j = scorer.score(j, &dag.parents[j as usize])?;
                if dag.has_edge(i, j) {
                    // Delete i→j — skip if required.
                    if !protected(i, j) {
                        let without: Vec<u32> = dag.parents[j as usize]
                            .iter()
                            .copied()
                            .filter(|&p| p != i)
                            .collect();
                        let d_del = scorer.score(j, &without)? - cur_j;
                        consider((d_del, 1, i, j), &mut best);
                    }
                    // Reverse i→j to j→i — skip if i→j is required or j→i is forbidden.
                    if !protected(i, j)
                        && dag.parents[i as usize].len() < opts.max_parents
                        && permitted(j, i)
                    {
                        dag.remove(i, j);
                        let cycle = dag.creates_cycle(j, i);
                        dag.add(i, j);
                        if !cycle {
                            let without: Vec<u32> = dag.parents[j as usize]
                                .iter()
                                .copied()
                                .filter(|&p| p != i)
                                .collect();
                            let d_del = scorer.score(j, &without)? - cur_j;
                            let mut with_j: Vec<u32> = dag.parents[i as usize].clone();
                            with_j.push(j);
                            let d_rev = d_del + scorer.score(i, &with_j)?
                                - scorer.score(i, &dag.parents[i as usize])?;
                            consider((d_rev, 2, i, j), &mut best);
                        }
                    }
                } else if !dag.has_edge(j, i)
                    && dag.parents[j as usize].len() < opts.max_parents
                    && permitted(i, j)
                    && !dag.creates_cycle(i, j)
                {
                    // Add i→j.
                    let mut with_i: Vec<u32> = dag.parents[j as usize].clone();
                    with_i.push(i);
                    consider((scorer.score(j, &with_i)? - cur_j, 0, i, j), &mut best);
                }
            }
        }
        match best {
            Some((delta, kind, i, j)) if delta > 1e-9 => {
                match kind {
                    0 => dag.add(i, j),
                    1 => dag.remove(i, j),
                    _ => {
                        dag.remove(i, j);
                        dag.add(j, i);
                    }
                }
                total += delta;
                report.iterations += 1;
                report.score_trace.push(total);
                ctrl.tick(Progress {
                    phase: "climbing",
                    done: report.iterations,
                    total: 0,
                    score: Some(total),
                });
            }
            _ => return Ok(total),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NodeKind, State};

    #[test]
    fn recovers_two_node_dependency() {
        let mut net = Network::new("t");
        let two = || vec![State::new("a"), State::new("b")];
        let x = net.add_node("X", NodeKind::Chance, two()).unwrap();
        let y = net.add_node("Y", NodeKind::Chance, two()).unwrap();
        let mut rows = vec![];
        for _ in 0..50 {
            rows.push(vec![Some(0), Some(0)]);
            rows.push(vec![Some(1), Some(1)]);
        }
        for _ in 0..5 {
            rows.push(vec![Some(0), Some(1)]);
        }
        let n = rows.len();
        let cases = CaseSet { nodes: vec![x, y], rows, weights: vec![1.0; n] };
        let report = learn_hill_climb(
            &mut net,
            &cases,
            &[x, y],
            &HillClimbOptions::default(),
            &SearchCtrl::default(),
        )
        .unwrap();
        assert_eq!(report.edges.len(), 1, "one edge between dependent vars");
        assert_eq!(net.edges().len(), 1);
    }

    /// Helper: strongly dependent (X,Y) case set — 90% correlated.
    fn dependent_cases(x: NodeId, y: NodeId) -> CaseSet {
        let mut rows = vec![];
        for _ in 0..50 {
            rows.push(vec![Some(0), Some(0)]);
            rows.push(vec![Some(1), Some(1)]);
        }
        for _ in 0..5 {
            rows.push(vec![Some(0), Some(1)]);
        }
        let n = rows.len();
        CaseSet { nodes: vec![x, y], rows, weights: vec![1.0; n] }
    }

    #[test]
    fn blacklisted_edge_never_learned() {
        let mut net = Network::new("t");
        let two = || vec![State::new("a"), State::new("b")];
        let x = net.add_node("X", NodeKind::Chance, two()).unwrap();
        let y = net.add_node("Y", NodeKind::Chance, two()).unwrap();
        let cases = dependent_cases(x, y);
        // Forbid both directions — no edge should appear.
        let opts = HillClimbOptions {
            constraints: EdgeConstraints {
                forbidden: vec![(x, y), (y, x)],
                ..Default::default()
            },
            ..Default::default()
        };
        let report =
            learn_hill_climb(&mut net, &cases, &[x, y], &opts, &SearchCtrl::default())
                .unwrap();
        assert!(report.edges.is_empty(), "forbidden edge must not be learned");
        assert!(net.edges().is_empty());
    }

    #[test]
    fn whitelisted_edge_always_present() {
        let mut net = Network::new("t");
        let two = || vec![State::new("a"), State::new("b")];
        // X and Y are independent (uniform joint).
        let x = net.add_node("X", NodeKind::Chance, two()).unwrap();
        let y = net.add_node("Y", NodeKind::Chance, two()).unwrap();
        let rows: Vec<Vec<Option<usize>>> = (0..100)
            .map(|i| vec![Some(i % 2), Some((i / 2) % 2)])
            .collect();
        let n = rows.len();
        let cases = CaseSet { nodes: vec![x, y], rows, weights: vec![1.0; n] };
        // Require X→Y even though data shows independence.
        let opts = HillClimbOptions {
            constraints: EdgeConstraints {
                required: vec![(x, y)],
                ..Default::default()
            },
            ..Default::default()
        };
        let report =
            learn_hill_climb(&mut net, &cases, &[x, y], &opts, &SearchCtrl::default())
                .unwrap();
        assert!(
            report.edges.contains(&(x, y)),
            "required edge X→Y must appear in the result"
        );
    }

    #[test]
    fn validate_rejects_self_loop() {
        let mut net = Network::new("t");
        let x = net.add_node("X", NodeKind::Chance, vec![State::new("a")]).unwrap();
        let c = EdgeConstraints { required: vec![(x, x)], ..Default::default() };
        assert!(c.validate(&net).is_err(), "self-loop must be rejected");
    }

    #[test]
    fn validate_rejects_contradictory_pair() {
        let mut net = Network::new("t");
        let two = || vec![State::new("a"), State::new("b")];
        let x = net.add_node("X", NodeKind::Chance, two()).unwrap();
        let y = net.add_node("Y", NodeKind::Chance, two()).unwrap();
        let c = EdgeConstraints {
            required: vec![(x, y)],
            forbidden: vec![(x, y)],
        };
        assert!(c.validate(&net).is_err(), "same pair in both lists must be rejected");
    }

    #[test]
    fn validate_rejects_cyclic_required() {
        let mut net = Network::new("t");
        let two = || vec![State::new("a"), State::new("b")];
        let a = net.add_node("A", NodeKind::Chance, two()).unwrap();
        let b = net.add_node("B", NodeKind::Chance, two()).unwrap();
        let c_node = net.add_node("C", NodeKind::Chance, two()).unwrap();
        let c = EdgeConstraints {
            required: vec![(a, b), (b, c_node), (c_node, a)],
            ..Default::default()
        };
        assert!(c.validate(&net).is_err(), "cyclic required edges must be rejected");
    }
}
