//! Hugin-architecture message passing on a junction tree: one potential per
//! clique and per sepset; collect toward a root, normalize, distribute.

use crate::error::InferenceError;
use crate::factor::Factor;
use crate::inference::compile::JunctionTree;

/// Full propagation. On success every clique potential holds
/// `P(clique vars | evidence)` and the log-probability of the evidence is
/// returned. Evidence must already be multiplied into the potentials.
pub fn propagate(
    jt: &JunctionTree,
    pot: &mut [Factor],
    sep: &mut [Factor],
) -> Result<f64, InferenceError> {
    if pot.is_empty() {
        return Ok(0.0);
    }
    // Rooted BFS order from clique 0 (the tree is connected by construction:
    // components are bridged with empty sepsets).
    let n = pot.len();
    let mut parent: Vec<Option<(usize, usize)>> = vec![None; n]; // (parent clique, edge)
    let mut order = Vec::with_capacity(n);
    let mut visited = vec![false; n];
    let mut queue = std::collections::VecDeque::from([0usize]);
    visited[0] = true;
    while let Some(c) = queue.pop_front() {
        order.push(c);
        for &(nb, e) in &jt.neighbors[c] {
            if !visited[nb] {
                visited[nb] = true;
                parent[nb] = Some((c, e));
                queue.push_back(nb);
            }
        }
    }
    debug_assert_eq!(order.len(), n);

    // Collect: leaves toward root.
    for &c in order.iter().rev() {
        if let Some((p, e)) = parent[c] {
            absorb(jt, pot, sep, c, p, e);
        }
    }
    let p_e = pot[0].sum();
    if !(p_e > 1e-300) {
        return Err(InferenceError::ConflictingEvidence);
    }
    pot[0].normalize();
    // Distribute: root toward leaves.
    for &c in order.iter() {
        if let Some((p, e)) = parent[c] {
            absorb(jt, pot, sep, p, c, e);
        }
    }
    Ok(p_e.ln())
}

/// Absorb from clique `from` into clique `to` through edge `e`.
fn absorb(jt: &JunctionTree, pot: &mut [Factor], sep: &mut [Factor], from: usize, to: usize, e: usize) {
    let sep_vars = &jt.edges[e].2;
    let new_sep = pot[from].marginalize_to(sep_vars);
    let mut ratio = new_sep.clone();
    ratio.divide_assign(&sep[e]);
    pot[to].multiply_assign(&ratio);
    sep[e] = new_sep;
}
