pub fn read_data(bits: &mut BitReader<'_>, max_sfb: u8) -> Result<super::aac_ld_ltp::LdLtpData> {
    if max_sfb > 37 {
        return Err(invalid("AAC LD max_sfb exceeds LTP geometry"));
    }
    let mut trial = bits.clone();
    let lag_update = if trial.bit()? {
        Some(trial.read(10)? as u16)
    } else {
        None
    };
    let coefficient_index = trial.read(3)? as u8;
    let mut used = Vec::with_capacity(usize::from(max_sfb));
    for _ in 0..max_sfb {
        used.push(trial.bit()?);
    }
    *bits = trial;
    Ok(super::aac_ld_ltp::LdLtpData {
        lag_update,
        coefficient_index,
        used,
    })
}
/// Common-window ER predictors are deferred until after the MS mask and
/// between channel bodies. Independent predictors are read in this header.
pub fn read_ics(
    bits: &mut BitReader<'_>,
    bands: u8,
    deferred: bool,
) -> Result<(
    super::aac_ics::IcsInfo,
    Option<super::aac_ld_ltp::LdLtpData>,
    bool,
)> {
    use super::aac_synthesis::{WindowSequence, WindowShape};
    let mut trial = bits.clone();
    if bands > 37 || trial.bit()? {
        return Err(invalid("invalid AAC LD ICS geometry or reserved bit"));
    }
    if trial.read(2)? != 0 {
        return Err(invalid("AAC LD requires ONLY_LONG_SEQUENCE"));
    }
    // Shape one means LD low-overlap; only LD synthesis interprets this tag.
    let shape = if trial.bit()? {
        WindowShape::Kbd
    } else {
        WindowShape::Sine
    };
    let max_sfb = trial.read(6)? as u8;
    if max_sfb > bands {
        return Err(invalid("AAC LD max_sfb exceeds band table"));
    }
    let present = trial.bit()?;
    let data = if present && !deferred && trial.bit()? {
        Some(read_data(&mut trial, max_sfb)?)
    } else {
        None
    };
    let info = super::aac_ics::IcsInfo {
        sequence: WindowSequence::OnlyLong,
        shape,
        max_sfb,
        group_lengths: vec![1],
        prediction: None,
    };
    *bits = trial;
    Ok((info, data, present))
}
