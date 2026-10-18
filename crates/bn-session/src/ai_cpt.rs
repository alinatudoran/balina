//! AI-suggested CPT rows: parsing the structured model's reply and
//! validating it against the node's table shape. Pure and offline, like
//! [`ai_draft`](crate::ai_draft) — the provider call lives in the app layer,
//! and errors are collected exhaustively so they can be fed back to the
//! model for a repair round-trip.

use serde::Deserialize;

/// Guardrail for one elicitation call; beyond this, data learning is the
/// honest tool anyway.
pub const MAX_CPT_ROWS: usize = 64;

#[derive(Deserialize)]
struct RowsReply {
    rows: Vec<Vec<f64>>,
}

/// Parse `{"rows": [[...], ...]}` out of a model reply, tolerating fences
/// and surrounding prose.
pub fn parse_rows_reply(reply: &str) -> Result<Vec<Vec<f64>>, String> {
    let start = reply.find('{').ok_or("reply contains no JSON object")?;
    let end = reply.rfind('}').ok_or("reply contains no JSON object")?;
    if start >= end {
        return Err("reply contains no JSON object".into());
    }
    serde_json::from_str::<RowsReply>(&reply[start..=end])
        .map(|r| r.rows)
        .map_err(|e| format!("invalid rows JSON: {e}"))
}

/// Check shape and content, then return the row-normalized flat table in the
/// engine's `[parent_0..k, self]` layout. Suggested rows are distributions,
/// so each row must have positive mass; exact sums are forgiven by
/// normalization.
pub fn validate_rows(
    rows: &[Vec<f64>],
    row_count: usize,
    out_card: usize,
) -> Result<Vec<f64>, Vec<String>> {
    let mut errors = Vec::new();
    if rows.len() != row_count {
        errors.push(format!("expected {row_count} rows, got {}", rows.len()));
    }
    for (r, row) in rows.iter().enumerate().take(row_count) {
        if row.len() != out_card {
            errors.push(format!("row {r}: expected {out_card} values, got {}", row.len()));
            continue;
        }
        if row.iter().any(|v| !v.is_finite() || *v < 0.0) {
            errors.push(format!("row {r}: values must be finite and >= 0"));
            continue;
        }
        if row.iter().sum::<f64>() <= 0.0 {
            errors.push(format!("row {r}: all values are zero"));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut data = Vec::with_capacity(row_count * out_card);
    for row in rows {
        let sum: f64 = row.iter().sum();
        data.extend(row.iter().map(|v| v / sum));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tolerates_fence_and_prose() {
        let rows =
            parse_rows_reply("Sure:\n```json\n{\"rows\": [[0.9, 0.1], [0.2, 0.8]]}\n```").unwrap();
        assert_eq!(rows, vec![vec![0.9, 0.1], vec![0.2, 0.8]]);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_rows_reply("no json").is_err());
        assert!(parse_rows_reply("{\"cols\": []}").is_err());
    }

    #[test]
    fn validate_normalizes_sloppy_sums() {
        let data = validate_rows(&[vec![8.0, 2.0], vec![1.0, 3.0]], 2, 2).unwrap();
        assert_eq!(data, vec![0.8, 0.2, 0.25, 0.75]);
    }

    #[test]
    fn validate_rejects_wrong_shape() {
        let errs = validate_rows(&[vec![1.0, 0.0]], 2, 2).unwrap_err();
        assert!(errs[0].contains("expected 2 rows"), "{errs:?}");
        let errs = validate_rows(&[vec![1.0], vec![1.0, 0.0]], 2, 2).unwrap_err();
        assert!(errs[0].contains("row 0: expected 2 values"), "{errs:?}");
    }

    #[test]
    fn validate_rejects_bad_values_per_row() {
        let errs =
            validate_rows(&[vec![-0.1, 1.1], vec![f64::NAN, 1.0], vec![0.0, 0.0]], 3, 2)
                .unwrap_err();
        assert_eq!(errs.len(), 3, "{errs:?}");
        assert!(errs[2].contains("all values are zero"));
    }
}
