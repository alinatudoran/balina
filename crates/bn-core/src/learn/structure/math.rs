//! Special functions needed by scoring (BDeu) and CI tests (G²), kept
//! in-house to avoid a stats dependency. Accuracy is far beyond what the
//! learning algorithms need (~1e-13 relative for `ln_gamma`).

/// Natural log of the gamma function for x > 0 (Lanczos, g = 7, n = 9).
pub(crate) fn ln_gamma(x: f64) -> f64 {
    debug_assert!(x > 0.0, "ln_gamma domain: x > 0, got {x}");
    const G: f64 = 7.0;
    const COEF: [f64; 9] = [
        0.999_999_999_999_809_93,
        676.520_368_121_885_1,
        -1259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection: Γ(x)Γ(1−x) = π / sin(πx).
        return std::f64::consts::PI.ln()
            - (std::f64::consts::PI * x).sin().ln()
            - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = COEF[0];
    let t = x + G + 0.5;
    for (i, &c) in COEF.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Chi-squared survival function P(X > x) with `df` degrees of freedom:
/// the upper regularized incomplete gamma Q(df/2, x/2).
pub(crate) fn chi2_sf(x: f64, df: f64) -> f64 {
    debug_assert!(df > 0.0);
    if x <= 0.0 {
        return 1.0;
    }
    gamma_q(df / 2.0, x / 2.0)
}

/// Upper regularized incomplete gamma Q(a, x) = 1 − P(a, x), a > 0, x ≥ 0.
fn gamma_q(a: f64, x: f64) -> f64 {
    if x < a + 1.0 {
        1.0 - gamma_p_series(a, x)
    } else {
        gamma_q_cf(a, x)
    }
}

/// Lower regularized incomplete gamma by series expansion (x < a + 1).
fn gamma_p_series(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let ln_ga = ln_gamma(a);
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut del = sum;
    for _ in 0..500 {
        ap += 1.0;
        del *= x / ap;
        sum += del;
        if del.abs() < sum.abs() * 1e-15 {
            break;
        }
    }
    sum * (a * x.ln() - x - ln_ga).exp()
}

/// Upper regularized incomplete gamma by continued fraction (Lentz).
fn gamma_q_cf(a: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let ln_ga = ln_gamma(a);
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / TINY;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..500 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b + an / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-15 {
            break;
        }
    }
    (a * x.ln() - x - ln_ga).exp() * h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ln_gamma_matches_factorials() {
        let mut fact = 1.0f64;
        for n in 1..=15u32 {
            // ln Γ(n) = ln (n−1)!
            assert!((ln_gamma(n as f64) - fact.ln()).abs() < 1e-12, "n = {n}");
            fact *= n as f64;
        }
        let half = std::f64::consts::PI.sqrt().ln();
        assert!((ln_gamma(0.5) - half).abs() < 1e-12);
        // Small fractional argument via reflection.
        assert!((ln_gamma(0.1) - 2.252_712_651_734_205_9).abs() < 1e-10);
    }

    #[test]
    fn chi2_sf_known_quantiles() {
        // 95th percentile of chi²(1) is 3.841…
        assert!((chi2_sf(3.841_458_820_694_124, 1.0) - 0.05).abs() < 1e-9);
        // 95th percentile of chi²(5) is 11.0705.
        assert!((chi2_sf(11.070_497_693_516_35, 5.0) - 0.05).abs() < 1e-9);
        assert!((chi2_sf(0.0, 3.0) - 1.0).abs() < 1e-15);
        // Monotone decreasing in x.
        let mut prev = 1.0;
        for i in 1..40 {
            let p = chi2_sf(i as f64 * 0.5, 4.0);
            assert!(p < prev);
            prev = p;
        }
    }
}
