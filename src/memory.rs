//! Checked, fallible allocation helpers shared by media domains.
use crate::{Result, invalid};

pub(crate) fn buffer(size: usize) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(size)
        .map_err(|_| invalid("frame allocation failed"))?;
    result.resize(size, 0);
    Ok(result)
}
