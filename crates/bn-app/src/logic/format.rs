//! Number formatting matching the egui app's output (the React port copied
//! these; now they are Rust `format!` again).

use bn_core::model::State;

/// Percentage with `decimals` places, width-padded so columns align
/// (width fits "100" plus the decimal point and digits).
pub fn pct(p: f64, decimals: u8) -> String {
    let d = decimals as usize;
    let width = if d == 0 { 3 } else { d + 4 };
    format!("{:width$.d$}", p * 100.0)
}

/// `{:.4e}` (P(findings) display).
pub fn exp4(v: f64) -> String {
    format!("{v:.4e}")
}

/// Expected value of a node: Σ value·belief, if every state has a value.
pub fn expected_value(states: &[State], beliefs: Option<&[f64]>) -> Option<f64> {
    let beliefs = beliefs?;
    let mut acc = 0.0;
    for (i, s) in states.iter().enumerate() {
        acc += s.value? * beliefs.get(i).copied().unwrap_or(0.0);
    }
    Some(acc)
}

/// Variance of a node: Σ belief·(value − E[X])².
/// Returns `None` if any state lacks a numeric value or beliefs are unavailable.
pub fn variance(states: &[State], beliefs: Option<&[f64]>) -> Option<f64> {
    let ev = expected_value(states, beliefs)?;
    let beliefs = beliefs?;
    let mut acc = 0.0;
    for (i, s) in states.iter().enumerate() {
        let v = s.value?;
        acc += beliefs.get(i).copied().unwrap_or(0.0) * (v - ev) * (v - ev);
    }
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_pads_to_align_columns() {
        // 1 decimal → width 5 (the historical format).
        assert_eq!(pct(0.051, 1), "  5.1");
        assert_eq!(pct(1.0, 1), "100.0");
        assert_eq!(pct(0.0, 1), "  0.0");
        assert_eq!(pct(0.99999, 1), "100.0");
        // 2 decimals (default precision) → width 6.
        assert_eq!(pct(0.051, 2), "  5.10");
        assert_eq!(pct(1.0, 2), "100.00");
        // 0 decimals → width 3, no decimal point.
        assert_eq!(pct(0.051, 0), "  5");
        assert_eq!(pct(1.0, 0), "100");
        // 8 decimals (the maximum) → width 12.
        assert_eq!(pct(0.5, 8), " 50.00000000");
    }

    #[test]
    fn exp4_matches_rust_exponential() {
        assert_eq!(exp4(0.00003), "3.0000e-5");
        assert_eq!(exp4(0.5), "5.0000e-1");
    }

    #[test]
    fn variance_two_states() {
        let states = vec![
            State { name: "a".into(), value: Some(0.0) },
            State { name: "b".into(), value: Some(10.0) },
        ];
        // E[X] = 0.25*0 + 0.75*10 = 7.5
        // Var  = 0.25*(0-7.5)^2 + 0.75*(10-7.5)^2
        let expected_var = 0.25 * 56.25 + 0.75 * 6.25;
        let v = variance(&states, Some(&[0.25, 0.75]));
        assert!((v.unwrap() - expected_var).abs() < 1e-12);
        // Missing value → None
        let s2 = vec![
            State { name: "a".into(), value: None },
            State { name: "b".into(), value: Some(10.0) },
        ];
        assert!(variance(&s2, Some(&[0.5, 0.5])).is_none());
        assert!(variance(&states, None).is_none());
    }

    #[test]
    fn expected_value_needs_all_values() {
        let states = vec![
            State { name: "a".into(), value: Some(0.0) },
            State { name: "b".into(), value: Some(10.0) },
        ];
        let ev = expected_value(&states, Some(&[0.25, 0.75]));
        assert!((ev.unwrap() - 7.5).abs() < 1e-12);
        let states_missing = vec![
            State { name: "a".into(), value: None },
            State { name: "b".into(), value: Some(10.0) },
        ];
        assert!(expected_value(&states_missing, Some(&[0.5, 0.5])).is_none());
        assert!(expected_value(&states, None).is_none());
    }
}
