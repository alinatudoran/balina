//! Exact influence-diagram evaluation: variable elimination with
//! (probability, utility) potentials (Jensen & Nielsen). Decisions are
//! processed in topological order under the no-forgetting assumption;
//! multiple utility nodes are additive.

use std::collections::HashSet;

use crate::error::IdError;
use crate::factor::{Factor, VarId};
use crate::inference::{CompiledNet, Evidence};
use crate::model::{Network, NodeId, NodeKind};

#[derive(Clone, Debug)]
pub struct Policy {
    pub decision: NodeId,
    /// Nodes the optimal choice depends on (sorted by variable index).
    pub domain: Vec<NodeId>,
    pub domain_cards: Vec<usize>,
    /// Optimal state of the decision per domain configuration
    /// (row-major, last domain variable fastest).
    pub best: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct IdSolution {
    pub meu: f64,
    pub policies: Vec<Policy>,
}

/// Solve the influence diagram: maximum expected utility and one optimal
/// policy per decision node.
pub fn solve_influence_diagram(
    net: &Network,
    evidence: &Evidence,
) -> Result<IdSolution, IdError> {
    let cn = CompiledNet::compile(net);
    let decisions: Vec<NodeId> = cn
        .order
        .iter()
        .copied()
        .filter(|&id| net.node(id).kind == NodeKind::Decision)
        .collect();
    if decisions.is_empty() {
        return Err(IdError::NoDecisions);
    }
    let utilities: Vec<NodeId> = net
        .nodes()
        .filter(|(_, n)| n.kind == NodeKind::Utility)
        .map(|(id, _)| id)
        .collect();
    if utilities.is_empty() {
        return Err(IdError::NoUtilities);
    }

    // Probability potentials: chance CPTs + evidence likelihoods.
    let mut phi: Vec<Factor> = Vec::new();
    for (i, &id) in cn.order.iter().enumerate() {
        if net.node(id).kind == NodeKind::Chance {
            phi.push(cn.families[i].clone());
        }
    }
    for (node, f) in evidence.iter() {
        if let Some(&var) = cn.var_of.get(node) {
            let card = cn.cards[var.0 as usize];
            phi.push(Factor::new(vec![var], vec![card], f.to_likelihood(card)));
        }
    }
    // Utility potentials over each utility node's parents.
    let mut psi: Vec<Factor> = Vec::new();
    for &u in &utilities {
        let node = net.node(u);
        if node.parents.is_empty() {
            psi.push(Factor::scalar(node.table.data[0]));
            continue;
        }
        let axes: Vec<VarId> = node.parents.iter().map(|&p| cn.var_of[p]).collect();
        let cards: Vec<usize> =
            node.parents.iter().map(|&p| net.node(p).n_states()).collect();
        psi.push(Factor::from_axes(&axes, &cards, &node.table.data));
    }

    // Partition chance variables into information blocks I_0 .. I_n.
    let mut placed: HashSet<VarId> = HashSet::new();
    let mut blocks: Vec<Vec<VarId>> = Vec::new(); // blocks[k] observed before decision k
    for &d in &decisions {
        let block: Vec<VarId> = net
            .node(d)
            .parents
            .iter()
            .filter(|&&p| net.node(p).kind == NodeKind::Chance)
            .map(|&p| cn.var_of[p])
            .filter(|v| !placed.contains(v))
            .collect();
        placed.extend(block.iter().copied());
        placed.insert(cn.var_of[d]);
        blocks.push(block);
    }
    let tail: Vec<VarId> = cn
        .order
        .iter()
        .enumerate()
        .filter(|&(_, &id)| net.node(id).kind == NodeKind::Chance)
        .map(|(i, _)| VarId(i as u32))
        .filter(|v| !placed.contains(v))
        .collect();
    blocks.push(tail);

    // Eliminate from the back: sum I_n, max D_n, sum I_{n-1}, ..., sum I_0.
    let mut policies: Vec<Policy> = Vec::new();
    for k in (0..blocks.len()).rev() {
        for &v in &blocks[k] {
            eliminate_sum(v, &mut phi, &mut psi);
        }
        if k > 0 {
            let d = decisions[k - 1];
            policies.push(eliminate_max(cn.var_of[d], d, &cn, &mut phi, &mut psi));
        }
    }
    policies.reverse();

    let mut u_final = Factor::scalar(0.0);
    for f in psi {
        u_final = factor_sum(&u_final, &f);
    }
    // Any residual (should be scalar already).
    while !u_final.vars.is_empty() {
        u_final = u_final.sum_out(u_final.vars[0]);
    }
    Ok(IdSolution { meu: u_final.data[0], policies })
}

fn drain_containing(factors: &mut Vec<Factor>, v: VarId) -> Vec<Factor> {
    let mut with = Vec::new();
    let mut i = 0;
    while i < factors.len() {
        if factors[i].vars.contains(&v) {
            with.push(factors.swap_remove(i));
        } else {
            i += 1;
        }
    }
    with
}

/// Sum-eliminate a chance variable using the (p, u) combination rule:
/// p* = Σ_v Π p,  u* = (Σ_v (Π p)·(Σ u)) / p*  (0/0 = 0).
fn eliminate_sum(v: VarId, phi: &mut Vec<Factor>, psi: &mut Vec<Factor>) {
    let phi_v = drain_containing(phi, v);
    let psi_v = drain_containing(psi, v);
    if phi_v.is_empty() && psi_v.is_empty() {
        return;
    }
    let mut p_prod = Factor::scalar(1.0);
    for f in &phi_v {
        p_prod = p_prod.product(f);
    }
    let p_new = p_prod.sum_out(v);
    if !psi_v.is_empty() {
        let mut u_sum = Factor::scalar(0.0);
        for f in &psi_v {
            u_sum = factor_sum(&u_sum, f);
        }
        // u_new's domain always contains p_new's, so the division broadcasts.
        let mut u_new = p_prod.product(&u_sum).sum_out(v);
        u_new.divide_assign(&p_new);
        psi.push(u_new);
    }
    phi.push(p_new);
}

/// Max-eliminate a decision variable; the maximization criterion is the
/// summed utility potential (the probability part is constant over the
/// decision in a well-formed diagram).
fn eliminate_max(
    v: VarId,
    node: NodeId,
    cn: &CompiledNet,
    phi: &mut Vec<Factor>,
    psi: &mut Vec<Factor>,
) -> Policy {
    let phi_v = drain_containing(phi, v);
    let psi_v = drain_containing(psi, v);
    let card = cn.cards[v.0 as usize];
    if psi_v.is_empty() {
        // Utility independent of this decision: keep p (any slice), trivial policy.
        if !phi_v.is_empty() {
            let mut p_prod = Factor::scalar(1.0);
            for f in &phi_v {
                p_prod = p_prod.product(f);
            }
            phi.push(p_prod.max_out(v).0);
        }
        return Policy { decision: node, domain: vec![], domain_cards: vec![], best: vec![0] };
    }
    let mut u_sum = Factor::scalar(0.0);
    for f in &psi_v {
        u_sum = factor_sum(&u_sum, f);
    }
    // Broadcast so the decision axis is present even if only via phi.
    if u_sum.card_of(v).is_none() {
        u_sum = u_sum.product(&Factor::unit(vec![v], vec![card]));
    }
    let (u_max, argmax) = u_sum.max_out(v);
    let domain: Vec<NodeId> =
        u_max.vars.iter().map(|w| cn.order[w.0 as usize]).collect();
    let domain_cards = u_max.cards.clone();
    if !phi_v.is_empty() {
        let mut p_prod = Factor::scalar(1.0);
        for f in &phi_v {
            p_prod = p_prod.product(f);
        }
        phi.push(p_prod.max_out(v).0);
    }
    psi.push(u_max);
    Policy { decision: node, domain, domain_cards, best: argmax }
}

/// Elementwise sum of two factors over the union of their domains.
fn factor_sum(a: &Factor, b: &Factor) -> Factor {
    let ua = a.product(&Factor::unit(b.vars.clone(), b.cards.clone()));
    let mut ub = b.product(&Factor::unit(a.vars.clone(), a.cards.clone()));
    debug_assert_eq!(ua.vars, ub.vars);
    for (x, y) in ub.data.iter_mut().zip(&ua.data) {
        *x += y;
    }
    ub
}
