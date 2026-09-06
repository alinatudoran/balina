//! Max-Min Hill Climbing (MMHC): MMPC constraint-based skeleton, AND-rule
//! symmetrization, score-based hill climb restricted to the skeleton.
//! (Tsamardinos, Brown & Aliferis 2006).

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
pub struct MmhcOptions {
    pub ci: CiOptions,
    pub hill: HillClimbOptions,
    /// Hard structural constraints (required / forbidden edges).
    pub constraints: EdgeConstraints,
}

impl Default for MmhcOptions {
    fn default() -> Self {
        MmhcOptions {
            ci: CiOptions { max_cond: 8, ..CiOptions::default() },
            hill: HillClimbOptions::default(),
            constraints: EdgeConstraints::default(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MmhcReport {
    /// CPC sets found per target (before AND-rule symmetrization).
    pub cpcs: Vec<(NodeId, Vec<NodeId>)>,
    pub ci_tests: usize,
    pub hill: HillClimbReport,
}

pub fn learn_mmhc(
    net: &mut Network,
    cases: &CaseSet,
    targets: &[NodeId],
    opts: &MmhcOptions,
    ctrl: &SearchCtrl,
) -> Result<MmhcReport, LearnError> {
    opts.constraints.validate(net)?;
    let targets = check_targets(targets, 2)?;
    let (req, forb) = opts.constraints.to_ix(&targets);
    let mut view = DataView::new(net, cases, &targets)?;
    let mut report = MmhcReport::default();

    let cpcs = {
        let mut tester = CiTester::new(&view, opts.ci.clone());
        let c = cpcs_ix(&view, &mut tester, ctrl)?;
        report.ci_tests = tester.tests_run;
        c
    };
    report.cpcs = cpcs
        .iter()
        .enumerate()
        .map(|(i, c)| (targets[i], c.iter().map(|&j| targets[j as usize]).collect()))
        .collect();

    // AND-rule: i may parent j only when both are in each other's CPC.
    let n = targets.len();
    let mut allowed: Vec<Vec<u32>> = (0..n)
        .map(|j| {
            (0..n as u32)
                .filter(|&i| {
                    i as usize != j
                        && cpcs[j].contains(&i)
                        && cpcs[i as usize].contains(&(j as u32))
                })
                .collect()
        })
        .collect();
    // Apply constraints: forbidden pairs removed, required pairs added.
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
    let start =
        if opts.hill.start_from_current { template.clone() } else { template.cleared() };
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

/// MMPC per target variable, in index space. Returns CPC sets.
fn cpcs_ix(
    view: &DataView,
    tester: &mut CiTester,
    ctrl: &SearchCtrl,
) -> Result<Vec<Vec<u32>>, LearnError> {
    let n = view.n_vars();
    let mut out = Vec::with_capacity(n);
    for t in 0..n as u32 {
        ctrl.check()?;
        ctrl.tick(Progress { phase: "MMPC", done: t as usize, total: n, score: None });

        // f[x] = min CMI(t, x, S) seen so far; initialized to unconditional CMI.
        let mut f: Vec<(f64, u32)> = (0..n as u32)
            .filter(|&x| x != t)
            .map(|x| {
                let (mi, _) = tester.cmi(t, x, &[]).unwrap_or((0.0, 1.0));
                (mi, x)
            })
            .collect();

        let mut cpc: Vec<u32> = vec![];
        // Candidates still under consideration (those not yet confirmed or rejected).
        let mut cands: Vec<u32> = (0..n as u32).filter(|&x| x != t).collect();

        // Grow.
        loop {
            ctrl.check()?;
            if cpc.len() >= tester.opts.max_cond {
                break;
            }
            // x* = argmax f among candidates not already in CPC.
            let xstar = cands
                .iter()
                .copied()
                .filter(|x| !cpc.contains(x))
                .max_by(|&a, &b| {
                    let fa = f.iter().find(|&&(_, x)| x == a).map_or(0.0, |&(v, _)| v);
                    let fb = f.iter().find(|&&(_, x)| x == b).map_or(0.0, |&(v, _)| v);
                    fa.total_cmp(&fb)
                });
            let Some(xstar) = xstar else { break };
            let fstar = f.iter().find(|&&(_, x)| x == xstar).map_or(0.0, |&(v, _)| v);
            if fstar <= 0.0 {
                break;
            }
            let (ind, _) = tester.independent(t, xstar, &cpc)?;
            if !ind {
                // x* is a genuine dependency — add to CPC and update f.
                cpc.push(xstar);
                for x in cands.iter().copied().filter(|&x| x != xstar && !cpc.contains(&x)) {
                    let (mi, _) = tester.cmi(t, x, &[xstar])?;
                    if let Some(e) = f.iter_mut().find(|(_, xi)| *xi == x) {
                        if mi < e.0 {
                            e.0 = mi;
                        }
                    }
                }
            } else {
                // t ⊥ x* | CPC → not a PC; remove from candidates.
                cands.retain(|&x| x != xstar);
            }
        }

        // Shrink: drop x if t ⊥ x | (CPC \ {x}).
        loop {
            let mut shrunk = false;
            let members = cpc.clone();
            for &x in &members {
                let rest: Vec<u32> = cpc.iter().copied().filter(|&y| y != x).collect();
                let (ind, _) = tester.independent(t, x, &rest)?;
                if ind {
                    cpc.retain(|&y| y != x);
                    shrunk = true;
                }
            }
            if !shrunk {
                break;
            }
        }
        cpc.sort_unstable();
        out.push(cpc);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NodeKind, State};

    #[test]
    fn mmhc_recovers_two_node_dependency() {
        let mut net = crate::model::Network::new("t");
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
        let report = learn_mmhc(
            &mut net,
            &cases,
            &[x, y],
            &MmhcOptions::default(),
            &SearchCtrl::default(),
        )
        .unwrap();
        assert_eq!(report.hill.edges.len(), 1, "one edge between strongly dependent vars");
        assert_eq!(net.edges().len(), 1);
    }
}
