//! The editable network model: nodes, states, tables, DAG structure.
//!
//! All structural edits go through `Network` methods, which keep every table
//! shaped consistently with the graph (adding a parent expands the child's
//! table, removing one contracts it, state edits remap both the node's own
//! table and every child's). CPT layout convention: axes are
//! `[parent_0, ..., parent_k, self]`, row-major, last axis fastest — so each
//! contiguous run of `out_card` entries is one conditional distribution.

use serde::{Deserialize, Serialize};
use slotmap::{SecondaryMap, SlotMap};
use std::collections::HashMap;

use crate::error::ModelError;

slotmap::new_key_type! {
    pub struct NodeId;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum NodeKind {
    Chance,
    Decision,
    Utility,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub name: String,
    /// Numeric level for expected-value / variance computations (optional).
    pub value: Option<f64>,
}

impl State {
    pub fn new(name: impl Into<String>) -> State {
        State { name: name.into(), value: None }
    }
}

/// Metadata retained when a node is created by discretizing a continuous
/// column during case import. Persisted in the native `.balina` format.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContinuousInfo {
    /// Weighted mean of the raw column.
    pub mean: f64,
    /// Weighted population standard deviation.
    pub std: f64,
    pub min: f64,
    pub max: f64,
    /// Total weight of the non-missing samples (≈ row count).
    pub n: f64,
    /// Bin edges used for discretization (strictly increasing; len = n_bins − 1).
    pub edges: Vec<f64>,
}

/// Flat table over `[parents..., self]` (utilities: over `[parents...]`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub data: Vec<f64>,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub title: String,
    pub kind: NodeKind,
    pub states: Vec<State>,
    /// Ordered: defines the table's parent axes.
    pub parents: Vec<NodeId>,
    pub table: Table,
    /// Experience (Dirichlet pseudo-count) per parent configuration.
    pub experience: Option<Vec<f64>>,
    pub comment: String,
    /// Set when the node was created by discretizing a continuous data column.
    pub continuous: Option<ContinuousInfo>,
}

impl Node {
    pub fn n_states(&self) -> usize {
        self.states.len()
    }
    /// Size of the table's own axis: 1 for utility nodes, #states otherwise.
    pub fn out_card(&self) -> usize {
        match self.kind {
            NodeKind::Utility => 1,
            _ => self.states.len(),
        }
    }
    pub fn has_table(&self) -> bool {
        !matches!(self.kind, NodeKind::Decision)
    }
    pub fn state_index(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|s| s.name.eq_ignore_ascii_case(name))
    }
    pub fn display_title(&self) -> &str {
        if self.title.is_empty() { &self.name } else { &self.title }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Network {
    pub name: String,
    pub comment: String,
    nodes: SlotMap<NodeId, Node>,
    children: SecondaryMap<NodeId, Vec<NodeId>>,
}

impl Network {
    pub fn new(name: impl Into<String>) -> Network {
        Network { name: name.into(), ..Default::default() }
    }

    // ---- access -------------------------------------------------------

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter()
    }
    pub fn node_ids(&self) -> Vec<NodeId> {
        self.nodes.keys().collect()
    }
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.children.get(id).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn find_by_name(&self, name: &str) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|(_, n)| n.name.eq_ignore_ascii_case(name))
            .map(|(id, _)| id)
    }
    pub fn edges(&self) -> Vec<(NodeId, NodeId)> {
        let mut out = vec![];
        for (id, n) in self.nodes.iter() {
            for &p in &n.parents {
                out.push((p, id));
            }
        }
        out
    }

    /// Cardinalities of a node's parents, in parent order.
    pub fn parent_cards(&self, id: NodeId) -> Vec<usize> {
        self.nodes[id].parents.iter().map(|&p| self.nodes[p].n_states()).collect()
    }

    /// Number of parent configurations (table rows).
    pub fn row_count(&self, id: NodeId) -> usize {
        self.parent_cards(id).iter().product()
    }

    /// Expected flat table length for a node.
    pub fn table_len(&self, id: NodeId) -> usize {
        self.row_count(id) * self.nodes[id].out_card()
    }

    /// Parent state indices for a table row (last parent varies fastest).
    pub fn row_assignment(&self, id: NodeId, row: usize) -> Vec<usize> {
        crate::factor::decode_index(row, &self.parent_cards(id))
    }

    /// One conditional distribution (table row) as a slice.
    pub fn table_row(&self, id: NodeId, row: usize) -> &[f64] {
        let n = self.nodes[id].out_card();
        &self.nodes[id].table.data[row * n..(row + 1) * n]
    }

    // ---- node edits ---------------------------------------------------

    pub fn add_node(
        &mut self,
        name: &str,
        kind: NodeKind,
        states: Vec<State>,
    ) -> Result<NodeId, ModelError> {
        if name.trim().is_empty() {
            return Err(ModelError::InvalidName(name.into()));
        }
        if self.find_by_name(name).is_some() {
            return Err(ModelError::DuplicateName(name.into()));
        }
        if states.is_empty() && kind != NodeKind::Utility {
            return Err(ModelError::NoStates);
        }
        let out = match kind {
            NodeKind::Utility => 1,
            _ => states.len(),
        };
        let table = match kind {
            NodeKind::Chance => Table { data: vec![1.0 / out as f64; out] },
            NodeKind::Utility => Table { data: vec![0.0] },
            NodeKind::Decision => Table::default(),
        };
        let id = self.nodes.insert(Node {
            name: name.into(),
            title: String::new(),
            kind,
            states,
            parents: vec![],
            table,
            experience: None,
            comment: String::new(),
            continuous: None,
        });
        self.children.insert(id, vec![]);
        Ok(id)
    }

    /// Restore a fully-specified node (undo support). Fails on name clash.
    pub fn insert_node_raw(&mut self, node: Node) -> Result<NodeId, ModelError> {
        if self.find_by_name(&node.name).is_some() {
            return Err(ModelError::DuplicateName(node.name));
        }
        debug_assert!(node.parents.is_empty(), "insert_node_raw: attach edges separately");
        let id = self.nodes.insert(node);
        self.children.insert(id, vec![]);
        Ok(id)
    }

    pub fn remove_node(&mut self, id: NodeId) {
        for c in self.children(id).to_vec() {
            self.remove_edge(id, c).ok();
        }
        for p in self.nodes[id].parents.clone() {
            if let Some(ch) = self.children.get_mut(p) {
                ch.retain(|&x| x != id);
            }
        }
        self.children.remove(id);
        self.nodes.remove(id);
    }

    pub fn rename_node(&mut self, id: NodeId, name: &str) -> Result<(), ModelError> {
        if name.trim().is_empty() {
            return Err(ModelError::InvalidName(name.into()));
        }
        if let Some(other) = self.find_by_name(name) {
            if other != id {
                return Err(ModelError::DuplicateName(name.into()));
            }
        }
        self.nodes[id].name = name.into();
        Ok(())
    }

    pub fn set_kind(&mut self, id: NodeId, kind: NodeKind) -> Result<(), ModelError> {
        if self.nodes[id].kind == kind {
            return Ok(());
        }
        if kind == NodeKind::Utility && !self.children(id).is_empty() {
            return Err(ModelError::UtilityWithChildren);
        }
        if kind != NodeKind::Utility && self.nodes[id].states.is_empty() {
            self.nodes[id].states =
                vec![State::new("state0"), State::new("state1")];
        }
        self.nodes[id].kind = kind;
        let rows = self.row_count(id);
        let out = self.nodes[id].out_card();
        self.nodes[id].table = match kind {
            NodeKind::Chance => Table { data: vec![1.0 / out as f64; rows * out] },
            NodeKind::Utility => Table { data: vec![0.0; rows] },
            NodeKind::Decision => Table::default(),
        };
        self.nodes[id].experience = None;
        Ok(())
    }

    // ---- edges ---------------------------------------------------------

    /// Would adding parent → child create a cycle (or a duplicate)?
    pub fn would_create_cycle(&self, parent: NodeId, child: NodeId) -> bool {
        parent == child || self.reaches(child, parent)
    }

    fn reaches(&self, from: NodeId, to: NodeId) -> bool {
        let mut stack = vec![from];
        let mut seen: SecondaryMap<NodeId, ()> = SecondaryMap::new();
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if seen.insert(n, ()).is_none() {
                for &c in self.children(n) {
                    stack.push(c);
                }
            }
        }
        false
    }

    pub fn add_edge(&mut self, parent: NodeId, child: NodeId) -> Result<(), ModelError> {
        if self.nodes[child].parents.contains(&parent) {
            return Err(ModelError::DuplicateEdge);
        }
        if self.nodes[parent].kind == NodeKind::Utility {
            return Err(ModelError::UtilityWithChildren);
        }
        if self.would_create_cycle(parent, child) {
            return Err(ModelError::WouldCreateCycle);
        }
        // New parent goes last: each old row is repeated k times.
        if self.nodes[child].has_table() {
            let k = self.nodes[parent].n_states();
            let out = self.nodes[child].out_card();
            let old = std::mem::take(&mut self.nodes[child].table.data);
            let mut new = Vec::with_capacity(old.len() * k);
            for row in old.chunks(out) {
                for _ in 0..k {
                    new.extend_from_slice(row);
                }
            }
            self.nodes[child].table.data = new;
        }
        self.nodes[child].parents.push(parent);
        self.nodes[child].experience = None;
        self.children.get_mut(parent).unwrap().push(child);
        Ok(())
    }

    pub fn remove_edge(&mut self, parent: NodeId, child: NodeId) -> Result<(), ModelError> {
        let pos = self.nodes[child]
            .parents
            .iter()
            .position(|&p| p == parent)
            .ok_or(ModelError::NoSuchEdge)?;
        if self.nodes[child].has_table() {
            // Average the table over the removed parent's axis.
            let mut dims = self.parent_cards(child);
            let out = self.nodes[child].out_card();
            dims.push(out);
            let card = dims[pos];
            let old = std::mem::take(&mut self.nodes[child].table.data);
            let new = sum_out_axis(&old, &dims, pos);
            self.nodes[child].table.data =
                new.into_iter().map(|v| v / card as f64).collect();
        }
        self.nodes[child].parents.remove(pos);
        self.nodes[child].experience = None;
        self.children.get_mut(parent).unwrap().retain(|&c| c != child);
        Ok(())
    }

    // ---- tables ---------------------------------------------------------

    pub fn set_table(&mut self, id: NodeId, table: Table) -> Result<(), ModelError> {
        let expected = self.table_len(id);
        if table.data.len() != expected {
            return Err(ModelError::TableShape { expected, got: table.data.len() });
        }
        self.nodes[id].table = table;
        Ok(())
    }

    pub fn set_experience(&mut self, id: NodeId, exp: Option<Vec<f64>>) -> Result<(), ModelError> {
        if let Some(e) = &exp {
            let expected = self.row_count(id);
            if e.len() != expected {
                return Err(ModelError::TableShape { expected, got: e.len() });
            }
        }
        self.nodes[id].experience = exp;
        Ok(())
    }

    pub fn set_comment(&mut self, id: NodeId, comment: String) {
        self.nodes[id].comment = comment;
    }
    pub fn set_title(&mut self, id: NodeId, title: String) {
        self.nodes[id].title = title;
    }

    /// Normalize every conditional distribution of a chance node to sum 1
    /// (all-zero rows become uniform).
    pub fn normalize_table(&mut self, id: NodeId) {
        if self.nodes[id].kind != NodeKind::Chance {
            return;
        }
        let out = self.nodes[id].out_card();
        for row in self.nodes[id].table.data.chunks_mut(out) {
            let s: f64 = row.iter().sum();
            if s > 0.0 {
                for v in row.iter_mut() {
                    *v /= s;
                }
            } else {
                for v in row.iter_mut() {
                    *v = 1.0 / out as f64;
                }
            }
        }
    }

    // ---- state remapping -------------------------------------------------

    /// Replace a node's state list. `map[i] = Some(old_index)` says new state
    /// `i` was old state `map[i]`; `None` marks a brand-new state. Reshapes
    /// this node's own table and the tables of every child.
    pub fn remap_states(
        &mut self,
        id: NodeId,
        new_states: Vec<State>,
        map: &[Option<usize>],
    ) -> Result<(), ModelError> {
        if map.len() != new_states.len() || new_states.is_empty() {
            return Err(ModelError::InvalidRemap);
        }
        let old_card = self.nodes[id].n_states();
        if map.iter().flatten().any(|&o| o >= old_card) {
            return Err(ModelError::InvalidRemap);
        }
        // Own table: remap the last axis (chance nodes only; utility tables
        // have no own axis, decision nodes have no table).
        if self.nodes[id].kind == NodeKind::Chance {
            let mut dims = self.parent_cards(id);
            dims.push(old_card);
            let old = std::mem::take(&mut self.nodes[id].table.data);
            let axis = dims.len() - 1;
            self.nodes[id].table.data = remap_axis(&old, &dims, axis, map, 0.0);
        }
        self.nodes[id].states = new_states;
        if self.nodes[id].kind == NodeKind::Chance {
            self.normalize_table(id);
        }
        // Children: remap this node's parent axis. New parent states get a
        // uniform row for chance children, zero for utility children.
        for c in self.children(id).to_vec() {
            if !self.nodes[c].has_table() {
                continue;
            }
            let pos = self.nodes[c].parents.iter().position(|&p| p == id).unwrap();
            let out = self.nodes[c].out_card();
            let mut dims: Vec<usize> = self.nodes[c]
                .parents
                .iter()
                .map(|&p| if p == id { old_card } else { self.nodes[p].n_states() })
                .collect();
            dims.push(out);
            let fill = if self.nodes[c].kind == NodeKind::Chance {
                1.0 / out as f64
            } else {
                0.0
            };
            let old = std::mem::take(&mut self.nodes[c].table.data);
            self.nodes[c].table.data = remap_axis(&old, &dims, pos, map, fill);
            self.nodes[c].experience = None;
        }
        self.nodes[id].experience = None;
        Ok(())
    }

    // ---- graph queries ----------------------------------------------------

    /// Topological order (parents before children).
    pub fn topo_order(&self) -> Vec<NodeId> {
        let mut indeg: HashMap<NodeId, usize> =
            self.nodes.keys().map(|id| (id, self.nodes[id].parents.len())).collect();
        let mut queue: Vec<NodeId> =
            indeg.iter().filter(|&(_, &d)| d == 0).map(|(&id, _)| id).collect();
        // Stable-ish order: sort roots by name for reproducibility.
        queue.sort_by(|a, b| self.nodes[*a].name.cmp(&self.nodes[*b].name));
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut qi = 0;
        while qi < queue.len() {
            let n = queue[qi];
            qi += 1;
            out.push(n);
            for &c in self.children(n) {
                let d = indeg.get_mut(&c).unwrap();
                *d -= 1;
                if *d == 0 {
                    queue.push(c);
                }
            }
        }
        debug_assert_eq!(out.len(), self.nodes.len(), "graph must be acyclic");
        out
    }
}

/// Sum a flat row-major (last axis fastest) array over one axis.
pub(crate) fn sum_out_axis(data: &[f64], dims: &[usize], axis: usize) -> Vec<f64> {
    let out_len: usize =
        dims.iter().enumerate().filter(|(i, _)| *i != axis).map(|(_, &c)| c).product();
    let mut out = vec![0.0; out_len];
    let mut idx = vec![0usize; dims.len()];
    // Strides of the output for each source axis (0 for the summed axis).
    let mut strides = vec![0usize; dims.len()];
    let mut acc = 1usize;
    for i in (0..dims.len()).rev() {
        if i != axis {
            strides[i] = acc;
            acc *= dims[i];
        }
    }
    let mut oi = 0usize;
    for &v in data {
        out[oi] += v;
        for ax in (0..dims.len()).rev() {
            idx[ax] += 1;
            oi += strides[ax];
            if idx[ax] < dims[ax] {
                break;
            }
            idx[ax] = 0;
            oi -= strides[ax] * dims[ax];
        }
    }
    out
}

/// Rebuild a flat row-major array with one axis remapped: along `axis`, new
/// index `i` copies old index `map[i]`, or is filled with `fill` if `None`.
pub(crate) fn remap_axis(
    data: &[f64],
    dims: &[usize],
    axis: usize,
    map: &[Option<usize>],
    fill: f64,
) -> Vec<f64> {
    let mut new_dims = dims.to_vec();
    new_dims[axis] = map.len();
    let n: usize = new_dims.iter().product();
    let old_strides = {
        let mut s = vec![1usize; dims.len()];
        for i in (0..dims.len().saturating_sub(1)).rev() {
            s[i] = s[i + 1] * dims[i + 1];
        }
        s
    };
    let mut out = vec![0.0; n];
    for (oi, slot) in out.iter_mut().enumerate() {
        let a = crate::factor::decode_index(oi, &new_dims);
        *slot = match map[a[axis]] {
            Some(old_state) => {
                let mut src = 0usize;
                for (ax, &st) in a.iter().enumerate() {
                    let s = if ax == axis { old_state } else { st };
                    src += s * old_strides[ax];
                }
                data[src]
            }
            None => fill,
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_state(name: &str) -> (String, Vec<State>) {
        (name.to_string(), vec![State::new("true"), State::new("false")])
    }

    fn add2(net: &mut Network, name: &str) -> NodeId {
        let (n, s) = two_state(name);
        net.add_node(&n, NodeKind::Chance, s).unwrap()
    }

    #[test]
    fn cycle_rejected() {
        let mut net = Network::new("t");
        let a = add2(&mut net, "A");
        let b = add2(&mut net, "B");
        net.add_edge(a, b).unwrap();
        assert!(matches!(net.add_edge(b, a), Err(ModelError::WouldCreateCycle)));
        assert!(net.would_create_cycle(b, a));
        assert!(!net.would_create_cycle(a, b)); // duplicate, but not a cycle
    }

    #[test]
    fn add_edge_expands_table() {
        let mut net = Network::new("t");
        let a = add2(&mut net, "A");
        let b = add2(&mut net, "B");
        net.set_table(b, Table { data: vec![0.3, 0.7] }).unwrap();
        net.add_edge(a, b).unwrap();
        // Rows for both parent states copy the old distribution.
        assert_eq!(net.node(b).table.data, vec![0.3, 0.7, 0.3, 0.7]);
        assert_eq!(net.table_len(b), 4);
    }

    #[test]
    fn remove_edge_averages() {
        let mut net = Network::new("t");
        let a = add2(&mut net, "A");
        let b = add2(&mut net, "B");
        net.add_edge(a, b).unwrap();
        net.set_table(b, Table { data: vec![0.9, 0.1, 0.5, 0.5] }).unwrap();
        net.remove_edge(a, b).unwrap();
        assert_eq!(net.node(b).table.data, vec![0.7, 0.3]);
    }

    #[test]
    fn remove_node_fixes_children() {
        let mut net = Network::new("t");
        let a = add2(&mut net, "A");
        let b = add2(&mut net, "B");
        let c = add2(&mut net, "C");
        net.add_edge(a, c).unwrap();
        net.add_edge(b, c).unwrap();
        net.remove_node(a);
        assert_eq!(net.node(c).parents, vec![b]);
        assert_eq!(net.table_len(c), 4);
        assert_eq!(net.node(c).table.data.len(), 4);
    }

    #[test]
    fn remap_states_grows_and_children_get_uniform() {
        let mut net = Network::new("t");
        let a = add2(&mut net, "A");
        let b = add2(&mut net, "B");
        net.add_edge(a, b).unwrap();
        net.set_table(b, Table { data: vec![0.9, 0.1, 0.2, 0.8] }).unwrap();
        // A gains a third state; keep old two.
        net.remap_states(
            a,
            vec![State::new("true"), State::new("false"), State::new("maybe")],
            &[Some(0), Some(1), None],
        )
        .unwrap();
        assert_eq!(net.node(a).n_states(), 3);
        assert_eq!(net.node(a).table.data.len(), 3);
        let s: f64 = net.node(a).table.data.iter().sum();
        assert!((s - 1.0).abs() < 1e-12);
        assert_eq!(net.node(b).table.data, vec![0.9, 0.1, 0.2, 0.8, 0.5, 0.5]);
    }

    #[test]
    fn topo_order_parents_first() {
        let mut net = Network::new("t");
        let a = add2(&mut net, "A");
        let b = add2(&mut net, "B");
        let c = add2(&mut net, "C");
        net.add_edge(c, b).unwrap();
        net.add_edge(b, a).unwrap();
        let order = net.topo_order();
        let pos = |id| order.iter().position(|&x| x == id).unwrap();
        assert!(pos(c) < pos(b) && pos(b) < pos(a));
    }
}
