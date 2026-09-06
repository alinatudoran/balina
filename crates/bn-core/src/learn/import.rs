//! Build nodes directly from a case file: column headers become node names,
//! observed values become states. A column whose non-missing values are all
//! numeric and varied (any fractional value, or more distinct values than
//! [`DISTINCT_CONTINUOUS`]) is treated as continuous: its weighted mean and
//! standard deviation are learned from the data and the node is discretized
//! into equal-frequency bins, each state carrying the bin's mean as its
//! numeric level (`State.value`).
//!
//! Two streaming passes over one byte buffer: pass 1 keeps only per-column
//! accumulators (capped distinct sets; numeric values, which exact quantiles
//! require), pass 2 builds the [`CaseSet`] row by row.

use std::collections::{HashMap, HashSet};

use crate::error::CaseError;
use crate::model::{ContinuousInfo, Network, NodeId, NodeKind, State};

use super::cases::{is_missing, load_bytes, state_for_value, CaseSet};

/// Numeric columns with more distinct values than this are continuous.
pub const DISTINCT_CONTINUOUS: usize = 8;

#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// Equal-frequency bins for continuous columns (ties may yield fewer).
    pub bins: usize,
    /// A discrete column with more distinct values than this is an error.
    pub max_discrete_states: usize,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions { bins: 4, max_discrete_states: 50 }
    }
}

#[derive(Clone, Debug)]
pub struct ColumnReport {
    pub name: String,
    /// False when the column matched an existing node or was all-missing.
    pub created: bool,
    pub n_states: usize,
    pub continuous: Option<ContinuousInfo>,
}

/// Per-column pass-1 accumulator. Distinct sets are capped so memory stays
/// bounded by `max_discrete_states`, not by the number of rows.
struct Acc {
    seen: usize,
    numeric: bool,
    any_fraction: bool,
    values: Vec<(f64, f64)>,
    distinct_bits: HashSet<u64>,
    distinct: Vec<String>,
    distinct_lower: HashSet<String>,
    overflow: bool,
}

impl Acc {
    fn new() -> Acc {
        Acc {
            seen: 0,
            numeric: true,
            any_fraction: false,
            values: Vec::new(),
            distinct_bits: HashSet::new(),
            distinct: Vec::new(),
            distinct_lower: HashSet::new(),
            overflow: false,
        }
    }
}

/// How a created column maps raw cells to state indices in pass 2.
enum ValueMap {
    /// Existing node: name / index / nearest-level, like `read_cases`.
    Existing,
    /// Lower-cased raw value → state index.
    DiscreteStr(HashMap<String, usize>),
    /// Parsed value (as bits) → state index.
    DiscreteNum(HashMap<u64, usize>),
    /// Internal bin edges; state = #edges ≤ value.
    Binned(Vec<f64>),
}

pub fn import_cases_with<R: std::io::Read>(
    net: &mut Network,
    reader: R,
    delimiter: Option<u8>,
    opts: &ImportOptions,
) -> Result<(CaseSet, Vec<ColumnReport>), CaseError> {
    let (data, delim) = load_bytes(reader, delimiter)?;
    let make_reader = || {
        csv::ReaderBuilder::new()
            .delimiter(delim)
            .trim(csv::Trim::All)
            .from_reader(data.as_slice())
    };

    // ---- pass 1: accumulate per-column summaries -----------------------
    let mut rdr = make_reader();
    let headers = rdr.headers()?.clone();
    let mut weight_col: Option<usize> = None;
    let mut skip = vec![false; headers.len()];
    for (i, h) in headers.iter().enumerate() {
        if h.eq_ignore_ascii_case("IDnum") {
            skip[i] = true;
        } else if h.eq_ignore_ascii_case("NumCases") || h.eq_ignore_ascii_case("__count") {
            weight_col = Some(i);
            skip[i] = true;
        }
    }
    let mut accs: Vec<Acc> = (0..headers.len()).map(|_| Acc::new()).collect();
    for rec in rdr.records() {
        let rec = rec?;
        let w = weight_col
            .and_then(|i| rec.get(i))
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(1.0);
        for (i, acc) in accs.iter_mut().enumerate() {
            if skip[i] {
                continue;
            }
            let raw = rec.get(i).unwrap_or("");
            if is_missing(raw) {
                continue;
            }
            acc.seen += 1;
            if !acc.overflow && !acc.distinct_lower.contains(&raw.to_ascii_lowercase()) {
                if acc.distinct.len() < opts.max_discrete_states {
                    acc.distinct.push(raw.to_string());
                    acc.distinct_lower.insert(raw.to_ascii_lowercase());
                } else {
                    acc.overflow = true;
                }
            }
            if acc.numeric {
                match raw.parse::<f64>() {
                    Ok(v) if v.is_finite() => {
                        if v.fract() != 0.0 {
                            acc.any_fraction = true;
                        }
                        if acc.distinct_bits.len() <= DISTINCT_CONTINUOUS {
                            acc.distinct_bits.insert(v.to_bits());
                        }
                        acc.values.push((v, w));
                    }
                    _ => {
                        acc.numeric = false;
                        acc.values = Vec::new();
                        acc.distinct_bits.clear();
                    }
                }
            }
        }
    }

    // ---- classify columns and create missing nodes ----------------------
    let mut cols: Vec<(usize, NodeId, ValueMap)> = Vec::new();
    let mut reports = Vec::new();
    for (i, h) in headers.iter().enumerate() {
        if skip[i] {
            continue;
        }
        if let Some(id) = net.find_by_name(h) {
            reports.push(ColumnReport {
                name: h.to_string(),
                created: false,
                n_states: net.node(id).n_states(),
                continuous: None,
            });
            cols.push((i, id, ValueMap::Existing));
            continue;
        }
        let acc = std::mem::replace(&mut accs[i], Acc::new());
        if acc.seen == 0 {
            // All-missing column: nothing to build a node from.
            reports.push(ColumnReport {
                name: h.to_string(),
                created: false,
                n_states: 0,
                continuous: None,
            });
            continue;
        }
        let continuous =
            acc.numeric && (acc.any_fraction || acc.distinct_bits.len() > DISTINCT_CONTINUOUS);
        let (id, map, n_states, stats) = if continuous {
            let (states, info) = bin_column(acc.values, opts.bins);
            let n = states.len();
            let id = net.add_node(h, NodeKind::Chance, states)?;
            let edges = info.edges.clone();
            net.node_mut(id).continuous = Some(info.clone());
            (id, ValueMap::Binned(edges), n, Some(info))
        } else if acc.numeric {
            // Discrete numeric codes: one state per distinct value, ascending.
            let mut vals: Vec<f64> = acc.distinct_bits.iter().map(|&b| f64::from_bits(b)).collect();
            vals.sort_by(|a, b| a.total_cmp(b));
            let states: Vec<State> = vals
                .iter()
                .map(|&v| State { name: format!("{v}"), value: Some(v) })
                .collect();
            let map: HashMap<u64, usize> =
                vals.iter().enumerate().map(|(s, &v)| (v.to_bits(), s)).collect();
            let n = states.len();
            let id = net.add_node(h, NodeKind::Chance, states)?;
            (id, ValueMap::DiscreteNum(map), n, None)
        } else {
            if acc.overflow {
                return Err(CaseError::TooManyStates {
                    column: h.to_string(),
                    max: opts.max_discrete_states,
                });
            }
            let mut names = acc.distinct;
            names.sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
            let states: Vec<State> = names
                .iter()
                .map(|n| State { name: n.clone(), value: n.parse::<f64>().ok().filter(|v| v.is_finite()) })
                .collect();
            let map: HashMap<String, usize> = names
                .iter()
                .enumerate()
                .map(|(s, n)| (n.to_ascii_lowercase(), s))
                .collect();
            let n = states.len();
            let id = net.add_node(h, NodeKind::Chance, states)?;
            (id, ValueMap::DiscreteStr(map), n, None)
        };
        reports.push(ColumnReport { name: h.to_string(), created: true, n_states, continuous: stats });
        cols.push((i, id, map));
    }
    if cols.is_empty() {
        return Err(CaseError::NoMatchingColumns);
    }

    // ---- pass 2: stream again and build the case set --------------------
    let mut rdr = make_reader();
    let mut rows = Vec::new();
    let mut weights = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let mut row = Vec::with_capacity(cols.len());
        for (i, id, map) in &cols {
            let raw = rec.get(*i).unwrap_or("");
            if is_missing(raw) {
                row.push(None);
                continue;
            }
            let unknown = || CaseError::UnknownState {
                node: net.node(*id).name.clone(),
                value: raw.to_string(),
            };
            let s = match map {
                ValueMap::Existing => state_for_value(net.node(*id), raw)?,
                ValueMap::DiscreteStr(m) => {
                    *m.get(&raw.to_ascii_lowercase()).ok_or_else(unknown)?
                }
                ValueMap::DiscreteNum(m) => {
                    let v: f64 = raw.parse().map_err(|_| unknown())?;
                    *m.get(&v.to_bits()).ok_or_else(unknown)?
                }
                ValueMap::Binned(edges) => {
                    let v: f64 = raw.parse().map_err(|_| unknown())?;
                    edges.partition_point(|&e| e <= v)
                }
            };
            row.push(Some(s));
        }
        let w = weight_col
            .and_then(|i| rec.get(i))
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(1.0);
        rows.push(row);
        weights.push(w);
    }
    Ok((CaseSet { nodes: cols.into_iter().map(|(_, id, _)| id).collect(), rows, weights }, reports))
}

/// Equal-frequency binning of a weighted sample. Returns the bin states
/// (range names, per-bin weighted mean as the numeric level) and the column's
/// weighted statistics + bin edges packed into a [`ContinuousInfo`].
fn bin_column(mut values: Vec<(f64, f64)>, bins: usize) -> (Vec<State>, ContinuousInfo) {
    values.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = values.iter().map(|&(_, w)| w).sum();
    let mean = values.iter().map(|&(v, w)| v * w).sum::<f64>() / total;
    let var = values.iter().map(|&(v, w)| w * (v - mean) * (v - mean)).sum::<f64>() / total;
    let min = values.first().map(|&(v, _)| v).unwrap_or(0.0);
    let max = values.last().map(|&(v, _)| v).unwrap_or(0.0);

    // Quantile edges: the data value where the cumulative weight crosses
    // k/bins. Deduped and kept strictly above the minimum, so every bin
    // [edge_i, edge_i+1) starts at a data value and is non-empty.
    let mut edges: Vec<f64> = Vec::new();
    let mut cum = 0.0;
    let mut k = 1;
    for &(v, w) in &values {
        cum += w;
        while k < bins && cum >= total * k as f64 / bins as f64 {
            edges.push(v);
            k += 1;
        }
    }
    edges.dedup_by(|a, b| a == b);
    edges.retain(|&e| e > min);

    // Per-bin weighted mean for the state's numeric level.
    let nbins = edges.len() + 1;
    let mut wsum = vec![0.0; nbins];
    let mut vsum = vec![0.0; nbins];
    for &(v, w) in &values {
        let b = edges.partition_point(|&e| e <= v);
        wsum[b] += w;
        vsum[b] += v * w;
    }
    let states: Vec<State> = (0..nbins)
        .map(|b| {
            let name = if nbins == 1 {
                if min == max { format!("{min}") } else { format!("{min} to {max}") }
            } else if b == 0 {
                format!("< {}", edges[0])
            } else if b == nbins - 1 {
                format!(">= {}", edges[nbins - 2])
            } else {
                format!("{} to {}", edges[b - 1], edges[b])
            };
            State { name, value: Some(vsum[b] / wsum[b]) }
        })
        .collect();
    let info = ContinuousInfo { mean, std: var.sqrt(), min, max, n: total, edges };
    (states, info)
}
