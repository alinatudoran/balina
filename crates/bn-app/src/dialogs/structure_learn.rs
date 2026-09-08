//! Structure learning: 7 algorithms, background job with live progress and
//! cancel. Editing the network while a job runs discards its result (the
//! session enforces this via change_seq).
//!
//! Async pattern: prepare (scoped SESSION write) → `platform::run_structure_job`
//! (desktop: `spawn_blocking`; web: a Web Worker — no signal borrow held
//! across any `.await`) → apply (scoped write again, staleness-checked).
//! Progress flows into JOB_PROGRESS on the UI scheduler only.

use bn_core::model::{NodeId, NodeKind};
use bn_session::jobs::JobEvent;
use bn_session::ops::learn::{LearnMethod, ScoreChoice, StructAlgo, StructureLearnOpts};
use bn_session::CmdError;
use dioxus::prelude::*;

use crate::dialogs::learn_cpts::file_label;
use crate::platform::{pick_case_file, CaseFile};
use crate::state::{
    close_dialog, exec, exec_res, log_message, JOB_PROGRESS, LAST_CASE_FILE, SESSION,
};
use crate::ui::{Modal, NumberInput, BTN, BTN_PRIMARY, BTN_SM, SELECT};

struct AlgoCfg {
    value: StructAlgo,
    label: &'static str,
    score: bool,
    alpha: bool,
    class: bool,
}

const ALGOS: [AlgoCfg; 7] = [
    AlgoCfg { value: StructAlgo::HillClimb, label: "Hill climbing (score-based)", score: true, alpha: false, class: false },
    AlgoCfg { value: StructAlgo::GrowShrink, label: "Grow-Shrink hybrid (blankets + score)", score: true, alpha: true, class: false },
    AlgoCfg { value: StructAlgo::Mmhc, label: "MMHC (Max-Min hill climbing)", score: true, alpha: true, class: false },
    AlgoCfg { value: StructAlgo::PcStable, label: "PC-stable (causal discovery)", score: false, alpha: true, class: false },
    AlgoCfg { value: StructAlgo::NaiveBayes, label: "Naive Bayes (classifier)", score: false, alpha: false, class: true },
    AlgoCfg { value: StructAlgo::Tan, label: "Tree-augmented Naive Bayes (TAN)", score: false, alpha: false, class: true },
    AlgoCfg { value: StructAlgo::StructuralEm, label: "Structural EM (missing data)", score: true, alpha: false, class: false },
];

#[component]
pub fn StructureLearnDialog() -> Element {
    let mut file: Signal<Option<CaseFile>> = use_signal(|| LAST_CASE_FILE.peek().clone());
    let mut algo = use_signal(|| StructAlgo::HillClimb);
    let mut score = use_signal(|| ScoreChoice::Bic);
    let mut ess = use_signal(|| 1.0f64);
    let mut max_parents = use_signal(|| 4usize);
    let mut alpha = use_signal(|| 0.05f64);
    let mut class_node: Signal<Option<NodeId>> = use_signal(|| None);
    let mut param_method = use_signal(|| LearnMethod::Counting);
    let mut em_iters = use_signal(|| 50usize);
    let mut running = use_signal(|| false);
    let mut report: Signal<Option<String>> = use_signal(|| None);
    // Edge constraints (whitelist / blacklist).
    let mut required_edges: Signal<Vec<(NodeId, NodeId)>> = use_signal(Vec::new);
    let mut forbidden_edges: Signal<Vec<(NodeId, NodeId)>> = use_signal(Vec::new);
    let mut new_req_from: Signal<Option<NodeId>> = use_signal(|| None);
    let mut new_req_to: Signal<Option<NodeId>> = use_signal(|| None);
    let mut new_forb_from: Signal<Option<NodeId>> = use_signal(|| None);
    let mut new_forb_to: Signal<Option<NodeId>> = use_signal(|| None);

    let cfg = ALGOS.iter().find(|a| a.value == *algo.read()).unwrap();
    let chance_nodes: Vec<(NodeId, String)> = {
        let s = SESSION.read();
        s.doc
            .net
            .nodes()
            .filter(|(_, n)| n.kind == NodeKind::Chance)
            .map(|(id, n)| (id, n.name.clone()))
            .collect()
    };
    // The class node may vanish via undo.
    let class_valid =
        class_node.read().is_some_and(|c| chance_nodes.iter().any(|(id, _)| *id == c));
    let can_run = file.read().is_some()
        && chance_nodes.len() >= 2
        && (!cfg.class || class_valid)
        && !*running.read();

    let needs_class = cfg.class;
    let run = move |_| {
        let Some(case_file) = file.read().clone() else { return };
        let opts = StructureLearnOpts {
            algo: *algo.read(),
            score: *score.read(),
            ess: *ess.read(),
            max_parents: *max_parents.read(),
            alpha: *alpha.read(),
            class_node: if needs_class { *class_node.read() } else { None },
            param_method: *param_method.read(),
            em_iters: *em_iters.read(),
            required_edges: required_edges.read().clone(),
            forbidden_edges: forbidden_edges.read().clone(),
        };
        running.set(true);
        report.set(None);
        *JOB_PROGRESS.write() = Some(JobEvent { frac: None, text: "starting…".into() });
        spawn(async move {
            let input =
                match exec_res(|s| bn_session::ops::learn::prepare_structure_job(s, opts)) {
                    Ok(i) => i,
                    Err(e) => {
                        report.set(Some(e.to_string()));
                        running.set(false);
                        *JOB_PROGRESS.write() = None;
                        return;
                    }
                };
            let started_seq = input.started_seq;
            match crate::platform::run_structure_job(input, case_file).await {
                Ok(outcome) => {
                    match exec_res(|s| {
                        crate::platform::apply_structure_outcome(s, outcome, started_seq)
                    }) {
                        Ok(res) => {
                            for w in &res.warnings {
                                log_message(format!("Warning: {w}"));
                            }
                            log_message(res.summary.clone());
                            report.set(Some(res.report));
                            *crate::canvas::controller::FIT_REQUEST.write() += 1;
                        }
                        Err(CmdError::Cancelled) => report.set(Some("Cancelled.".into())),
                        Err(e @ CmdError::Stale(_)) => report.set(Some(e.to_string())),
                        Err(e) => report.set(Some(format!("Learning failed: {e}"))),
                    }
                }
                Err(e) => {
                    SESSION.write().job_cancel = None; // clear the busy slot
                    report.set(Some(format!("Learning crashed: {e}")));
                }
            }
            running.set(false);
            *JOB_PROGRESS.write() = None;
        });
    };

    let progress = JOB_PROGRESS.read().clone();
    let chance_for_select = chance_nodes.clone();
    // Pre-clones for move closures in the constraint UI.
    let cn_req_from = chance_nodes.clone();
    let cn_req_to = chance_nodes.clone();
    let cn_forb_from = chance_nodes.clone();
    let cn_forb_to = chance_nodes.clone();

    rsx! {
        Modal {
            title: "Learn structure from cases",
            width: "max-w-lg",
            draggable: false, // configuration dialog
            on_close: move |_| close_dialog(),
            footer: rsx! {
                if *running.read() {
                    button {
                        class: BTN,
                        onclick: move |_| {
                            exec(|s| {
                                bn_session::ops::learn::cancel_structure_job(s);
                                Ok(())
                            });
                        },
                        "Cancel job"
                    }
                }
                button { class: BTN, onclick: move |_| close_dialog(), "Close" }
                button { class: BTN_PRIMARY, disabled: !can_run, onclick: run, "Learn" }
            },
            div { class: "space-y-3 text-sm",
                div { class: "flex items-center gap-2",
                    button {
                        class: BTN_SM,
                        disabled: *running.read(),
                        onclick: move |_| {
                            spawn(async move {
                                if let Some(f) = pick_case_file().await {
                                    file.set(Some(f));
                                }
                            });
                        },
                        "Choose CSV/TSV…"
                    }
                    span { class: "truncate font-mono text-xs text-muted-foreground",
                        "{file_label(&file.read())}"
                    }
                }
                div { class: "flex items-center gap-2",
                    label { class: "w-24", "Algorithm:" }
                    select {
                        class: "{SELECT} flex-1",
                        disabled: *running.read(),
                        onchange: move |ev| {
                            if let Ok(i) = ev.value().parse::<usize>() {
                                algo.set(ALGOS[i].value);
                            }
                        },
                        for (i, a) in ALGOS.iter().enumerate() {
                            option { value: "{i}", selected: *algo.read() == a.value, "{a.label}" }
                        }
                    }
                }
                if cfg.score {
                    div { class: "flex items-center gap-3",
                        label { class: "w-24", "Score:" }
                        for (sc, sid, label) in [
                            (ScoreChoice::Bic, "sl-bic", "BIC"),
                            (ScoreChoice::Bdeu, "sl-bdeu", "BDeu"),
                        ] {
                            div { class: "flex items-center gap-1.5",
                                input {
                                    id: sid,
                                    r#type: "radio",
                                    name: "sl-score",
                                    checked: *score.read() == sc,
                                    onchange: move |_| score.set(sc),
                                }
                                label { r#for: sid, "{label}" }
                            }
                        }
                        if *score.read() == ScoreChoice::Bdeu {
                            label { "ESS:" }
                            NumberInput {
                                step: "0.1".to_string(),
                                min: "0.1".to_string(),
                                max: "100".to_string(),
                                value: ess.read().to_string(),
                                onchange: move |v: String| {
                                    if let Ok(v) = v.parse::<f64>() {
                                        ess.set(v.clamp(0.1, 100.0));
                                    }
                                },
                            }
                        }
                    }
                    div { class: "flex items-center gap-2",
                        label { class: "w-24", "Max parents:" }
                        NumberInput {
                            min: "1".to_string(),
                            max: "8".to_string(),
                            value: max_parents.read().to_string(),
                            onchange: move |v: String| {
                                if let Ok(v) = v.parse::<usize>() {
                                    max_parents.set(v.clamp(1, 8));
                                }
                            },
                        }
                    }
                }
                if cfg.alpha {
                    div { class: "flex items-center gap-2",
                        label { class: "w-24", "CI α:" }
                        NumberInput {
                            class: "h-7 w-20".to_string(),
                            step: "0.01".to_string(),
                            // min is the HTML step base: 0.001 would snap the
                            // stepper to 0.011/0.021/… — keep it 0 and let the
                            // onchange clamp enforce the real α ≥ 0.001 bound.
                            min: "0".to_string(),
                            max: "0.5".to_string(),
                            value: alpha.read().to_string(),
                            onchange: move |v: String| {
                                if let Ok(v) = v.parse::<f64>() {
                                    alpha.set(v.clamp(0.001, 0.5));
                                }
                            },
                        }
                    }
                }
                if cfg.class {
                    div { class: "flex items-center gap-2",
                        label { class: "w-24", "Class node:" }
                        select {
                            class: "{SELECT} w-44",
                            disabled: *running.read(),
                            onchange: move |ev| {
                                class_node.set(
                                    ev.value()
                                        .parse::<usize>()
                                        .ok()
                                        .and_then(|i| chance_for_select.get(i).map(|(id, _)| *id)),
                                );
                            },
                            option { value: "none", selected: class_node.read().is_none(), "—" }
                            for (i, (id, name)) in chance_nodes.iter().enumerate() {
                                option {
                                    value: "{i}",
                                    selected: *class_node.read() == Some(*id),
                                    "{name}"
                                }
                            }
                        }
                    }
                }
                div { class: "flex items-center gap-3",
                    label { class: "w-24", "Parameters:" }
                    for (m, mid, label) in [
                        (LearnMethod::Counting, "sl-counting", "Counting"),
                        (LearnMethod::Em, "sl-em", "EM"),
                    ] {
                        div { class: "flex items-center gap-1.5",
                            input {
                                id: mid,
                                r#type: "radio",
                                name: "sl-param",
                                checked: *param_method.read() == m,
                                onchange: move |_| param_method.set(m),
                            }
                            label { r#for: mid, "{label}" }
                        }
                    }
                    if *param_method.read() == LearnMethod::Em
                        || *algo.read() == StructAlgo::StructuralEm
                    {
                        label { "EM iters:" }
                        NumberInput {
                            min: "1".to_string(),
                            max: "500".to_string(),
                            value: em_iters.read().to_string(),
                            onchange: move |v: String| {
                                if let Ok(v) = v.parse::<usize>() {
                                    em_iters.set(v.clamp(1, 500));
                                }
                            },
                        }
                    }
                }
                // Edge constraints (not applicable for NB / TAN).
                if !cfg.class {
                    details {
                        summary {
                            class: "cursor-pointer select-none text-xs font-medium text-muted-foreground",
                            "Edge constraints"
                        }
                        div { class: "mt-2 space-y-3 text-xs",
                            // Required edges (whitelist).
                            div {
                                div { class: "mb-1 font-medium", "Required (whitelist):" }
                                for (i, &(p, c)) in required_edges.read().iter().enumerate() {
                                    {
                                        let pname = SESSION.read().doc.net.node(p).name.clone();
                                        let cname = SESSION.read().doc.net.node(c).name.clone();
                                        rsx! {
                                            div { class: "flex items-center gap-1",
                                                span { class: "flex-1 font-mono", "{pname} → {cname}" }
                                                button {
                                                    class: "text-muted-foreground hover:text-destructive",
                                                    disabled: *running.read(),
                                                    onclick: move |_| { required_edges.write().remove(i); },
                                                    "×"
                                                }
                                            }
                                        }
                                    }
                                }
                                div { class: "flex items-center gap-1",
                                    select {
                                        class: "h-6 rounded border bg-background px-1 text-xs",
                                        disabled: *running.read(),
                                        onchange: move |ev| {
                                            new_req_from.set(
                                                ev.value().parse::<usize>().ok()
                                                    .and_then(|i| cn_req_from.get(i).map(|(id, _)| *id)),
                                            );
                                        },
                                        option { value: "none", "From…" }
                                        for (i, (id, name)) in chance_nodes.iter().enumerate() {
                                            option {
                                                value: "{i}",
                                                selected: *new_req_from.read() == Some(*id),
                                                "{name}"
                                            }
                                        }
                                    }
                                    span { "→" }
                                    select {
                                        class: "h-6 rounded border bg-background px-1 text-xs",
                                        disabled: *running.read(),
                                        onchange: move |ev| {
                                            new_req_to.set(
                                                ev.value().parse::<usize>().ok()
                                                    .and_then(|i| cn_req_to.get(i).map(|(id, _)| *id)),
                                            );
                                        },
                                        option { value: "none", "To…" }
                                        for (i, (id, name)) in chance_nodes.iter().enumerate() {
                                            option {
                                                value: "{i}",
                                                selected: *new_req_to.read() == Some(*id),
                                                "{name}"
                                            }
                                        }
                                    }
                                    button {
                                        class: "rounded border px-2 py-0.5 text-xs hover:bg-muted disabled:opacity-40",
                                        disabled: new_req_from.read().is_none()
                                            || new_req_to.read().is_none()
                                            || *new_req_from.read() == *new_req_to.read()
                                            || *running.read(),
                                        onclick: move |_| {
                                            if let (Some(p), Some(c)) =
                                                (*new_req_from.read(), *new_req_to.read())
                                            {
                                                if p != c
                                                    && !required_edges.read().contains(&(p, c))
                                                {
                                                    required_edges.write().push((p, c));
                                                }
                                            }
                                        },
                                        "Add"
                                    }
                                }
                            }
                            // Forbidden edges (blacklist).
                            div {
                                div { class: "mb-1 font-medium", "Forbidden (blacklist):" }
                                for (i, &(p, c)) in forbidden_edges.read().iter().enumerate() {
                                    {
                                        let pname = SESSION.read().doc.net.node(p).name.clone();
                                        let cname = SESSION.read().doc.net.node(c).name.clone();
                                        rsx! {
                                            div { class: "flex items-center gap-1",
                                                span { class: "flex-1 font-mono", "{pname} → {cname}" }
                                                button {
                                                    class: "text-muted-foreground hover:text-destructive",
                                                    disabled: *running.read(),
                                                    onclick: move |_| { forbidden_edges.write().remove(i); },
                                                    "×"
                                                }
                                            }
                                        }
                                    }
                                }
                                div { class: "flex items-center gap-1",
                                    select {
                                        class: "h-6 rounded border bg-background px-1 text-xs",
                                        disabled: *running.read(),
                                        onchange: move |ev| {
                                            new_forb_from.set(
                                                ev.value().parse::<usize>().ok()
                                                    .and_then(|i| cn_forb_from.get(i).map(|(id, _)| *id)),
                                            );
                                        },
                                        option { value: "none", "From…" }
                                        for (i, (id, name)) in chance_nodes.iter().enumerate() {
                                            option {
                                                value: "{i}",
                                                selected: *new_forb_from.read() == Some(*id),
                                                "{name}"
                                            }
                                        }
                                    }
                                    span { "→" }
                                    select {
                                        class: "h-6 rounded border bg-background px-1 text-xs",
                                        disabled: *running.read(),
                                        onchange: move |ev| {
                                            new_forb_to.set(
                                                ev.value().parse::<usize>().ok()
                                                    .and_then(|i| cn_forb_to.get(i).map(|(id, _)| *id)),
                                            );
                                        },
                                        option { value: "none", "To…" }
                                        for (i, (id, name)) in chance_nodes.iter().enumerate() {
                                            option {
                                                value: "{i}",
                                                selected: *new_forb_to.read() == Some(*id),
                                                "{name}"
                                            }
                                        }
                                    }
                                    button {
                                        class: "rounded border px-2 py-0.5 text-xs hover:bg-muted disabled:opacity-40",
                                        disabled: new_forb_from.read().is_none()
                                            || new_forb_to.read().is_none()
                                            || *new_forb_from.read() == *new_forb_to.read()
                                            || *running.read(),
                                        onclick: move |_| {
                                            if let (Some(p), Some(c)) =
                                                (*new_forb_from.read(), *new_forb_to.read())
                                            {
                                                if p != c
                                                    && !forbidden_edges.read().contains(&(p, c))
                                                {
                                                    forbidden_edges.write().push((p, c));
                                                }
                                            }
                                        },
                                        "Add"
                                    }
                                }
                            }
                        }
                    }
                }
                // Explain WHY Learn is disabled — structure learning finds
                // links between EXISTING chance nodes; it never creates them.
                if chance_nodes.len() < 2 {
                    div { class: "rounded border p-2 text-xs text-amber-700",
                        "Structure learning finds links between the chance nodes already \
                         in the network (matched to case-file columns by name) — it does \
                         not create nodes. This network has {chance_nodes.len()} chance \
                         node(s); at least 2 are needed. Tip: run Cases → \"Learn CPTs \
                         from cases\" with \"Add missing nodes from columns\" first to \
                         create nodes from the file."
                    }
                } else if cfg.class && !class_valid && !*running.read() {
                    div { class: "rounded border p-2 text-xs text-amber-700",
                        "This algorithm needs a class node — pick one above."
                    }
                }
                if let Some(p) = progress {
                    div { class: "space-y-1",
                        div { class: "h-2 w-full overflow-hidden rounded bg-muted",
                            div {
                                class: "h-full bg-primary transition-all",
                                style: match p.frac {
                                    Some(f) => format!("width: {:.1}%;", f * 100.0),
                                    None => "width: 100%; opacity: 0.3;".to_string(),
                                },
                            }
                        }
                        div { class: "text-xs text-muted-foreground", "{p.text}" }
                    }
                }
                if let Some(r) = report.read().clone() {
                    pre { class: "max-h-48 overflow-auto rounded border bg-muted/30 p-2 \
                                  font-mono text-[11px] whitespace-pre-wrap select-text",
                        "{r}"
                    }
                }
            }
        }
    }
}
