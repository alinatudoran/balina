//! Native versioned JSON format (`.balina` / `.json`).
//! Format: `"balina-project"` v1 — a project with one or more network tabs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::IoError;
use crate::io::{Document, NodeVisual, NoteInfo, Project, ProjectSheet, VisualInfo};
use crate::model::{ContinuousInfo, Network, NodeKind, State, Table};

pub const FORMAT_TAG: &str = "balina-project";
pub const VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct ProjectDto {
    format: String,
    version: u32,
    #[serde(default)]
    active_tab: usize,
    tabs: Vec<TabDto>,
}

#[derive(Serialize, Deserialize)]
struct TabDto {
    #[serde(default)]
    label: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    comment: String,
    nodes: Vec<NodeDto>,
    #[serde(default)]
    visual: HashMap<String, NodeVisual>,
    #[serde(default)]
    notes: Vec<NoteInfo>,
}

#[derive(Serialize, Deserialize)]
struct NodeDto {
    name: String,
    #[serde(default)]
    title: String,
    kind: NodeKind,
    states: Vec<State>,
    /// Parent node names, in table-axis order.
    #[serde(default)]
    parents: Vec<String>,
    #[serde(default)]
    table: Vec<f64>,
    #[serde(default)]
    experience: Option<Vec<f64>>,
    #[serde(default)]
    comment: String,
    #[serde(default)]
    continuous: Option<ContinuousInfo>,
}

// ---------------------------------------------------------------------------
// Serialize
// ---------------------------------------------------------------------------

pub fn project_to_json(project: &Project) -> Result<String, IoError> {
    let tabs: Vec<TabDto> = project
        .sheets
        .iter()
        .map(|sheet| tab_to_dto(&sheet.doc, &sheet.label))
        .collect();
    let dto = ProjectDto {
        format: FORMAT_TAG.into(),
        version: VERSION,
        active_tab: project.active,
        tabs,
    };
    Ok(serde_json::to_string_pretty(&dto)?)
}

fn tab_to_dto(doc: &Document, label: &str) -> TabDto {
    let net = &doc.network;
    let nodes: Vec<NodeDto> = net
        .topo_order()
        .into_iter()
        .map(|id| {
            let n = net.node(id);
            NodeDto {
                name: n.name.clone(),
                title: n.title.clone(),
                kind: n.kind,
                states: n.states.clone(),
                parents: n.parents.iter().map(|&p| net.node(p).name.clone()).collect(),
                table: n.table.data.clone(),
                experience: n.experience.clone(),
                comment: n.comment.clone(),
                continuous: n.continuous.clone(),
            }
        })
        .collect();
    TabDto {
        label: label.to_string(),
        name: net.name.clone(),
        comment: net.comment.clone(),
        nodes,
        visual: doc.visual.nodes.clone(),
        notes: doc.visual.notes.clone(),
    }
}

// ---------------------------------------------------------------------------
// Deserialize
// ---------------------------------------------------------------------------

pub fn project_from_json(text: &str) -> Result<Project, IoError> {
    // Peek at the format tag to give a good error for old files.
    let peek: serde_json::Value = serde_json::from_str(text)?;
    let format = peek.get("format").and_then(|v| v.as_str()).unwrap_or("");
    if format == "balina-net" {
        return Err(IoError::Malformed(
            "this is an old single-network file (balina-net v1); \
             please convert it to the new balina-project format"
                .into(),
        ));
    }
    if format != FORMAT_TAG {
        return Err(IoError::Malformed(format!(
            "not a {FORMAT_TAG} file (format = `{format}`)"
        )));
    }

    let dto: ProjectDto = serde_json::from_str(text)?;
    let mut sheets = Vec::with_capacity(dto.tabs.len());
    for tab in dto.tabs {
        let doc = tab_from_dto(tab.name, tab.comment, tab.nodes, tab.visual, tab.notes)?;
        sheets.push(ProjectSheet { label: tab.label, doc });
    }
    Ok(Project { sheets, active: dto.active_tab })
}

fn tab_from_dto(
    name: String,
    comment: String,
    nodes: Vec<NodeDto>,
    visual: HashMap<String, NodeVisual>,
    notes: Vec<NoteInfo>,
) -> Result<Document, IoError> {
    let mut net = Network::new(name);
    net.comment = comment;
    // Pass 1: create all nodes; pass 2: edges + tables.
    for n in &nodes {
        net.add_node(&n.name, n.kind, n.states.clone())?;
    }
    for n in &nodes {
        let id = net.find_by_name(&n.name).unwrap();
        for pname in &n.parents {
            let p = net
                .find_by_name(pname)
                .ok_or_else(|| IoError::Malformed(format!("unknown parent `{pname}`")))?;
            net.add_edge(p, id)?;
        }
        if net.node(id).has_table() && !n.table.is_empty() {
            net.set_table(id, Table { data: n.table.clone() })?;
        }
        if n.experience.is_some() {
            net.set_experience(id, n.experience.clone())?;
        }
        net.set_title(id, n.title.clone());
        net.set_comment(id, n.comment.clone());
        if let Some(ci) = n.continuous.clone() {
            net.node_mut(id).continuous = Some(ci);
        }
    }
    Ok(Document { network: net, visual: VisualInfo { nodes: visual, notes } })
}

// Keep the old public names for backward compat within the crate.
pub fn to_json(doc: &Document) -> Result<String, IoError> {
    let project = Project {
        sheets: vec![ProjectSheet { label: doc.network.name.clone(), doc: doc.clone() }],
        active: 0,
    };
    project_to_json(&project)
}

pub fn from_json(text: &str) -> Result<Document, IoError> {
    let project = project_from_json(text)?;
    let idx = project.active.min(project.sheets.len().saturating_sub(1));
    Ok(project.sheets.into_iter().nth(idx).map(|s| s.doc).unwrap_or_default())
}
