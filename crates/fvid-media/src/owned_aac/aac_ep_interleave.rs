//! Owned inverse recursive EP stage, ISO/IEC 14496-3 section 1.8.4.8.1.
//! Symbols are bytes so the primitive also supports octet-oriented stages.
use super::{Result, invalid};

/// Undo the occupied-cell interleaver. X was filled in row order; Y occupied
/// the remaining cells in column order. The transmitted stream is column order.
/// No padding cells are added, including when the final row is incomplete.
pub fn split_stage(encoded: &[u8], x_len: usize, width: usize) -> Result<(Vec<u8>, Vec<u8>)> {
    if width == 0 {
        return Err(invalid("zero EP interleave width"));
    }
    if x_len > encoded.len() {
        return Err(invalid("truncated EP interleave stage"));
    }
    let mut x = Vec::new();
    let mut y = Vec::new();
    x.try_reserve_exact(x_len)
        .map_err(|_| invalid("EP interleave allocation failed"))?;
    y.try_reserve_exact(encoded.len() - x_len)
        .map_err(|_| invalid("EP interleave allocation failed"))?;
    x.resize(x_len, 0);
    let full_rows = encoded.len() / width;
    let last_columns = encoded.len() % width;
    let x_rows = x_len / width;
    let x_columns = x_len % width;
    let mut start = 0;
    for column in 0..width.min(encoded.len()) {
        let count = full_rows + usize::from(column < last_columns);
        let protected = x_rows + usize::from(column < x_columns);
        for row in 0..protected {
            x[row * width + column] = encoded[start + row];
        }
        y.extend_from_slice(&encoded[start + protected..start + count]);
        start += count;
    }
    debug_assert_eq!(start, encoded.len());
    Ok((x, y))
}

/// Reverse an RS stage whose X octets occupy eight consecutive cells in
/// each column; Y may contain a non-octet-aligned SRCPC stream.
pub fn split_octet_stage(encoded: &[u8], x_len: usize) -> Result<(Vec<u8>, Vec<u8>)> {
    if x_len % 8 != 0 {
        return Err(invalid("EP RS stage is not octet aligned"));
    }
    if x_len > encoded.len() {
        return Err(invalid("truncated EP RS interleave stage"));
    }
    if x_len == 0 {
        return Ok((vec![], encoded.to_vec()));
    }
    let width = x_len / 8;
    let mut x = Vec::new();
    let mut y = Vec::new();
    x.try_reserve_exact(x_len)
        .map_err(|_| invalid("EP interleave allocation failed"))?;
    y.try_reserve_exact(encoded.len() - x_len)
        .map_err(|_| invalid("EP interleave allocation failed"))?;
    let rows = encoded.len() / width;
    let extra = encoded.len() % width;
    let mut start = 0;
    for col in 0..width {
        let count = rows + usize::from(col < extra);
        x.extend_from_slice(&encoded[start..start + 8]);
        y.extend_from_slice(&encoded[start + 8..start + count]);
        start += count;
    }
    Ok((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inverse_stages_match_independent_occupied_cell_matrix() {
        let m: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ep-interleave/manifest.json"
        ))
        .unwrap();
        let bytes = include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-ep-interleave/vectors.bin"
        );
        let rows = m["vectors"].as_array().unwrap();
        assert!(rows.len() > 10_000);
        for row in rows {
            let x = row["x"].as_u64().unwrap() as usize;
            let y = row["y"].as_u64().unwrap() as usize;
            let width = row["width"].as_u64().unwrap() as usize;
            let at = row["offset"].as_u64().unwrap() as usize;
            let expected_x = &bytes[at..at + x];
            let expected_y = &bytes[at + x..at + x + y];
            let encoded = &bytes[at + x + y..at + 2 * (x + y)];
            let (actual_x, actual_y) = split_stage(encoded, x, width).unwrap();
            assert_eq!(actual_x, expected_x, "X={x} Y={y} width={width}");
            assert_eq!(actual_y, expected_y, "X={x} Y={y} width={width}");
        }
    }
    #[test]
    fn rs_column_stages_match_independent_mixed_unit_oracle() {
        let m: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ep-class-codes/manifest.json"
        ))
        .unwrap();
        let bytes = include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-ep-class-codes/vectors.bin"
        );
        for row in m["octet_stages"].as_array().unwrap() {
            let x = row["x"].as_u64().unwrap() as usize;
            let y = row["y"].as_u64().unwrap() as usize;
            let at = row["offset"].as_u64().unwrap() as usize;
            let (actual_x, actual_y) =
                split_octet_stage(&bytes[at + x + y..at + 2 * (x + y)], x).unwrap();
            assert_eq!(actual_x, &bytes[at..at + x]);
            assert_eq!(actual_y, &bytes[at + x..at + x + y]);
        }
        assert!(split_octet_stage(&[0; 9], 7).is_err());
        assert!(split_octet_stage(&[0; 7], 8).is_err());
        assert_eq!(
            split_octet_stage(&[1, 0, 1], 0).unwrap(),
            (vec![], vec![1, 0, 1])
        );
    }
    #[test]
    fn bounds_and_unoccupied_columns_do_not_wrap_or_pad() {
        assert!(split_stage(&[], 0, 0).is_err());
        assert!(split_stage(&[1], 2, 1).is_err());
        assert_eq!(split_stage(&[], 0, usize::MAX).unwrap(), (vec![], vec![]));
        assert_eq!(
            split_stage(&[7, 8, 9], 2, usize::MAX).unwrap(),
            (vec![7, 8], vec![9])
        );
        assert_eq!(
            split_stage(&[7, 8, 9], 0, 2).unwrap(),
            (vec![], vec![7, 8, 9])
        );
        assert_eq!(
            split_stage(&[7, 8, 9], 3, 2).unwrap(),
            (vec![7, 9, 8], vec![])
        );
    }
}
