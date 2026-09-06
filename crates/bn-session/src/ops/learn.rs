//! Learning + simulation ops. CPT learning is synchronous (fast at editor
//! scale); structure learning is split into prepare / run / apply so the UI
//! can run the job body off the session (via `spawn_blocking`):
//!
//! 1. [`prepare_structure_job`] — on the session: busy check, clone the net,
//!    record `change_seq`, stash the cancel flag.
//! 2. [`run_structure_job`] — off the session: the worker body (pure w.r.t.
//!    the session), reports progress through [`JobCtx`].
//! 3. [`apply_structure_outcome`] — on the session again: staleness check,
//!    one `begin_change`, clone-swap the net (NodeIds survive, so visuals
//!    and evidence stay attached).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bn_core::learn::{self, CountingOptions, EmOptions, ImportOptions};
use bn_core::model::{Network, NodeId};
use bn_core::EdgeConstraints;

use crate::doc::Dirt;
use crate::error::CmdError;
use crate::jobs::{JobCtx, JobEvent};
use crate::session::Session;
use crate::views::{check_node, LearnCptsResult, StructureLearnResult};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LearnMethod {
    Counting,
    Em,
}

// .tsv is always tab-delimited; anything else infers from the header.
fn delim_for(path: &Path) -> Option<u8> {
    path.extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.eq_ignore_ascii_case("tsv"))
        .map(|_| b'\t')
}

// ---------------------------------------------------------------------------
// CPT learning (synchronous)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct LearnCptsOpts {
    pub method: LearnMethod,
    pub em_iters: usize,
    pub create_nodes: bool,
    pub bins: usize,
}

pub fn learn_cpts(
    s: &mut Session,
    path: PathBuf,
    opts: LearnCptsOpts,
) -> Result<LearnCptsResult, CmdError> {
    let file = std::fs::File::open(&path)?;
    let delim = delim_for(&path);
    // One undo step for node creation + learning together.
    s.doc.begin_change();
    let mut created_lines: Vec<String> = vec![];
    let cases = if opts.create_nodes {
        let import = ImportOptions { bins: opts.bins, ..Default::default() };
        match learn::import_cases_with(&mut s.doc.net, file, delim, &import) {
            Ok((cases, reports)) => {
                for r in reports.iter().filter(|r| r.created) {
                    created_lines.push(match &r.continuous {
                        Some(st) => format!(
                            "Created {} (continuous: mean={:.4}, sd={:.4}, {} bins).",
                            r.name, st.mean, st.std, r.n_states
                        ),
                        None => format!("Created {} ({} states).", r.name, r.n_states),
                    });
                }
                cases
            }
            Err(e) => {
                s.doc.undo();
                return Err(e.into());
            }
        }
    } else {
        match learn::read_cases_with(&s.doc.net, file, delim) {
            Ok(c) => c,
            Err(e) => {
                s.doc.undo();
                return Err(e.into());
            }
        }
    };
    s.doc.ensure_visuals();
    let result: Result<String, CmdError> = match opts.method {
        LearnMethod::Counting => {
            learn::learn_counting(&mut s.doc.net, &cases, &CountingOptions::default())
                .map(|r| {
                    format!(
                        "Counting: {} node(s) updated from {} case(s).",
                        r.nodes_updated, r.cases_used
                    )
                })
                .map_err(Into::into)
        }
        LearnMethod::Em => {
            let em = EmOptions { max_iters: opts.em_iters, ..Default::default() };
            learn::learn_em(&mut s.doc.net, &cases, &em)
                .map(|r| {
                    format!(
                        "EM: {} iteration(s), final log-likelihood {:.4}{}",
                        r.iterations,
                        r.log_likelihood_trace.last().copied().unwrap_or(f64::NAN),
                        if r.conflicting_cases > 0 {
                            format!(", {} conflicting case(s) skipped", r.conflicting_cases)
                        } else {
                            String::new()
                        }
                    )
                })
                .map_err(Into::into)
        }
    };
    match result {
        Ok(summary) => {
            let report = if created_lines.is_empty() {
                summary.clone()
            } else {
                format!("{}\n{}", created_lines.join("\n"), summary)
            };
            // Node creation is structural; parameter-only learning is not.
            let dirt = if created_lines.is_empty() { Dirt::Params } else { Dirt::Structure };
            s.finish(dirt);
            Ok(LearnCptsResult { report, summary })
        }
        Err(e) => {
            s.doc.undo();
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Structure learning (background job)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StructAlgo {
    HillClimb,
    GrowShrink,
    Mmhc,
    PcStable,
    NaiveBayes,
    Tan,
    StructuralEm,
}

impl StructAlgo {
    fn short_name(self) -> &'static str {
        match self {
            StructAlgo::HillClimb => "hill climbing",
            StructAlgo::GrowShrink => "grow-shrink",
            StructAlgo::Mmhc => "MMHC",
            StructAlgo::PcStable => "PC-stable",
            StructAlgo::NaiveBayes => "naive Bayes",
            StructAlgo::Tan => "TAN",
            StructAlgo::StructuralEm => "structural EM",
        }
    }
    fn needs_class(self) -> bool {
        matches!(self, StructAlgo::NaiveBayes | StructAlgo::Tan)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ScoreChoice {
    Bic,
    Bdeu,
}

#[derive(Clone, Debug)]
pub struct StructureLearnOpts {
    pub path: PathBuf,
    pub algo: StructAlgo,
    pub score: ScoreChoice,
    pub ess: f64,
    pub max_parents: usize,
    pub alpha: f64,
    /// Required by NaiveBayes/Tan.
    pub class_node: Option<NodeId>,
    pub param_method: LearnMethod,
    pub em_iters: usize,
    /// Required edges (whitelist): must appear in the learned DAG.
    pub required_edges: Vec<(NodeId, NodeId)>,
    /// Forbidden edges (blacklist): must not appear in the learned DAG.
    pub forbidden_edges: Vec<(NodeId, NodeId)>,
}

/// What the worker sends back; built entirely off the session lock.
pub enum StructureOutcome {
    Done {
        /// The document's network clone with rewired edges + fitted CPTs.
        net: Network,
        report: String,
        summary: String,
        warnings: Vec<String>,
    },
    Cancelled,
    Failed(String),
}

/// Everything the worker body needs, captured under the lock.
pub struct StructureJobInput {
    pub net: Network,
    pub started_seq: u64,
    pub cancel: Arc<AtomicBool>,
    pub class: Option<NodeId>,
    pub opts: StructureLearnOpts,
}

pub fn prepare_structure_job(
    s: &mut Session,
    opts: StructureLearnOpts,
) -> Result<StructureJobInput, CmdError> {
    if s.job_cancel.is_some() {
        return Err(CmdError::Busy("a learning job is already running".into()));
    }
    let class = match (opts.class_node, opts.algo.needs_class()) {
        (Some(c), _) => {
            check_node(&s.doc.net, c)?;
            Some(c)
        }
        (None, true) => return Err(CmdError::BadRequest("this algorithm needs a class node".into())),
        (None, false) => None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    s.job_cancel = Some(cancel.clone());
    Ok(StructureJobInput {
        net: s.doc.net.clone(),
        started_seq: s.doc.change_seq,
        cancel,
        class,
        opts,
    })
}

pub fn cancel_structure_job(s: &mut Session) {
    if let Some(c) = &s.job_cancel {
        c.store(true, Ordering::Relaxed);
    }
}

/// Apply a finished job's outcome. Always clears the busy slot.
pub fn apply_structure_outcome(
    s: &mut Session,
    outcome: StructureOutcome,
    started_seq: u64,
) -> Result<StructureLearnResult, CmdError> {
    s.job_cancel = None;
    match outcome {
        StructureOutcome::Done { net, report, summary, warnings } => {
            if s.doc.change_seq != started_seq {
                return Err(CmdError::Stale(
                    "network was edited while learning ran — result discarded; run again".into(),
                ));
            }
            s.doc.begin_change();
            s.doc.net = net; // clone shares NodeIds with visual/evidence
            s.doc.ensure_visuals();
            s.finish(Dirt::Structure);
            Ok(StructureLearnResult { report, summary, warnings })
        }
        StructureOutcome::Cancelled => Err(CmdError::Cancelled),
        StructureOutcome::Failed(e) => Err(CmdError::Learn(e)),
    }
}

/// The worker body: owns a network clone, never touches the session.
pub fn run_structure_job(input: StructureJobInput, jc: &JobCtx) -> StructureOutcome {
    use bn_core::learn::structure as st;
    use bn_core::LearnError;

    let StructureJobInput { mut net, opts, class, .. } = input;
    let path = opts.path.clone();
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => return StructureOutcome::Failed(format!("cannot open file: {e}")),
    };
    jc.report(JobEvent { frac: None, text: "reading cases…".into() });
    let cases = match learn::read_cases_with(&net, file, delim_for(&path)) {
        Ok(c) => c,
        Err(e) => return StructureOutcome::Failed(format!("cannot read cases: {e}")),
    };
    // Targets: chance nodes with a case column, in name order.
    let targets: Vec<NodeId> = {
        let mut t: Vec<NodeId> = st::default_targets(&net)
            .into_iter()
            .filter(|id| cases.nodes.contains(id))
            .collect();
        if let Some(c) = class
            && !t.contains(&c)
        {
            return StructureOutcome::Failed(format!(
                "class node `{}` has no column in the case file",
                net.node(c).name
            ));
        }
        t.sort_by(|&a, &b| net.node(a).name.cmp(&net.node(b).name));
        t
    };
    let progress = |p: st::Progress| {
        jc.report(JobEvent {
            frac: (p.total > 0).then(|| (p.done as f32 / p.total as f32).min(1.0)),
            text: match p.score {
                Some(sc) => format!("{}: {} (score {sc:.1})", p.phase, p.done),
                None => format!("{}: {}", p.phase, p.done),
            },
        });
    };
    let ctrl = st::SearchCtrl { progress: Some(&progress), cancel: Some(&jc.cancel) };
    let score = match opts.score {
        ScoreChoice::Bic => st::ScoreKind::Bic,
        ScoreChoice::Bdeu => st::ScoreKind::Bdeu { ess: opts.ess },
    };
    let missing_data = cases.rows.iter().any(|r| r.iter().any(|v| v.is_none()));

    let constraints = EdgeConstraints {
        required: opts.required_edges.clone(),
        forbidden: opts.forbidden_edges.clone(),
    };
    let mut warnings: Vec<String> = vec![];
    let mut lines: Vec<String> = vec![];
    let mut sem_fitted = false;
    let result: Result<String, LearnError> = (|| {
        match opts.algo {
            StructAlgo::HillClimb => {
                let ho = st::HillClimbOptions {
                    score,
                    max_parents: opts.max_parents,
                    constraints: constraints.clone(),
                    ..Default::default()
                };
                let r = st::learn_hill_climb(&mut net, &cases, &targets, &ho, &ctrl)?;
                Ok(format!(
                    "Hill climbing: {} edge(s), score {:.2}, {} move(s), {} restart(s).",
                    r.edges.len(),
                    r.score,
                    r.iterations,
                    r.restarts
                ))
            }
            StructAlgo::GrowShrink => {
                let go = st::GsOptions {
                    ci: st::CiOptions { alpha: opts.alpha, ..st::GsOptions::default().ci },
                    hill: st::HillClimbOptions {
                        score,
                        max_parents: opts.max_parents,
                        ..Default::default()
                    },
                    constraints: constraints.clone(),
                };
                let r = st::learn_gs(&mut net, &cases, &targets, &go, &ctrl)?;
                for (id, b) in &r.blankets {
                    let names: Vec<&str> = b.iter().map(|&x| net.node(x).name.as_str()).collect();
                    lines.push(format!("  MB({}) = {{{}}}", net.node(*id).name, names.join(", ")));
                }
                Ok(format!(
                    "Grow-Shrink hybrid: {} edge(s), score {:.2}, {} CI test(s).",
                    r.hill.edges.len(),
                    r.hill.score,
                    r.ci_tests
                ))
            }
            StructAlgo::Mmhc => {
                let mo = st::MmhcOptions {
                    ci: st::CiOptions { alpha: opts.alpha, ..st::MmhcOptions::default().ci },
                    hill: st::HillClimbOptions {
                        score,
                        max_parents: opts.max_parents,
                        ..Default::default()
                    },
                    constraints: constraints.clone(),
                };
                let r = st::learn_mmhc(&mut net, &cases, &targets, &mo, &ctrl)?;
                for (id, c) in &r.cpcs {
                    let names: Vec<&str> =
                        c.iter().map(|&x| net.node(x).name.as_str()).collect();
                    lines.push(format!(
                        "  CPC({}) = {{{}}}",
                        net.node(*id).name,
                        names.join(", ")
                    ));
                }
                Ok(format!(
                    "MMHC: {} edge(s), score {:.2}, {} CI test(s).",
                    r.hill.edges.len(),
                    r.hill.score,
                    r.ci_tests
                ))
            }
            StructAlgo::PcStable => {
                let po = st::PcOptions {
                    ci: st::CiOptions { alpha: opts.alpha, ..Default::default() },
                    constraints: constraints.clone(),
                };
                let r = st::learn_pc(&mut net, &cases, &targets, &po, &ctrl)?;
                for &(a, b) in &r.cpdag.undirected {
                    warnings.push(format!(
                        "edge {} — {} is reversible; direction chosen arbitrarily",
                        net.node(a).name,
                        net.node(b).name
                    ));
                }
                if r.forced_orientations > 0 || r.v_structure_conflicts > 0 {
                    warnings.push(format!(
                        "CI tests were inconsistent ({} forced orientation(s), {} \
                         v-structure conflict(s))",
                        r.forced_orientations, r.v_structure_conflicts
                    ));
                }
                Ok(format!(
                    "PC-stable: {} compelled + {} reversible edge(s), {} CI test(s).",
                    r.cpdag.directed.len(),
                    r.cpdag.undirected.len(),
                    r.ci_tests
                ))
            }
            StructAlgo::NaiveBayes => {
                if !constraints.required.is_empty() || !constraints.forbidden.is_empty() {
                    warnings.push(
                        "edge constraints are ignored for Naive Bayes / TAN".into(),
                    );
                }
                let class = class.unwrap();
                st::learn_naive_bayes(&mut net, &targets, class)?;
                Ok(format!(
                    "Naive Bayes: {} feature(s) under class `{}`.",
                    targets.len() - 1,
                    net.node(class).name
                ))
            }
            StructAlgo::Tan => {
                // Warning already emitted by NaiveBayes arm if needed; TAN
                // shares the same NaiveBayes warning path via the flag below.
                if !constraints.required.is_empty() || !constraints.forbidden.is_empty() {
                    // Only warn if NaiveBayes didn't already (different arm).
                    warnings.push(
                        "edge constraints are ignored for Naive Bayes / TAN".into(),
                    );
                }
                let class = class.unwrap();
                let r = st::learn_tan(
                    &mut net,
                    &cases,
                    &targets,
                    class,
                    &st::TanOptions::default(),
                    &ctrl,
                )?;
                for ((a, b), w) in r.tree_edges.iter().zip(&r.cmi) {
                    lines.push(format!(
                        "  tree: {} → {} (CMI {:.4})",
                        net.node(*a).name,
                        net.node(*b).name,
                        w
                    ));
                }
                Ok(format!(
                    "TAN: class `{}` + {} tree edge(s).",
                    net.node(class).name,
                    r.tree_edges.len()
                ))
            }
            StructAlgo::StructuralEm => {
                sem_fitted = true;
                let so = st::SemOptions {
                    hill: st::HillClimbOptions {
                        score,
                        max_parents: opts.max_parents.min(3),
                        random_restarts: 0,
                        ..Default::default()
                    },
                    em: EmOptions { max_iters: opts.em_iters, ..Default::default() },
                    constraints: constraints.clone(),
                    ..Default::default()
                };
                let r = st::learn_structural_em(&mut net, &cases, &targets, &so, &ctrl)?;
                Ok(format!(
                    "Structural EM: {} edge(s), {} outer iteration(s), log-likelihood {:.4}.",
                    r.edges.len(),
                    r.outer_iterations,
                    r.log_likelihood
                ))
            }
        }
    })();
    let algo_msg = match result {
        Ok(m) => m,
        Err(LearnError::Cancelled) => return StructureOutcome::Cancelled,
        Err(e) => return StructureOutcome::Failed(e.to_string()),
    };
    if jc.cancelled() {
        return StructureOutcome::Cancelled;
    }
    // Fit parameters (Structural EM already did).
    let fit_msg = if sem_fitted {
        String::new()
    } else {
        jc.report(JobEvent { frac: None, text: "fitting parameters…".into() });
        let method = if missing_data { LearnMethod::Em } else { opts.param_method };
        if missing_data && opts.param_method == LearnMethod::Counting {
            warnings.push("data has missing values; fitted parameters with EM".into());
        }
        let fitted = match method {
            LearnMethod::Counting => {
                learn::learn_counting(&mut net, &cases, &CountingOptions::default()).map(|r| {
                    format!(
                        "Parameters: counting, {} node(s) from {} case(s).",
                        r.nodes_updated, r.cases_used
                    )
                })
            }
            LearnMethod::Em => learn::learn_em(
                &mut net,
                &cases,
                &EmOptions { max_iters: opts.em_iters, ..Default::default() },
            )
            .map(|r| {
                format!(
                    "Parameters: EM, {} iteration(s), log-likelihood {:.4}.",
                    r.iterations,
                    r.log_likelihood_trace.last().copied().unwrap_or(f64::NAN)
                )
            }),
        };
        match fitted {
            Ok(m) => m,
            Err(e) => return StructureOutcome::Failed(format!("parameter fit failed: {e}")),
        }
    };
    if jc.cancelled() {
        return StructureOutcome::Cancelled;
    }
    // Resolve edge names against the worker's own clone.
    let mut edge_lines: Vec<String> = net
        .edges()
        .into_iter()
        .map(|(p, c)| format!("  {} → {}", net.node(p).name, net.node(c).name))
        .collect();
    edge_lines.sort();
    let mut report = algo_msg.clone();
    if !fit_msg.is_empty() {
        report.push('\n');
        report.push_str(&fit_msg);
    }
    if !lines.is_empty() {
        report.push('\n');
        report.push_str(&lines.join("\n"));
    }
    report.push_str("\nLinks:\n");
    report.push_str(&edge_lines.join("\n"));
    let summary = format!(
        "Structure learning ({}): {}, {} case column(s) used.",
        opts.algo.short_name(),
        algo_msg,
        cases.nodes.len()
    );
    StructureOutcome::Done { net, report, summary, warnings }
}

// ---------------------------------------------------------------------------
// Case simulation
// ---------------------------------------------------------------------------

/// Takes a network clone so the UI can run it off the session.
pub fn simulate_cases_to_file(
    net: &Network,
    path: PathBuf,
    n: usize,
    missing_pct: f64,
) -> Result<String, CmdError> {
    let mut rng = rand::rng();
    let cases = bn_core::sample::generate_cases(net, n, missing_pct / 100.0, &mut rng);
    let file = std::fs::File::create(&path)?;
    learn::write_cases(net, &cases, file).map_err(|e| CmdError::Io(e.to_string()))?;
    Ok(format!("Wrote {} cases to {}", n, path.display()))
}
