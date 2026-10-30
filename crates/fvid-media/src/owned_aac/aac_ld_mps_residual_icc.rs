//! Residual ICC corrections in the original parameter quantization domain.
//! Raw syntax and entropy histories remain unchanged. This is not residual AAC
//! reconstruction or native playback admission.
use super::{
    Result,
    aac_ld_mps_config::{Config, ExtensionValue},
    aac_ld_mps_entropy::Kind,
    aac_ld_mps_frame::ParameterId,
    aac_ld_mps_prepare::{Prepared, dequantize},
    aac_ld_mps_residual::Frame,
    invalid,
};
/// ISO mapIndexData adds the nonnegative difference before coarse2fine.
/// Out-of-grid values are invalid; the reference's unchecked lookup is not a
/// saturation rule. Coarse input is supplied in the parser's fine-domain units.
pub fn index(base: i16, coarse: bool, difference: u8) -> Result<i16> {
    if !(0..=7).contains(&base) || difference > 7 || coarse && (base % 2 != 0 || base > 6) {
        return Err(invalid("invalid residual ICC base or difference"));
    }
    let value = base + i16::from(difference) * if coarse { 2 } else { 1 };
    if value > 7 {
        return Err(invalid("residual ICC correction exceeds quantization grid"));
    }
    Ok(value)
}
/// Returns a corrected copy. The raw Frame/ParameterRows and original Prepared
/// are preserved, including the shared ICC region. Shared ICC is materialized
/// per non-LFE OTT box so different residual corrections cannot leak to siblings.
/// MDCT payload contents are handled by the separate residual codec stage.
pub fn apply(
    config: &Config,
    extension: &ExtensionValue,
    prepared: &Prepared,
    residual: &Frame,
) -> Result<Prepared> {
    let ExtensionValue::Residual { bands, .. } = extension else {
        return Err(invalid("expected residual ICC configuration"));
    };
    if !config.extensions.iter().any(|e| &e.value == extension)
        || bands.len() != config.tree.ott_lfe.len() + usize::from(config.tree.ttt_boxes)
        || residual.boxes.len() != bands.len()
    {
        return Err(invalid(
            "residual ICC box layout disagrees with configuration",
        ));
    }
    let sets = prepared.syntax.slots.len();
    if !(1..=2).contains(&sets) || !(sets..=sets + 1).contains(&prepared.slots.len()) {
        return Err(invalid("invalid residual ICC parameter-set layout"));
    }
    let mut next = prepared.clone();
    for (box_index, band) in bands.iter().enumerate() {
        let count = usize::from(band.unwrap_or(0));
        if count > usize::from(config.parameter_bands) {
            return Err(invalid("residual ICC band range exceeds resolution"));
        }
        let Some(data) = &residual.boxes[box_index] else {
            if count != 0 {
                return Err(invalid("missing residual ICC box"));
            }
            continue;
        };
        if count == 0 {
            return Err(invalid("unexpected inactive residual ICC box"));
        }
        if box_index >= config.tree.ott_lfe.len() {
            if !data.icc_differences.is_empty() {
                return Err(invalid("TTT residual has OTT ICC corrections"));
            }
            continue;
        }
        if data.icc_differences.len() != sets
            || data
                .icc_differences
                .iter()
                .flatten()
                .any(|r| r.len() != count || r.iter().any(|&v| v > 7))
        {
            return Err(invalid("invalid residual ICC difference dimensions"));
        }
        if config.tree.ott_lfe[box_index] {
            continue;
        }
        let source_id = ParameterId::OttIcc(if config.one_icc == Some(true) {
            None
        } else {
            Some(box_index as u8)
        });
        let raw = prepared
            .syntax
            .parameters
            .iter()
            .find(|p| p.id == source_id)
            .ok_or_else(|| invalid("missing raw residual ICC parameters"))?;
        if raw.kind != Kind::Icc
            || raw.coarse.len() != sets
            || raw.values.len() != sets
            || raw.start_band != 0
            || raw
                .values
                .iter()
                .any(|r| r.len() != usize::from(config.parameter_bands))
        {
            return Err(invalid("invalid raw residual ICC quantization metadata"));
        }
        let source = prepared
            .parameters
            .iter()
            .find(|p| p.id == source_id)
            .ok_or_else(|| invalid("missing physical residual ICC parameters"))?;
        if source.indices.len() != prepared.slots.len()
            || source.values.len() != prepared.slots.len()
        {
            return Err(invalid("invalid physical residual ICC endpoints"));
        }
        let mut corrected = source.clone();
        corrected.id = ParameterId::OttIcc(Some(box_index as u8));
        for set in 0..sets {
            // Always derive from raw syntax, making a second application idempotent.
            corrected.indices[set] = raw.values[set].clone();
            if let Some(differences) = &data.icc_differences[set] {
                for (band, &difference) in differences.iter().enumerate() {
                    corrected.indices[set][band] =
                        index(raw.values[set][band], raw.coarse[set], difference)?;
                }
            }
            corrected.values[set] = corrected.indices[set]
                .iter()
                .map(|&v| dequantize(Kind::Icc, v))
                .collect::<Result<_>>()?;
        }
        if prepared.slots.len() > sets {
            corrected.indices[sets] = corrected.indices[sets - 1].clone();
            corrected.values[sets] = corrected.values[sets - 1].clone();
        }
        if let Some(at) = next.parameters.iter().position(|p| p.id == corrected.id) {
            next.parameters[at] = corrected;
        } else {
            next.parameters.push(corrected);
        }
    }
    Ok(next)
}
