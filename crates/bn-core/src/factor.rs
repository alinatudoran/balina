//! Dense factor algebra over discrete variables.
//!
//! Layout convention (used across the whole crate): row-major with the LAST
//! variable varying fastest. `vars` is always sorted ascending, which makes
//! the variable union in `product` a sorted merge.

/// Dense variable index used inside the inference engine (0..n, topological).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct VarId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Factor {
    pub vars: Vec<VarId>,
    pub cards: Vec<usize>,
    pub data: Vec<f64>,
}

fn strides(cards: &[usize]) -> Vec<usize> {
    let mut s = vec![1usize; cards.len()];
    for i in (0..cards.len().saturating_sub(1)).rev() {
        s[i] = s[i + 1] * cards[i + 1];
    }
    s
}

fn size(cards: &[usize]) -> usize {
    cards.iter().product()
}

impl Factor {
    pub fn new(vars: Vec<VarId>, cards: Vec<usize>, data: Vec<f64>) -> Factor {
        debug_assert!(vars.windows(2).all(|w| w[0] < w[1]), "vars must be sorted");
        debug_assert_eq!(size(&cards), data.len());
        Factor { vars, cards, data }
    }

    pub fn scalar(v: f64) -> Factor {
        Factor { vars: vec![], cards: vec![], data: vec![v] }
    }

    pub fn unit(vars: Vec<VarId>, cards: Vec<usize>) -> Factor {
        let n = size(&cards);
        Factor::new(vars, cards, vec![1.0; n])
    }

    /// Build a factor from data laid out over `axes` (distinct, possibly
    /// unsorted), permuting it into canonical sorted-variable order.
    pub fn from_axes(axes: &[VarId], cards: &[usize], data: &[f64]) -> Factor {
        debug_assert_eq!(size(cards), data.len());
        let mut order: Vec<usize> = (0..axes.len()).collect();
        order.sort_by_key(|&i| axes[i]);
        if order.iter().enumerate().all(|(i, &o)| i == o) {
            return Factor::new(axes.to_vec(), cards.to_vec(), data.to_vec());
        }
        let out_vars: Vec<VarId> = order.iter().map(|&i| axes[i]).collect();
        let out_cards: Vec<usize> = order.iter().map(|&i| cards[i]).collect();
        let out_strides = strides(&out_cards);
        // For each source axis, its stride in the output.
        let mut src_out_stride = vec![0usize; axes.len()];
        for (out_ax, &src_ax) in order.iter().enumerate() {
            src_out_stride[src_ax] = out_strides[out_ax];
        }
        let mut out = vec![0.0; data.len()];
        let mut idx = vec![0usize; axes.len()];
        let mut oi = 0usize;
        for &v in data {
            out[oi] = v;
            for ax in (0..axes.len()).rev() {
                idx[ax] += 1;
                oi += src_out_stride[ax];
                if idx[ax] < cards[ax] {
                    break;
                }
                idx[ax] = 0;
                oi -= src_out_stride[ax] * cards[ax];
            }
        }
        Factor::new(out_vars, out_cards, out)
    }

    pub fn card_of(&self, var: VarId) -> Option<usize> {
        self.vars.iter().position(|&v| v == var).map(|i| self.cards[i])
    }

    /// Stride of each of this factor's axes when iterating an assignment over
    /// `outer_vars`/`outer_cards` (0 for variables this factor doesn't have).
    fn strides_in(&self, outer_vars: &[VarId]) -> Vec<usize> {
        let own = strides(&self.cards);
        outer_vars
            .iter()
            .map(|v| self.vars.iter().position(|w| w == v).map_or(0, |i| own[i]))
            .collect()
    }

    pub fn product(&self, other: &Factor) -> Factor {
        // Sorted merge of variable sets.
        let mut vars = Vec::with_capacity(self.vars.len() + other.vars.len());
        let mut cards = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < self.vars.len() || j < other.vars.len() {
            if j >= other.vars.len() || (i < self.vars.len() && self.vars[i] < other.vars[j]) {
                vars.push(self.vars[i]);
                cards.push(self.cards[i]);
                i += 1;
            } else if i >= self.vars.len() || other.vars[j] < self.vars[i] {
                vars.push(other.vars[j]);
                cards.push(other.cards[j]);
                j += 1;
            } else {
                debug_assert_eq!(self.cards[i], other.cards[j]);
                vars.push(self.vars[i]);
                cards.push(self.cards[i]);
                i += 1;
                j += 1;
            }
        }
        let a_str = self.strides_in(&vars);
        let b_str = other.strides_in(&vars);
        let n = size(&cards);
        let mut data = vec![0.0; n];
        let mut idx = vec![0usize; vars.len()];
        let (mut ia, mut ib) = (0usize, 0usize);
        for out in data.iter_mut() {
            *out = self.data[ia] * other.data[ib];
            for ax in (0..vars.len()).rev() {
                idx[ax] += 1;
                ia += a_str[ax];
                ib += b_str[ax];
                if idx[ax] < cards[ax] {
                    break;
                }
                idx[ax] = 0;
                ia -= a_str[ax] * cards[ax];
                ib -= b_str[ax] * cards[ax];
            }
        }
        Factor::new(vars, cards, data)
    }

    /// In-place `self *= other`, requiring other.vars ⊆ self.vars.
    pub fn multiply_assign(&mut self, other: &Factor) {
        self.zip_assign(other, |a, b| a * b);
    }

    /// In-place `self /= other` with the Hugin convention 0/0 = 0.
    /// Requires other.vars ⊆ self.vars.
    pub fn divide_assign(&mut self, other: &Factor) {
        self.zip_assign(other, |a, b| if b == 0.0 { 0.0 } else { a / b });
    }

    fn zip_assign(&mut self, other: &Factor, f: impl Fn(f64, f64) -> f64) {
        debug_assert!(other.vars.iter().all(|v| self.vars.contains(v)));
        let b_str = other.strides_in(&self.vars);
        let mut idx = vec![0usize; self.vars.len()];
        let mut ib = 0usize;
        for a in self.data.iter_mut() {
            *a = f(*a, other.data[ib]);
            for ax in (0..self.vars.len()).rev() {
                idx[ax] += 1;
                ib += b_str[ax];
                if idx[ax] < self.cards[ax] {
                    break;
                }
                idx[ax] = 0;
                ib -= b_str[ax] * self.cards[ax];
            }
        }
    }

    /// Marginalize down to `keep` (a sorted subset of self.vars).
    pub fn marginalize_to(&self, keep: &[VarId]) -> Factor {
        debug_assert!(keep.windows(2).all(|w| w[0] < w[1]));
        let cards: Vec<usize> = keep.iter().map(|v| self.card_of(*v).unwrap()).collect();
        let target = Factor::new(keep.to_vec(), cards, vec![0.0; size_of(self, keep)]);
        let mut target = target;
        let t_str = target.strides_in(&self.vars);
        let mut idx = vec![0usize; self.vars.len()];
        let mut ti = 0usize;
        for &v in &self.data {
            target.data[ti] += v;
            for ax in (0..self.vars.len()).rev() {
                idx[ax] += 1;
                ti += t_str[ax];
                if idx[ax] < self.cards[ax] {
                    break;
                }
                idx[ax] = 0;
                ti -= t_str[ax] * self.cards[ax];
            }
        }
        target
    }

    pub fn sum_out(&self, var: VarId) -> Factor {
        let keep: Vec<VarId> = self.vars.iter().copied().filter(|&v| v != var).collect();
        self.marginalize_to(&keep)
    }

    /// Max out `var`. Returns the maximized factor and, per remaining
    /// configuration, the state index of `var` achieving the max.
    pub fn max_out(&self, var: VarId) -> (Factor, Vec<usize>) {
        let keep: Vec<VarId> = self.vars.iter().copied().filter(|&v| v != var).collect();
        let cards: Vec<usize> = keep.iter().map(|v| self.card_of(*v).unwrap()).collect();
        let n = size(&cards);
        let mut out = Factor::new(keep, cards, vec![f64::NEG_INFINITY; n]);
        let mut arg = vec![0usize; n];
        let t_str = out.strides_in(&self.vars);
        let var_ax = self.vars.iter().position(|&v| v == var).unwrap();
        let mut idx = vec![0usize; self.vars.len()];
        let mut ti = 0usize;
        for &v in &self.data {
            if v > out.data[ti] {
                out.data[ti] = v;
                arg[ti] = idx[var_ax];
            }
            for ax in (0..self.vars.len()).rev() {
                idx[ax] += 1;
                ti += t_str[ax];
                if idx[ax] < self.cards[ax] {
                    break;
                }
                idx[ax] = 0;
                ti -= t_str[ax] * self.cards[ax];
            }
        }
        (out, arg)
    }

    /// Multiply a likelihood vector over a single variable into this factor.
    pub fn multiply_likelihood(&mut self, var: VarId, lik: &[f64]) {
        let card = self.card_of(var).expect("var not in factor");
        debug_assert_eq!(card, lik.len());
        let f = Factor::new(vec![var], vec![card], lik.to_vec());
        self.multiply_assign(&f);
    }

    /// Normalize to sum 1. Returns the pre-normalization sum.
    pub fn normalize(&mut self) -> f64 {
        let s: f64 = self.data.iter().sum();
        if s > 0.0 {
            for v in self.data.iter_mut() {
                *v /= s;
            }
        }
        s
    }

    pub fn sum(&self) -> f64 {
        self.data.iter().sum()
    }

    /// Value at a full assignment (one state index per self.vars entry).
    pub fn at(&self, assignment: &[usize]) -> f64 {
        let s = strides(&self.cards);
        let mut i = 0;
        for (ax, &st) in assignment.iter().enumerate() {
            i += st * s[ax];
        }
        self.data[i]
    }
}

fn size_of(f: &Factor, keep: &[VarId]) -> usize {
    keep.iter().map(|v| f.card_of(*v).unwrap()).product()
}

/// Decode a linear index into a mixed-radix assignment (last axis fastest).
pub fn decode_index(mut i: usize, cards: &[usize]) -> Vec<usize> {
    let mut out = vec![0usize; cards.len()];
    for ax in (0..cards.len()).rev() {
        out[ax] = i % cards[ax];
        i /= cards[ax];
    }
    out
}

/// Encode a mixed-radix assignment (last axis fastest) into a linear index.
pub fn encode_index(assignment: &[usize], cards: &[usize]) -> usize {
    let mut i = 0usize;
    for ax in 0..cards.len() {
        i = i * cards[ax] + assignment[ax];
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(i: u32) -> VarId {
        VarId(i)
    }

    /// Naive reference: evaluate a factor at an assignment given by a map
    /// over a global assignment vector.
    fn eval(f: &Factor, global: &[usize]) -> f64 {
        let a: Vec<usize> = f.vars.iter().map(|v| global[v.0 as usize]).collect();
        f.at(&a)
    }

    fn all_assignments(cards: &[usize]) -> Vec<Vec<usize>> {
        let n: usize = cards.iter().product();
        (0..n).map(|i| decode_index(i, cards)).collect()
    }

    #[test]
    fn product_matches_naive() {
        // Global vars 0,1,2 with cards 2,3,2. f over {0,2}, g over {1,2}.
        let f = Factor::new(vec![v(0), v(2)], vec![2, 2], vec![0.1, 0.9, 0.4, 0.6]);
        let g = Factor::new(
            vec![v(1), v(2)],
            vec![3, 2],
            vec![0.2, 0.8, 0.5, 0.5, 0.3, 0.7],
        );
        let p = f.product(&g);
        assert_eq!(p.vars, vec![v(0), v(1), v(2)]);
        for ga in all_assignments(&[2, 3, 2]) {
            let want = eval(&f, &ga) * eval(&g, &ga);
            assert!((eval(&p, &ga) - want).abs() < 1e-12);
        }
    }

    #[test]
    fn marginalize_matches_naive() {
        let p = Factor::new(
            vec![v(0), v(1), v(2)],
            vec![2, 3, 2],
            (0..12).map(|i| i as f64 * 0.5 + 0.1).collect(),
        );
        let m = p.marginalize_to(&[v(0), v(2)]);
        for a0 in 0..2 {
            for a2 in 0..2 {
                let mut want = 0.0;
                for a1 in 0..3 {
                    want += eval(&p, &[a0, a1, a2]);
                }
                assert!((m.at(&[a0, a2]) - want).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn divide_zero_over_zero_is_zero() {
        let mut f = Factor::new(vec![v(0)], vec![2], vec![0.0, 0.5]);
        let g = Factor::new(vec![v(0)], vec![2], vec![0.0, 0.25]);
        f.divide_assign(&g);
        assert_eq!(f.data, vec![0.0, 2.0]);
    }

    #[test]
    fn from_axes_permutes() {
        // data over axes [1, 0] with cards [3, 2]; canonical is [0, 1].
        let data: Vec<f64> = (0..6).map(|i| i as f64).collect();
        let f = Factor::from_axes(&[v(1), v(0)], &[3, 2], &data);
        assert_eq!(f.vars, vec![v(0), v(1)]);
        assert_eq!(f.cards, vec![2, 3]);
        for b in 0..3 {
            for a in 0..2 {
                // source index over [1,0] layout: b*2 + a
                assert_eq!(f.at(&[a, b]), data[b * 2 + a]);
            }
        }
    }

    #[test]
    fn max_out_records_argmax() {
        let f = Factor::new(vec![v(0), v(1)], vec![2, 2], vec![1.0, 3.0, 5.0, 2.0]);
        let (m, arg) = f.max_out(v(1));
        assert_eq!(m.data, vec![3.0, 5.0]);
        assert_eq!(arg, vec![1, 0]);
    }

    #[test]
    fn product_with_scalar() {
        let f = Factor::new(vec![v(0)], vec![2], vec![0.3, 0.7]);
        let s = Factor::scalar(2.0);
        let p = f.product(&s);
        assert_eq!(p.data, vec![0.6, 1.4]);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let cards = [2usize, 3, 4];
        for i in 0..24 {
            assert_eq!(encode_index(&decode_index(i, &cards), &cards), i);
        }
    }
}
