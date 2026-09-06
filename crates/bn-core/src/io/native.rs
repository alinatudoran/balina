//! Native versioned JSON format (`.balina` / `.json`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::IoError;
use crate::io::{Document, NodeVisual, VisualInfo};
use crate::model::{ContinuousInfo, Network, NodeKind, State, Table};

pub const FORMAT_TAG: &str = "balina-net";
pub const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct NetDto {
    format: String,
    version: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    comment: String,
    nodes: Vec<NodeDto>,
    #[serde(default)]
    visual: HashMap<String, NodeVisual>,
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

pub fn to_json(doc: &Document) -> Result<String, IoError> {
    let net = &doc.network;
    // Topological order keeps files diffable and lets loading add edges in
    // one pass.
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
    let dto = NetDto {
        format: FORMAT_TAG.into(),
        version: VERSION,
        name: net.name.clone(),
        comment: net.comment.clone(),
        nodes,
        visual: doc.visual.nodes.clone(),
    };
    Ok(serde_json::to_string_pretty(&dto)?)
}

pub fn from_json(text: &str) -> Result<Document, IoError> {
    let dto: NetDto = serde_json::from_str(text)?;
    if dto.format != FORMAT_TAG {
        return Err(IoError::Malformed(format!(
            "not a {FORMAT_TAG} file (format = `{}`)",
            dto.format
        )));
    }
    let mut net = Network::new(dto.name);
    net.comment = dto.comment;
    // Pass 1: create all nodes; pass 2: edges + tables.
    for n in &dto.nodes {
        net.add_node(&n.name, n.kind, n.states.clone())?;
    }
    for n in &dto.nodes {
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
    Ok(Document { network: net, visual: VisualInfo { nodes: dto.visual } })
}
