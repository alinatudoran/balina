//! XDSL (GeNIe/SMILE) format. Unlike XMLBIF it represents influence
//! diagrams: `<cpt>`, `<decision>` and `<utility>` node elements, plus
//! layout in the `<extensions><genie>` block. Probability/utility values are
//! listed with parents as outer axes and the node itself varying fastest —
//! bn-core's native layout.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;

use crate::error::IoError;
use crate::io::{Document, NodeVisual, VisualInfo, Warning};
use crate::model::{Network, NodeKind, State, Table};

#[derive(Default)]
struct NodeDto {
    id: String,
    kind: Option<NodeKind>,
    states: Vec<String>,
    parents: Vec<String>,
    values: Vec<f64>,
}

pub fn from_xml(text: &str) -> Result<Document, IoError> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut net_name = String::from("network");
    let mut nodes: Vec<NodeDto> = Vec::new();
    let mut cur: Option<NodeDto> = None;
    let mut path: Vec<String> = Vec::new();
    // genie extension data: node id -> (title, position)
    let mut titles: HashMap<String, String> = HashMap::new();
    let mut positions: HashMap<String, (f32, f32)> = HashMap::new();
    let mut ext_node: Option<String> = None;

    let get_attr = |e: &quick_xml::events::BytesStart, name: &str| -> Option<String> {
        e.attributes().flatten().find_map(|a| {
            if a.key.as_ref().eq_ignore_ascii_case(name.as_bytes()) {
                Some(String::from_utf8_lossy(&a.value).to_string())
            } else {
                None
            }
        })
    };

    loop {
        match reader.read_event().map_err(|e| IoError::Xml(e.to_string()))? {
            Event::Start(e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                match tag.as_str() {
                    "smile" => {
                        if let Some(id) = get_attr(&e, "id") {
                            net_name = id;
                        }
                    }
                    "cpt" | "deterministic" => {
                        cur = Some(NodeDto {
                            id: get_attr(&e, "id").unwrap_or_default(),
                            kind: Some(NodeKind::Chance),
                            ..Default::default()
                        })
                    }
                    "decision" => {
                        cur = Some(NodeDto {
                            id: get_attr(&e, "id").unwrap_or_default(),
                            kind: Some(NodeKind::Decision),
                            ..Default::default()
                        })
                    }
                    "utility" => {
                        cur = Some(NodeDto {
                            id: get_attr(&e, "id").unwrap_or_default(),
                            kind: Some(NodeKind::Utility),
                            ..Default::default()
                        })
                    }
                    "node" => ext_node = get_attr(&e, "id"),
                    _ => {}
                }
                path.push(tag);
            }
            Event::Empty(e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                if tag == "state" {
                    if let (Some(n), Some(id)) = (cur.as_mut(), get_attr(&e, "id")) {
                        n.states.push(id);
                    }
                }
            }
            Event::Text(t) => {
                let txt = t
                    .decode()
                    .map_err(|e| IoError::Xml(e.to_string()))?
                    .trim()
                    .to_string();
                if txt.is_empty() {
                    continue;
                }
                let tag = path.last().map(|s| s.as_str()).unwrap_or("");
                match tag {
                    "parents" => {
                        if let Some(n) = cur.as_mut() {
                            n.parents = txt.split_whitespace().map(String::from).collect();
                        }
                    }
                    "probabilities" | "utilities" => {
                        if let Some(n) = cur.as_mut() {
                            for tok in txt.split_whitespace() {
                                n.values.push(tok.parse().map_err(|_| {
                                    IoError::Malformed(format!("bad value `{tok}`"))
                                })?);
                            }
                        }
                    }
                    "name" => {
                        if let Some(id) = &ext_node {
                            titles.insert(id.clone(), txt);
                        }
                    }
                    "position" => {
                        if let Some(id) = &ext_node {
                            let mut it = txt.split_whitespace().map(|s| s.parse::<f32>());
                            if let (Some(Ok(x)), Some(Ok(y))) = (it.next(), it.next()) {
                                positions.insert(id.clone(), (x, y));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                match tag.as_str() {
                    "cpt" | "deterministic" | "decision" | "utility" => {
                        if let Some(n) = cur.take() {
                            nodes.push(n);
                        }
                    }
                    "node" => ext_node = None,
                    _ => {}
                }
                path.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }

    let mut net = Network::new(net_name);
    let mut visual = VisualInfo::default();
    for n in &nodes {
        let kind = n.kind.unwrap_or(NodeKind::Chance);
        let states: Vec<State> = if kind == NodeKind::Utility {
            vec![]
        } else {
            n.states.iter().map(State::new).collect()
        };
        net.add_node(&n.id, kind, states)?;
    }
    for n in &nodes {
        let id = net.find_by_name(&n.id).unwrap();
        for pname in &n.parents {
            let p = net
                .find_by_name(pname)
                .ok_or_else(|| IoError::Malformed(format!("unknown parent `{pname}`")))?;
            net.add_edge(p, id)?;
        }
        if net.node(id).has_table() && !n.values.is_empty() {
            net.set_table(id, Table { data: n.values.clone() })?;
        }
        if let Some(t) = titles.get(&n.id) {
            net.set_title(id, t.clone());
        }
        if let Some(&(x, y)) = positions.get(&n.id) {
            visual
                .nodes
                .insert(n.id.clone(), NodeVisual { x, y, display: Default::default(), color: None });
        }
    }
    Ok(Document { network: net, visual })
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// State/node ids in XDSL must be identifier-like.
fn ident(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if out.chars().next().map_or(true, |c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

pub fn to_xml(doc: &Document) -> Result<(String, Vec<Warning>), IoError> {
    let net = &doc.network;
    let warnings = Vec::new();
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<smile version=\"1.0\" id=\"{}\" numsamples=\"10000\">\n<nodes>\n",
        esc(&ident(&net.name))
    ));
    let order = net.topo_order();
    for &id in &order {
        let n = net.node(id);
        let tag = match n.kind {
            NodeKind::Chance => "cpt",
            NodeKind::Decision => "decision",
            NodeKind::Utility => "utility",
        };
        out.push_str(&format!("  <{tag} id=\"{}\">\n", esc(&ident(&n.name))));
        if n.kind != NodeKind::Utility {
            for s in &n.states {
                out.push_str(&format!("    <state id=\"{}\" />\n", esc(&ident(&s.name))));
            }
        }
        if !n.parents.is_empty() {
            let ps: Vec<String> =
                n.parents.iter().map(|&p| ident(&net.node(p).name)).collect();
            out.push_str(&format!("    <parents>{}</parents>\n", ps.join(" ")));
        }
        match n.kind {
            NodeKind::Chance => {
                let vals: Vec<String> = n.table.data.iter().map(|v| format!("{v}")).collect();
                out.push_str(&format!(
                    "    <probabilities>{}</probabilities>\n",
                    vals.join(" ")
                ));
            }
            NodeKind::Utility => {
                let vals: Vec<String> = n.table.data.iter().map(|v| format!("{v}")).collect();
                out.push_str(&format!("    <utilities>{}</utilities>\n", vals.join(" ")));
            }
            NodeKind::Decision => {}
        }
        out.push_str(&format!("  </{tag}>\n"));
    }
    out.push_str("</nodes>\n<extensions>\n");
    out.push_str(&format!(
        "  <genie version=\"1.0\" app=\"balina\" name=\"{}\">\n",
        esc(&net.name)
    ));
    for &id in &order {
        let n = net.node(id);
        let (x, y) = doc
            .visual
            .nodes
            .get(&n.name)
            .map(|v| (v.x, v.y))
            .unwrap_or((0.0, 0.0));
        out.push_str(&format!(
            "    <node id=\"{}\"><name>{}</name><position>{} {} {} {}</position></node>\n",
            esc(&ident(&n.name)),
            esc(n.display_title()),
            x as i32,
            y as i32,
            x as i32 + 120,
            y as i32 + 60
        ));
    }
    out.push_str("  </genie>\n</extensions>\n</smile>\n");
    Ok((out, warnings))
}
