//! Pure table math for the CPT editor (port of the React `cptMath.ts`,
//! unit-tested).

/// Normalize each row of `out_card` values to sum 1 (zero rows unchanged).
pub fn normalize_rows(data: &[f64], out_card: usize) -> Vec<f64> {
    let mut out = data.to_vec();
    for row in out.chunks_mut(out_card) {
        let sum: f64 = row.iter().sum();
        if sum > 0.0 {
            for v in row {
                *v /= sum;
            }
        }
    }
    out
}

/// Every cell = 1 / out_card.
pub fn uniform(len: usize, out_card: usize) -> Vec<f64> {
    vec![1.0 / out_card as f64; len]
}

pub fn row_sum(data: &[f64], row: usize, out_card: usize) -> f64 {
    data[row * out_card..(row + 1) * out_card].iter().sum()
}

pub fn row_sum_ok(sum: f64) -> bool {
    (sum - 1.0).abs() < 1e-6
}

/// Clamp a probability cell; utility cells are unbounded.
pub fn clamp_cell(v: f64, is_utility: bool) -> f64 {
    if is_utility { v } else { v.clamp(0.0, 1.0) }
}

/// Up to 5 decimals, no trailing-zero noise (cell display).
pub fn format_cell(v: f64) -> String {
    let rounded = (v * 1e5).round() / 1e5;
    let s = format!("{rounded}");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_rows_scales_each_row() {
        let data = vec![2.0, 2.0, 0.0, 0.0, 1.0, 3.0];
        let out = normalize_rows(&data, 2);
        assert_eq!(out[0], 0.5);
        assert_eq!(out[1], 0.5);
        // Zero row unchanged.
        assert_eq!(&out[2..4], &[0.0, 0.0]);
        assert_eq!(out[4], 0.25);
        assert_eq!(out[5], 0.75);
    }

    #[test]
    fn uniform_fills() {
        assert_eq!(uniform(4, 4), vec![0.25; 4]);
    }

    #[test]
    fn row_sums() {
        let data = vec![0.2, 0.8, 0.5, 0.6];
        assert!((row_sum(&data, 0, 2) - 1.0).abs() < 1e-12);
        assert!((row_sum(&data, 1, 2) - 1.1).abs() < 1e-12);
        assert!(row_sum_ok(1.0000001));
        assert!(!row_sum_ok(1.1));
    }

    #[test]
    fn clamp_cell_bounds_probabilities_only() {
        assert_eq!(clamp_cell(1.5, false), 1.0);
        assert_eq!(clamp_cell(-0.5, false), 0.0);
        assert_eq!(clamp_cell(-42.0, true), -42.0);
    }

    #[test]
    fn format_cell_drops_noise() {
        assert_eq!(format_cell(0.5), "0.5");
        assert_eq!(format_cell(1.0 / 3.0), "0.33333");
        assert_eq!(format_cell(1.0), "1");
    }
}
