//! XMLBIF v0.3 interchange format (read/write). Table values are listed with
//! parents (GIVEN order) as outer axes and the FOR variable varying fastest —
//! the same layout bn-core uses. Node positions travel in a
//! `<PROPERTY>position = (x, y)</PROPERTY>` element, as emitted by JavaBayes
//! and friends. Decision/utility nodes are not representable: saving them is
//! lossy (they are converted/skipped with a warning).

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::error::IoError;
use crate::io::{Document, NodeVisual, VisualInfo, Warning};
use crate::model::{Network, NodeKind, State, Table};

#[derive(Default)]
struct VarDto {
    name: String,
    outcomes: Vec<String>,
    position: Option<(f32, f32)>,
}

#[derive(Default)]
struct DefDto {
    for_: String,
    given: Vec<String>,
    table: Vec<f64>,
}

pub fn from_xml(text: &str) -> Result<Document, IoError> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut net_name = String::from("network");
    let mut vars: Vec<VarDto> = Vec::new();
    let mut defs: Vec<DefDto> = Vec::new();
    let mut cur_var: Option<VarDto> = None;
    let mut cur_def: Option<DefDto> = None;
    let mut path: Vec<String> = Vec::new();

    loop {
        match reader.read_event().map_err(|e| IoError::Xml(e.to_string()))? {
            Event::Start(e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_uppercase();
                match tag.as_str() {
                    "VARIABLE" => cur_var = Some(VarDto::default()),
                    "DEFINITION" | "PROBABILITY" => cur_def = Some(DefDto::default()),
                    _ => {}
                }
                path.push(tag);
            }
            Event::Empty(_) => {}
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
                let parent = path.iter().rev().nth(1).map(|s| s.as_str()).unwrap_or("");
                match (parent, tag) {
                    ("NETWORK", "NAME") => net_name = txt,
                    ("VARIABLE", "NAME") => {
                        if let Some(v) = cur_var.as_mut() {
                            v.name = txt;
                        }
                    }
                    ("VARIABLE", "OUTCOME") => {
                        if let Some(v) = cur_var.as_mut() {
                            v.outcomes.push(txt);
                        }
                    }
                    ("VARIABLE", "PROPERTY") => {
                        if let Some(v) = cur_var.as_mut() {
                            if let Some(pos) = parse_position(&txt) {
                                v.position = Some(pos);
                            }
                        }
                    }
                    (_, "FOR") => {
                        if let Some(d) = cur_def.as_mut() {
                            d.for_ = txt;
                        }
                    }
                    (_, "GIVEN") => {
                        if let Some(d) = cur_def.as_mut() {
                            d.given.push(txt);
                        }
                    }
                    (_, "TABLE") => {
                        if let Some(d) = cur_def.as_mut() {
                            for tok in txt.split_whitespace() {
                                d.table.push(tok.parse().map_err(|_| {
                                    IoError::Malformed(format!("bad table value `{tok}`"))
                                })?);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_uppercase();
                match tag.as_str() {
                    "VARIABLE" => {
                        if let Some(v) = cur_var.take() {
                            vars.push(v);
                        }
                    }
                    "DEFINITION" | "PROBABILITY" => {
                        if let Some(d) = cur_def.take() {
                            defs.push(d);
                        }
                    }
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
    for v in &vars {
        if v.outcomes.is_empty() {
            return Err(IoError::Malformed(format!("variable `{}` has no outcomes", v.name)));
        }
        net.add_node(
            &v.name,
            NodeKind::Chance,
            v.outcomes.iter().map(State::new).collect(),
        )?;
        if let Some((x, y)) = v.position {
            visual
                .nodes
                .insert(v.name.clone(), NodeVisual { x, y, display: Default::default(), color: None });
        }
    }
    for d in &defs {
        let id = net
            .find_by_name(&d.for_)
            .ok_or_else(|| IoError::Malformed(format!("DEFINITION for unknown `{}`", d.for_)))?;
        for g in &d.given {
            let p = net
                .find_by_name(g)
                .ok_or_else(|| IoError::Malformed(format!("unknown GIVEN `{g}`")))?;
            net.add_edge(p, id)?;
        }
        net.set_table(id, Table { data: d.table.clone() })?;
    }
    Ok(Document { network: net, visual })
}

fn parse_position(prop: &str) -> Option<(f32, f32)> {
    let p = prop.trim();
    if !p.to_lowercase().starts_with("position") {
        return None;
    }
    let open = p.find('(')?;
    let close = p.find(')')?;
    let mut it = p[open + 1..close].split(',').map(|s| s.trim().parse::<f32>());
    match (it.next(), it.next()) {
        (Some(Ok(x)), Some(Ok(y))) => Some((x, y)),
        _ => None,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn to_xml(doc: &Document) -> Result<(String, Vec<Warning>), IoError> {
    let net = &doc.network;
    let mut warnings = Vec::new();
    if !doc.visual.notes.is_empty() {
        warnings.push(Warning::Lossy(format!(
            "{} note(s) omitted (XMLBIF cannot store notes)",
            doc.visual.notes.len()
        )));
    }
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<BIF VERSION=\"0.3\">\n<NETWORK>\n");
    out.push_str(&format!("<NAME>{}</NAME>\n", esc(&net.name)));
    let order = net.topo_order();
    for &id in &order {
        let n = net.node(id);
        if n.kind != NodeKind::Chance {
            warnings.push(Warning::Lossy(format!(
                "node `{}` ({:?}) written as a chance node (XMLBIF has no decision/utility nodes)",
                n.name, n.kind
            )));
        }
        if n.kind == NodeKind::Utility {
            continue; // no states/table representation possible
        }
        out.push_str("<VARIABLE TYPE=\"nature\">\n");
        out.push_str(&format!("  <NAME>{}</NAME>\n", esc(&n.name)));
        for s in &n.states {
            out.push_str(&format!("  <OUTCOME>{}</OUTCOME>\n", esc(&s.name)));
        }
        if let Some(v) = doc.visual.nodes.get(&n.name) {
            out.push_str(&format!("  <PROPERTY>position = ({}, {})</PROPERTY>\n", v.x, v.y));
        }
        out.push_str("</VARIABLE>\n");
    }
    for &id in &order {
        let n = net.node(id);
        if n.kind == NodeKind::Utility {
            continue;
        }
        out.push_str("<DEFINITION>\n");
        out.push_str(&format!("  <FOR>{}</FOR>\n", esc(&n.name)));
        for &p in &n.parents {
            out.push_str(&format!("  <GIVEN>{}</GIVEN>\n", esc(&net.node(p).name)));
        }
        let table: Vec<String> = if n.kind == NodeKind::Decision {
            // Uniform placeholder so other tools can read the file.
            let rows = net.row_count(id);
            let k = n.n_states();
            vec![format!("{}", 1.0 / k as f64); rows * k]
        } else {
            n.table.data.iter().map(|v| format!("{v}")).collect()
        };
        out.push_str(&format!("  <TABLE>{}</TABLE>\n", table.join(" ")));
        out.push_str("</DEFINITION>\n");
    }
    out.push_str("</NETWORK>\n</BIF>\n");
    Ok((out, warnings))
}
