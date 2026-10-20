//! Device stereo fold in owned AAC emitted-PCM position order.
use super::{
    aac_pce::{ChannelPosition, HeightLayer, Position},
    config::AudioSpecificConfig,
    NativeAacDecoder,
};
use crate::owned_stereo_router::StereoRouter;
fn weights(position: ChannelPosition, total: u16) -> [f32; 2] {
    if position.position == Position::Lfe {
        return [0., 0.];
    }
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let n = position.group_channels;
    let index = position.index;
    if n % 2 == 1 && index == 0 {
        let gain = if total == 1
            && position.height == HeightLayer::Normal
            && position.position == Position::Front
        {
            1.
        } else {
            k
        };
        return [gain, gain];
    }
    let gain = if position.height == HeightLayer::Normal
        && position.position == Position::Front
        && index >= n.saturating_sub(2)
    {
        1.
    } else {
        k
    };
    if (index - u8::from(n % 2 == 1)) % 2 == 0 {
        [gain, 0.]
    } else {
        [0., gain]
    }
}
/// Preserve explicit PCE positions even when duplicate/bottom/side positions
/// cannot be represented in a WAVE mask. Parametric stereo emits FL/FR.
pub fn stereo_router(
    asc: &[u8],
    output_rate: u32,
    output_channels: u16,
    in_band_ps: bool,
) -> Result<StereoRouter, String> {
    let parsed = AudioSpecificConfig::parse(asc).map_err(|e| e.to_string())?;
    if in_band_ps || parsed.ps_present == Some(true) {
        if output_channels != 2 {
            return Err("PS output layout requires two channels".into());
        }
        return StereoRouter::new(2, 3);
    }
    let decoder =
        NativeAacDecoder::new_with_output_rate(asc, output_rate).map_err(|e| e.to_string())?;
    if u16::from(decoder.channels()) != output_channels {
        return Err("AAC routing channels disagree with decoded layout".into());
    }
    if let Some(positions) = decoder.channel_positions().map_err(|e| e.to_string())? {
        if positions.len() != usize::from(output_channels) {
            return Err("AAC position count disagrees with decoded layout".into());
        }
        let mut coefficients = Vec::new();
        coefficients
            .try_reserve_exact(positions.len())
            .map_err(|_| "AAC routing allocation failed")?;
        coefficients.extend(positions.into_iter().map(|p| weights(p, output_channels)));
        StereoRouter::from_coefficients(&coefficients)
    } else {
        StereoRouter::new(output_channels, decoder.channel_mask())
    }
}
