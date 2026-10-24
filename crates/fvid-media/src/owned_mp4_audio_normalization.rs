//! Speaker routing shared by MP4 presentation exporters in both library layers.
/// Input-channel rows in emitted PCM order, destination-channel columns.
pub fn coefficients(
    codec: [u8; 4],
    configuration: &[u8],
    rate: u32,
    input: u16,
    mask: u32,
    in_band_ps: bool,
    output: u16,
    destination: u32,
) -> Result<Vec<Vec<f64>>, String> {
    if output == 0 || output > 8 {
        return Err("unsupported MP4 normalization destination layout".into());
    }
    if input == output && mask != 0 && mask == destination {
        return Ok((0..usize::from(input))
            .map(|i| {
                (0..usize::from(output))
                    .map(|j| if i == j { 1. } else { 0. })
                    .collect()
            })
            .collect());
    }
    if destination == 0 || (output > 1 && destination & 3 != 3 && mask != destination) {
        return Err("MP4 normalization requires an explicit destination speaker layout".into());
    }
    if codec == *b"mp4a"
        && ((matches!(output, 1 | 2)
            && crate::owned_pcm_channels::standard_mask(output) == Some(u64::from(destination)))
            || mask == 0)
    {
        let asc = crate::owned_codec_config::aac_specific_config(configuration)
            .map_err(|e| e.to_string())?;
        let router =
            crate::owned_aac::speaker_routing::stereo_router(asc, rate, input, in_band_ps)?;
        let expansion = if output > 2 {
            Some(crate::owned_pcm_channels::normalization_coefficients(
                2,
                output,
                3,
                destination,
            )?)
        } else {
            None
        };
        return Ok(router
            .coefficients()
            .iter()
            .map(|w| {
                if output == 1 {
                    vec![(f64::from(w[0]) + f64::from(w[1])) * 0.5]
                } else if let Some(matrix) = &expansion {
                    (0..usize::from(output))
                        .map(|j| f64::from(w[0]) * matrix[0][j] + f64::from(w[1]) * matrix[1][j])
                        .collect()
                } else {
                    vec![f64::from(w[0]), f64::from(w[1])]
                }
            })
            .collect());
    }
    let matrix =
        crate::owned_pcm_channels::normalization_coefficients(input, output, mask, destination)?;
    Ok((0..usize::from(input))
        .map(|i| matrix[i][..usize::from(output)].to_vec())
        .collect())
}
