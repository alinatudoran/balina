//! CSV case files: one row per case, one column per node. `*`, `?`, `NA`
//! or an empty cell mean "missing". An optional `NumCases` column carries
//! case weights/multiplicities; an `IDnum` column is ignored.
//!
//! Files may be delimited by comma, semicolon, or tab; [`read_cases`] infers
//! the delimiter from the header line ([`read_cases_with`] can force one,
//! e.g. tab for `.tsv`). A leading UTF-8 BOM is stripped. Output via
//! [`write_cases`] is always comma-delimited.

use crate::error::CaseError;
use crate::model::{Network, Node, NodeId};

#[derive(Clone, Debug)]
pub struct CaseSet {
    /// Column → node mapping.
    pub nodes: Vec<NodeId>,
    /// Per case: state index per column, `None` = missing.
    pub rows: Vec<Vec<Option<usize>>>,
    pub weights: Vec<f64>,
}

impl CaseSet {
    pub fn n_cases(&self) -> usize {
        self.rows.len()
    }
    pub fn total_weight(&self) -> f64 {
        self.weights.iter().sum()
    }
}

pub(crate) fn is_missing(s: &str) -> bool {
    matches!(s.trim(), "" | "*" | "?" | "NA" | "na" | "N/A")
}

/// Read all bytes, strip a UTF-8 BOM, and resolve the delimiter (sniffed
/// from the header line unless forced).
pub(crate) fn load_bytes<R: std::io::Read>(
    mut reader: R,
    delimiter: Option<u8>,
) -> Result<(Vec<u8>, u8), CaseError> {
    let mut data = Vec::new();
    reader.read_to_end(&mut data)?;
    if data.starts_with(b"\xEF\xBB\xBF") {
        data.drain(..3);
    }
    let delim = delimiter.unwrap_or_else(|| {
        let first_line = data.split(|&b| b == b'\n').next().unwrap_or(&[]);
        sniff_delimiter(String::from_utf8_lossy(first_line).trim_end_matches('\r'))
    });
    Ok((data, delim))
}

/// Map a raw cell to a state index: state name, numeric state index, or —
/// when every state carries a numeric level — the state with the nearest
/// level (so raw continuous values resolve against binned nodes).
pub(crate) fn state_for_value(node: &Node, raw: &str) -> Result<usize, CaseError> {
    if let Some(s) = node.state_index(raw) {
        return Ok(s);
    }
    if let Ok(s) = raw.parse::<usize>() {
        if s < node.n_states() {
            return Ok(s);
        }
    }
    if let Ok(x) = raw.parse::<f64>() {
        if !node.states.is_empty() && node.states.iter().all(|s| s.value.is_some()) {
            let best = node
                .states
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    (a.value.unwrap() - x).abs().total_cmp(&(b.value.unwrap() - x).abs())
                })
                .map(|(i, _)| i)
                .unwrap();
            return Ok(best);
        }
    }
    Err(CaseError::UnknownState { node: node.name.clone(), value: raw.to_string() })
}

/// Pick the delimiter (`\t`, `;`, or `,`) that occurs most often in the
/// header line, ignoring characters inside double-quoted fields. Ties and
/// delimiter-free lines fall back to comma.
pub fn sniff_delimiter(header_line: &str) -> u8 {
    let (mut tabs, mut semis, mut commas) = (0usize, 0usize, 0usize);
    let mut in_quotes = false;
    for c in header_line.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            '\t' if !in_quotes => tabs += 1,
            ';' if !in_quotes => semis += 1,
            ',' if !in_quotes => commas += 1,
            _ => {}
        }
    }
    if tabs > semis && tabs > commas {
        b'\t'
    } else if semis > tabs && semis > commas {
        b';'
    } else {
        b','
    }
}

pub fn read_cases<R: std::io::Read>(net: &Network, reader: R) -> Result<CaseSet, CaseError> {
    read_cases_with(net, reader, None)
}

/// Like [`read_cases`], with an explicit delimiter (`None` = infer from the
/// header line via [`sniff_delimiter`]).
pub fn read_cases_with<R: std::io::Read>(
    net: &Network,
    reader: R,
    delimiter: Option<u8>,
) -> Result<CaseSet, CaseError> {
    let (data, delim) = load_bytes(reader, delimiter)?;
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delim)
        .trim(csv::Trim::All)
        .from_reader(data.as_slice());
    let headers = rdr.headers()?.clone();
    let mut cols: Vec<(usize, NodeId)> = Vec::new();
    let mut weight_col: Option<usize> = None;
    for (i, h) in headers.iter().enumerate() {
        if h.eq_ignore_ascii_case("IDnum") {
            continue;
        }
        if h.eq_ignore_ascii_case("NumCases") || h.eq_ignore_ascii_case("__count") {
            weight_col = Some(i);
            continue;
        }
        if let Some(id) = net.find_by_name(h) {
            cols.push((i, id));
        }
        // Columns matching no node are skipped (the GUI wizard reports them).
    }
    if cols.is_empty() {
        return Err(CaseError::NoMatchingColumns);
    }
    let mut rows = Vec::new();
    let mut weights = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let mut row = Vec::with_capacity(cols.len());
        for &(i, id) in &cols {
            let raw = rec.get(i).unwrap_or("");
            if is_missing(raw) {
                row.push(None);
            } else {
                row.push(Some(state_for_value(net.node(id), raw)?));
            }
        }
        let w = weight_col
            .and_then(|i| rec.get(i))
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(1.0);
        rows.push(row);
        weights.push(w);
    }
    Ok(CaseSet { nodes: cols.into_iter().map(|(_, id)| id).collect(), rows, weights })
}

pub fn write_cases<W: std::io::Write>(
    net: &Network,
    cases: &CaseSet,
    writer: W,
) -> Result<(), CaseError> {
    let mut wtr = csv::Writer::from_writer(writer);
    let mut header: Vec<String> =
        cases.nodes.iter().map(|&id| net.node(id).name.clone()).collect();
    let uniform = cases.weights.iter().all(|&w| w == 1.0);
    if !uniform {
        header.push("NumCases".into());
    }
    wtr.write_record(&header)?;
    for (row, &w) in cases.rows.iter().zip(&cases.weights) {
        let mut rec: Vec<String> = row
            .iter()
            .zip(&cases.nodes)
            .map(|(v, &id)| match v {
                Some(s) => net.node(id).states[*s].name.clone(),
                None => "*".into(),
            })
            .collect();
        if !uniform {
            rec.push(format!("{w}"));
        }
        wtr.write_record(&rec)?;
    }
    wtr.flush().map_err(CaseError::Io)?;
    Ok(())
}
