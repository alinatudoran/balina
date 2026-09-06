//! Inference engine facade: compile once, enter findings, query beliefs.

pub mod compile;
pub mod enumeration;
pub mod hugin;

pub use compile::{CompiledNet, JunctionTree};
pub use enumeration::posterior_enumeration;

use slotmap::SecondaryMap;

use crate::error::InferenceError;
use crate::factor::{Factor, VarId};
use crate::model::{Network, NodeId};

/// A finding on a node: hard evidence or a likelihood (soft) finding.
#[derive(Clone, Debug, PartialEq)]
pub enum Finding {
    Hard(usize),
    Likelihood(Vec<f64>),
}

impl Finding {
    pub fn to_likelihood(&self, card: usize) -> Vec<f64> {
        match self {
            Finding::Hard(s) => {
                let mut v = vec![0.0; card];
                v[*s] = 1.0;
                v
            }
            Finding::Likelihood(l) => l.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Evidence {
    findings: SecondaryMap<NodeId, Finding>,
}

impl Evidence {
    pub fn new() -> Evidence {
        Evidence::default()
    }
    pub fn set(&mut self, node: NodeId, finding: Finding) {
        self.findings.insert(node, finding);
    }
    pub fn retract(&mut self, node: NodeId) {
        self.findings.remove(node);
    }
    pub fn clear(&mut self) {
        self.findings.clear();
    }
    pub fn get(&self, node: NodeId) -> Option<&Finding> {
        self.findings.get(node)
    }
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Finding)> {
        self.findings.iter()
    }
    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }
    pub fn len(&self) -> usize {
        self.findings.len()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EngineState {
    Stale,
    Ready,
    Conflict,
}

/// Compiled junction-tree engine over a snapshot of a network. Structural or
/// CPT edits to the network invalidate the engine: recompile.
pub struct Engine {
    cn: CompiledNet,
    jt: JunctionTree,
    init_pot: Vec<Factor>,
    pot: Vec<Factor>,
    sep: Vec<Factor>,
    evidence: Evidence,
    state: EngineState,
    log_p_e: f64,
}

impl Engine {
    pub fn compile(net: &Network) -> Engine {
        let cn = CompiledNet::compile(net);
        let jt = JunctionTree::build(net, &cn);
        // Initial clique potentials: product of the families assigned there.
        let mut init_pot: Vec<Factor> = jt
            .cliques
            .iter()
            .map(|c| {
                let cards = c.iter().map(|v| cn.cards[v.0 as usize]).collect();
                Factor::unit(c.clone(), cards)
            })
            .collect();
        for (v, fam) in cn.families.iter().enumerate() {
            init_pot[jt.home_of_family[v]].multiply_assign(fam);
        }
        let sep = unit_seps(&jt, &cn.cards);
        Engine {
            pot: init_pot.clone(),
            init_pot,
            sep,
            cn,
            jt,
            evidence: Evidence::new(),
            state: EngineState::Stale,
            log_p_e: 0.0,
        }
    }

    pub fn compiled(&self) -> &CompiledNet {
        &self.cn
    }
    pub fn junction_tree(&self) -> &JunctionTree {
        &self.jt
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    /// Refresh CPT values without rebuilding the tree (same structure).
    pub fn refresh_potentials(&mut self, net: &Network) {
        let cn = CompiledNet::compile(net);
        debug_assert_eq!(cn.order, self.cn.order);
        self.cn = cn;
        for (p, c) in self.init_pot.iter_mut().zip(&self.jt.cliques) {
            let cards = c.iter().map(|v| self.cn.cards[v.0 as usize]).collect();
            *p = Factor::unit(c.clone(), cards);
        }
        for (v, fam) in self.cn.families.iter().enumerate() {
            self.init_pot[self.jt.home_of_family[v]].multiply_assign(fam);
        }
        self.state = EngineState::Stale;
    }

    pub fn set_finding(&mut self, node: NodeId, finding: Finding) -> Result<(), InferenceError> {
        let var = self.var_of(node)?;
        let card = self.cn.cards[var.0 as usize];
        match &finding {
            Finding::Hard(s) if *s >= card => return Err(InferenceError::BadState),
            Finding::Likelihood(l) if l.len() != card => {
                return Err(InferenceError::BadLikelihood)
            }
            _ => {}
        }
        self.evidence.set(node, finding);
        self.state = EngineState::Stale;
        Ok(())
    }

    pub fn retract_finding(&mut self, node: NodeId) {
        self.evidence.retract(node);
        self.state = EngineState::Stale;
    }

    pub fn retract_all(&mut self) {
        self.evidence.clear();
        self.state = EngineState::Stale;
    }

    pub fn set_evidence(&mut self, evidence: Evidence) {
        self.evidence = evidence;
        self.state = EngineState::Stale;
    }

    pub fn var_of(&self, node: NodeId) -> Result<VarId, InferenceError> {
        self.cn.var_of.get(node).copied().ok_or(InferenceError::NotAVariable)
    }

    pub fn node_of(&self, var: VarId) -> NodeId {
        self.cn.order[var.0 as usize]
    }

    fn ensure(&mut self) -> Result<(), InferenceError> {
        match self.state {
            EngineState::Ready => Ok(()),
            EngineState::Conflict => Err(InferenceError::ConflictingEvidence),
            EngineState::Stale => {
                self.pot = self.init_pot.clone();
                self.sep = unit_seps(&self.jt, &self.cn.cards);
                for (node, f) in self.evidence.iter() {
                    // Evidence on nodes no longer in the net is skipped.
                    if let Some(&var) = self.cn.var_of.get(node) {
                        let card = self.cn.cards[var.0 as usize];
                        let lik = f.to_likelihood(card);
                        let c = self.jt.belief_clique[var.0 as usize];
                        self.pot[c].multiply_likelihood(var, &lik);
                    }
                }
                match hugin::propagate(&self.jt, &mut self.pot, &mut self.sep) {
                    Ok(lp) => {
                        self.log_p_e = lp;
                        self.state = EngineState::Ready;
                        Ok(())
                    }
                    Err(e) => {
                        self.state = EngineState::Conflict;
                        Err(e)
                    }
                }
            }
        }
    }

    /// Posterior marginal P(node | evidence).
    pub fn beliefs(&mut self, node: NodeId) -> Result<Vec<f64>, InferenceError> {
        let var = self.var_of(node)?;
        self.ensure()?;
        let c = self.jt.belief_clique[var.0 as usize];
        let mut m = self.pot[c].marginalize_to(&[var]);
        m.normalize();
        Ok(m.data)
    }

    /// Posterior marginals for every chance and decision node.
    pub fn all_beliefs(&mut self) -> Result<SecondaryMap<NodeId, Vec<f64>>, InferenceError> {
        self.ensure()?;
        let mut out = SecondaryMap::new();
        for (i, &id) in self.cn.order.iter().enumerate() {
            let var = VarId(i as u32);
            let c = self.jt.belief_clique[i];
            let mut m = self.pot[c].marginalize_to(&[var]);
            m.normalize();
            out.insert(id, m.data);
        }
        Ok(out)
    }

    pub fn log_prob_of_findings(&mut self) -> Result<f64, InferenceError> {
        self.ensure()?;
        Ok(self.log_p_e)
    }

    /// Joint posterior over a node's family, P(node, parents | evidence),
    /// as a factor over sorted VarIds.
    pub fn family_posterior(&mut self, node: NodeId) -> Result<Factor, InferenceError> {
        let var = self.var_of(node)?;
        self.ensure()?;
        let c = self.jt.home_of_family[var.0 as usize];
        let mut m = self.pot[c].marginalize_to(&self.cn.family_vars[var.0 as usize]);
        m.normalize();
        Ok(m)
    }

    /// Joint posterior over an arbitrary set of nodes. Uses a single clique
    /// when one contains them all; otherwise falls back to variable
    /// elimination over the original factors plus evidence.
    pub fn joint_posterior(&mut self, nodes: &[NodeId]) -> Result<Factor, InferenceError> {
        let mut vars: Vec<VarId> = nodes
            .iter()
            .map(|&n| self.var_of(n))
            .collect::<Result<_, _>>()?;
        vars.sort();
        vars.dedup();
        self.ensure()?;
        if let Some(c) = self.jt.clique_containing(&vars, &self.cn.cards) {
            let mut m = self.pot[c].marginalize_to(&vars);
            m.normalize();
            return Ok(m);
        }
        // VE fallback over original factors + evidence likelihoods.
        let mut factors: Vec<Factor> = self.cn.families.clone();
        for (node, f) in self.evidence.iter() {
            if let Some(&var) = self.cn.var_of.get(node) {
                let card = self.cn.cards[var.0 as usize];
                factors.push(Factor::new(vec![var], vec![card], f.to_likelihood(card)));
            }
        }
        let mut elim: Vec<VarId> = (0..self.cn.n_vars() as u32)
            .map(VarId)
            .filter(|v| !vars.contains(v))
            .collect();
        while let Some(pos) = pick_min_size(&elim, &factors, &self.cn.cards) {
            let v = elim.swap_remove(pos);
            let (with, without): (Vec<Factor>, Vec<Factor>) =
                factors.into_iter().partition(|f| f.vars.contains(&v));
            factors = without;
            let mut prod = Factor::scalar(1.0);
            for f in with {
                prod = prod.product(&f);
            }
            factors.push(prod.sum_out(v));
        }
        let mut result = Factor::scalar(1.0);
        for f in factors {
            result = result.product(&f);
        }
        let mut m = result.marginalize_to(&vars);
        if m.normalize() <= 0.0 {
            return Err(InferenceError::ConflictingEvidence);
        }
        Ok(m)
    }
}

fn unit_seps(jt: &JunctionTree, cards: &[usize]) -> Vec<Factor> {
    jt.edges
        .iter()
        .map(|(_, _, sep)| {
            Factor::unit(sep.clone(), sep.iter().map(|v| cards[v.0 as usize]).collect())
        })
        .collect()
}

fn pick_min_size(elim: &[VarId], factors: &[Factor], cards: &[usize]) -> Option<usize> {
    if elim.is_empty() {
        return None;
    }
    let mut best = (0usize, f64::INFINITY);
    for (i, &v) in elim.iter().enumerate() {
        let mut vars: Vec<VarId> = vec![];
        for f in factors.iter().filter(|f| f.vars.contains(&v)) {
            for &w in &f.vars {
                if !vars.contains(&w) {
                    vars.push(w);
                }
            }
        }
        let size: f64 = vars.iter().map(|w| cards[w.0 as usize] as f64).product();
        if size < best.1 {
            best = (i, size);
        }
    }
    Some(best.0)
}
