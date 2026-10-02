//! Shared decoded geometry frame storage.
#[derive(Debug)]
pub struct GeometryFrame {
    pub width: usize,
    pub height: usize,
    /// Horizontal/vertical luma samples per chroma sample; None for RGB.
    /// A quarter-turn swaps the axes (4:2:2 becomes 4:4:0).
    pub subsampling: Option<[usize; 2]>,
    /// Packed RGB or packed Y, Cb, Cr. Sample depth is unchanged;
    /// chroma axes follow `subsampling`.
    pub data: Vec<u8>,
}

pub(crate) fn buffer(size: usize) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    data.try_reserve_exact(size).map_err(|e| e.to_string())?;
    data.resize(size, 0);
    Ok(data)
}
