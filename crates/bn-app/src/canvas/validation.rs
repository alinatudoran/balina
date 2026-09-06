//! Synchronous link validation, answered from precomputed ancestor sets —
//! no session op during a connect drag (port of the React `validation.ts`).

use std::collections::HashSet;

use bn_core::model::{Network, NodeId, NodeKind};
use slotmap::SecondaryMap;

pub type AncestorSets = SecondaryMap<NodeId, HashSet<NodeId>>;

#[derive(Clone, PartialEq, Debug)]
pub struct LinkCheck {
    pub ok: bool,
    pub reason: Option<String>,
}

impl LinkCheck {
    fn ok() -> LinkCheck {
        LinkCheck { ok: true, reason: None }
    }
    fn no(reason: impl Into<String>) -> LinkCheck {
        LinkCheck { ok: false, reason: Some(reason.into()) }
    }
    fn silent_no() -> LinkCheck {
        LinkCheck { ok: false, reason: None }
    }
}

/// Validate a prospective link `source → target`. While reconnecting an
/// existing edge, its original child stays valid (no-op drop).
pub fn check_link(
    net: &Network,
    ancestors: &AncestorSets,
    source: NodeId,
    target: NodeId,
    reconnect_original_child: Option<NodeId>,
) -> LinkCheck {
    if !net.contains(source) || !net.contains(target) {
        return LinkCheck::silent_no();
    }
    if source == target {
        return LinkCheck::no("a node cannot be its own parent");
    }
    let from = net.node(source);
    let to = net.node(target);
    if from.kind == NodeKind::Utility {
        return LinkCheck::no("utility nodes cannot have children");
    }
    if reconnect_original_child == Some(target) {
        return LinkCheck::ok(); // no-op drop
    }
    if to.parents.contains(&source) {
        return LinkCheck::no(format!("link {} → {} already exists", from.name, to.name));
    }
    if ancestors.get(source).is_some_and(|a| a.contains(&target)) {
        return LinkCheck::no(format!("link {} → {} would create a cycle", from.name, to.name));
    }
    LinkCheck::ok()
}

#[cfg(test)]
mod tests {
    use bn_core::model::State;
    use bn_session::views::ancestor_sets;

    use super::*;

    fn two_states() -> Vec<State> {
        vec![State::new("a"), State::new("b")]
    }

    /// A → B → C, plus a utility node U (mirrors validation.test.ts).
    fn chain() -> (Network, NodeId, NodeId, NodeId, NodeId) {
        let mut net = Network::new("t");
        let a = net.add_node("A", NodeKind::Chance, two_states()).unwrap();
        let b = net.add_node("B", NodeKind::Chance, two_states()).unwrap();
        let c = net.add_node("C", NodeKind::Chance, two_states()).unwrap();
        let u = net.add_node("U", NodeKind::Utility, vec![]).unwrap();
        net.add_edge(a, b).unwrap();
        net.add_edge(b, c).unwrap();
        (net, a, b, c, u)
    }

    #[test]
    fn accepts_a_fresh_valid_link() {
        let (net, a, _, c, _) = chain();
        let anc = ancestor_sets(&net);
        assert!(check_link(&net, &anc, a, c, None).ok);
    }

    #[test]
    fn rejects_self_loops() {
        let (net, a, ..) = chain();
        let anc = ancestor_sets(&net);
        assert!(!check_link(&net, &anc, a, a, None).ok);
    }

    #[test]
    fn rejects_duplicates() {
        let (net, a, b, ..) = chain();
        let anc = ancestor_sets(&net);
        let r = check_link(&net, &anc, a, b, None);
        assert!(!r.ok);
        assert!(r.reason.unwrap().contains("already exists"));
    }

    #[test]
    fn rejects_cycles_via_ancestors() {
        let (net, a, _, c, _) = chain();
        let anc = ancestor_sets(&net);
        let r = check_link(&net, &anc, c, a, None);
        assert!(!r.ok);
        assert!(r.reason.unwrap().contains("cycle"));
    }

    #[test]
    fn rejects_utility_as_parent() {
        let (net, a, _, _, u) = chain();
        let anc = ancestor_sets(&net);
        assert!(!check_link(&net, &anc, u, a, None).ok);
    }

    #[test]
    fn allows_reconnect_back_onto_original_child() {
        let (net, a, b, ..) = chain();
        let anc = ancestor_sets(&net);
        assert!(check_link(&net, &anc, a, b, Some(b)).ok);
    }

    #[test]
    fn rejects_stale_ids_silently() {
        let (mut net, a, b, ..) = chain();
        let anc = ancestor_sets(&net);
        net.remove_node(b);
        let r = check_link(&net, &anc, a, b, None);
        assert!(!r.ok && r.reason.is_none());
    }
}
