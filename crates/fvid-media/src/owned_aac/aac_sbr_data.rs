//! Owned legacy non-scalable SBR single-channel/channel-pair data syntax.
//! Parses quantized delta rows; reconstruction and PCM synthesis are separate.
use super::{
    Result,
    aac_sbr_bands::FrequencyTables,
    aac_sbr_coefficients::{self, DeltaRow},
    aac_sbr_controls::{self, DeltaFlags, InverseFilterMode},
    aac_sbr_grid::GridSyntax,
    aac_sbr_header::Header,
    aac_sbr_mapping::read_harmonics,
    bits::BitReader,
    invalid,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub grid: GridSyntax,
    pub delta: DeltaFlags,
    pub inverse_filter: Vec<InverseFilterMode>,
    pub envelope: Vec<DeltaRow>,
    pub noise: Vec<DeltaRow>,
    pub harmonics: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Data {
    pub frequency: FrequencyTables,
    pub coupled: bool,
    pub channels: Vec<Channel>,
    /// Complete extension area, in transmitted order. Retaining these bytes
    /// does not decode PS or any other extension. A synthesis caller must
    /// interpret supported extensions or explicitly reject unsupported ones.
    /// None distinguishes an absent flag from a present zero-length area.
    pub extended_data: Option<Vec<u8>>,
}

fn field(bits: &mut BitReader<'_>, end: usize, width: u8) -> Result<u8> {
    if end < bits.position()
        || end - bits.position() > bits.remaining()
        || usize::from(width) > end - bits.position()
    {
        return Err(invalid("truncated or invalid SBR data payload"));
    }
    Ok(bits.read(width)? as u8)
}

impl Data {
    /// Read sbr_data for a non-scalable AAC SCE (one channel) or CPE (two).
    /// `end` is the enclosing extension's absolute bit boundary, not a byte
    /// count. The reader commits only after the entire data syntax succeeds.
    /// The caller handles the outer header, CRC and trailing fill bits.
    pub fn read(
        bits: &mut BitReader<'_>,
        end: usize,
        header: &Header,
        sbr_rate: u32,
        slots: u8,
        channel_count: usize,
    ) -> Result<Self> {
        if !(1..=2).contains(&channel_count) || !matches!(slots, 15 | 16) {
            return Err(invalid("invalid non-scalable SBR element dimensions"));
        }
        let frequency = FrequencyTables::from_header(header, sbr_rate)?;
        let mut trial = bits.clone();
        if field(&mut trial, end, 1)? != 0 {
            // The SCE has one reserved nibble; the CPE has two.
            for _ in 0..channel_count {
                field(&mut trial, end, 4)?;
            }
        }
        let coupled = channel_count == 2 && field(&mut trial, end, 1)? != 0;
        let mut grids = Vec::with_capacity(channel_count);
        for _ in 0..if coupled { 1 } else { channel_count } {
            grids.push(GridSyntax::read_validated(&mut trial, end, slots)?.0);
        }
        if coupled {
            grids.push(grids[0].clone());
        }
        let mut channels = Vec::with_capacity(channel_count);
        for grid in grids {
            let delta = DeltaFlags::read(&mut trial, end, &grid, slots)?;
            channels.push(Channel {
                grid,
                delta,
                inverse_filter: vec![],
                envelope: vec![],
                noise: vec![],
                harmonics: vec![],
            });
        }
        let noise_bands = frequency.noise.len() - 1;
        for channel in channels
            .iter_mut()
            .take(if coupled { 1 } else { channel_count })
        {
            channel.inverse_filter =
                aac_sbr_controls::read_inverse_filter(&mut trial, end, noise_bands)?;
        }
        if coupled {
            channels[1].inverse_filter = channels[0].inverse_filter.clone();
        }
        let read_envelope =
            |bits: &mut BitReader<'_>, channel: &mut Channel, balance| -> Result<()> {
                channel.envelope = aac_sbr_coefficients::read_envelopes(
                    bits,
                    end,
                    &channel.grid,
                    slots,
                    &channel.delta,
                    header.amplitude_resolution,
                    balance,
                    frequency.low.len() - 1,
                    frequency.high.len() - 1,
                )?;
                Ok(())
            };
        let read_noise = |bits: &mut BitReader<'_>, channel: &mut Channel, balance| -> Result<()> {
            channel.noise = aac_sbr_coefficients::read_noise(
                bits,
                end,
                &channel.grid,
                slots,
                &channel.delta,
                balance,
                noise_bands,
            )?;
            Ok(())
        };
        if coupled {
            // Coupled order is level envelope/noise, then balance envelope/noise.
            for (index, channel) in channels.iter_mut().enumerate() {
                read_envelope(&mut trial, channel, index == 1)?;
                read_noise(&mut trial, channel, index == 1)?;
            }
        } else {
            // Uncoupled order groups all envelopes before all noise rows.
            for channel in &mut channels {
                read_envelope(&mut trial, channel, false)?;
            }
            for channel in &mut channels {
                read_noise(&mut trial, channel, false)?;
            }
        }
        for channel in &mut channels {
            let present = field(&mut trial, end, 1)? != 0;
            channel.harmonics = read_harmonics(&mut trial, end, present, frequency.high.len() - 1)?;
        }
        let extended_data = if field(&mut trial, end, 1)? != 0 {
            let mut count = usize::from(field(&mut trial, end, 4)?);
            if count == 15 {
                count += usize::from(field(&mut trial, end, 8)?);
            }
            if count * 8 > end.saturating_sub(trial.position()) {
                return Err(invalid("truncated SBR extended data area"));
            }
            let mut bytes = Vec::with_capacity(count);
            for _ in 0..count {
                bytes.push(field(&mut trial, end, 8)?);
            }
            Some(bytes)
        } else {
            None
        };
        *bits = trial;
        Ok(Self {
            frequency,
            coupled,
            channels,
            extended_data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{aac_sbr_controls::DeltaDirection, aac_sbr_grid::FrameClass};
    use super::*;
    use serde_json::{Value, json};

    fn header(amplitude: bool) -> Header {
        Header {
            amplitude_resolution: amplitude,
            start_frequency: 0,
            stop_frequency: 14,
            crossover: 0,
            frequency_scale: 0,
            alter_scale: false,
            noise_bands: 3,
            limiter_bands: 2,
            limiter_gains: 2,
            interpolate_frequency: true,
            smoothing_mode: true,
        }
    }
    fn name(class: FrameClass) -> &'static str {
        match class {
            FrameClass::FixFix => "FixFix",
            FrameClass::FixVar => "FixVar",
            FrameClass::VarFix => "VarFix",
            FrameClass::VarVar => "VarVar",
        }
    }
    fn direction(value: DeltaDirection) -> &'static str {
        match value {
            DeltaDirection::Frequency => "Frequency",
            DeltaDirection::Time => "Time",
        }
    }
    fn channel_value(channel: &Channel) -> Value {
        let rows = |rows: &[DeltaRow]| {
            rows.iter()
                .map(|row| json!({"direction":direction(row.direction), "values":row.values}))
                .collect::<Vec<_>>()
        };
        json!({
            "grid": {
                "class_name":name(channel.grid.class),
                "leading_offset":channel.grid.leading_offset,
                "trailing_offset":channel.grid.trailing_offset,
                "leading_relative":channel.grid.leading_relative,
                "trailing_relative":channel.grid.trailing_relative,
                "pointer":channel.grid.pointer,
                "high_resolution":channel.grid.high_resolution,
            },
            "envelope_flags":channel.delta.envelope.iter().map(|d| *d == DeltaDirection::Time).collect::<Vec<_>>(),
            "noise_flags":channel.delta.noise.iter().map(|d| *d == DeltaDirection::Time).collect::<Vec<_>>(),
            "inverse_filter":channel.inverse_filter.iter().map(|m| match m {
                InverseFilterMode::Off => 0, InverseFilterMode::Low => 1,
                InverseFilterMode::Intermediate => 2, InverseFilterMode::High => 3,
            }).collect::<Vec<_>>(),
            "envelope":rows(&channel.envelope), "noise":rows(&channel.noise),
            "harmonics":channel.harmonics,
        })
    }
    fn pack(source: &[u8], length: usize, offset: usize) -> Vec<u8> {
        let mut bytes = vec![255; (offset + length + 16).div_ceil(8)];
        for i in 0..length {
            if source[i / 8] & (1 << (7 - i % 8)) == 0 {
                let p = offset + i;
                bytes[p / 8] &= !(1 << (7 - p % 8));
            }
        }
        bytes
    }

    #[test]
    fn original_sce_cpe_payloads_and_every_truncation_are_transactional() {
        let manifest: Value = serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-data-syntax.json"
        ))
        .unwrap();
        let binary =
            include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-data-syntax.bin");
        assert_eq!(manifest["vectors"].as_array().unwrap().len(), 81);
        for (case_index, case) in manifest["vectors"].as_array().unwrap().iter().enumerate() {
            let length = case["bit_length"].as_u64().unwrap() as usize;
            let source = &binary[case["offset"].as_u64().unwrap() as usize..][..length.div_ceil(8)];
            let slots = case["slots"].as_u64().unwrap() as u8;
            let header = header(case["amplitude"].as_bool().unwrap());
            let count = case["channels"].as_array().unwrap().len();
            for offset in 0..8 {
                let bytes = pack(source, length, offset);
                let mut bits = BitReader::new(&bytes);
                bits.skip(offset).unwrap();
                for end in offset..offset + length {
                    assert!(
                        Data::read(&mut bits, end, &header, 48_000, slots, count).is_err(),
                        "case {case_index}, offset {offset}, end {end}"
                    );
                    assert_eq!(bits.position(), offset);
                }
                let result = Data::read(&mut bits, offset + length, &header, 48_000, slots, count)
                    .unwrap_or_else(|e| panic!("case {case_index}, offset {offset}: {e}"));
                assert_eq!(bits.position(), offset + length);
                assert_eq!(bits.read(8).unwrap(), 255);
                assert_eq!(result.coupled, case["coupled"].as_bool().unwrap());
                assert_eq!(json!(result.extended_data), case["extended_data"]);
                assert_eq!(
                    json!(
                        result
                            .channels
                            .iter()
                            .map(channel_value)
                            .collect::<Vec<_>>()
                    ),
                    case["channels"],
                    "case {case_index}"
                );
                assert_eq!(result.frequency.high, [7, 8, 9, 10, 11, 12, 14]);
                assert_eq!(result.frequency.low, [7, 9, 11, 14]);
                assert_eq!(result.frequency.noise, [7, 9, 11, 14]);
            }
        }
    }

    #[test]
    fn invalid_dimensions_boundaries_and_grid_preserve_reader() {
        let bytes = [255; 16];
        let mut bits = BitReader::new(&bytes);
        bits.skip(3).unwrap();
        for count in [0, 3, usize::MAX] {
            assert!(Data::read(&mut bits, 80, &header(true), 48_000, 16, count).is_err());
            assert_eq!(bits.position(), 3);
        }
        for slots in [0, 14, 17, 255] {
            assert!(Data::read(&mut bits, 80, &header(true), 48_000, slots, 1).is_err());
            assert_eq!(bits.position(), 3);
        }
        for end in [0, 2, 3, 129, usize::MAX] {
            assert!(Data::read(&mut bits, end, &header(true), 48_000, 16, 1).is_err());
            assert_eq!(bits.position(), 3);
        }
        // data_extra=0, FIXFIX, eight envelopes: well-formed raw syntax but
        // invalid legacy time geometry. Semantic rejection also rolls back.
        let mut invalid_grid = BitReader::new(&[0b0001_1100, 0, 0, 0]);
        assert!(Data::read(&mut invalid_grid, 32, &header(true), 48_000, 16, 1).is_err());
        assert_eq!(invalid_grid.position(), 0);
        let mut invalid_header = header(true);
        invalid_header.start_frequency = 16;
        assert!(Data::read(&mut bits, 80, &invalid_header, 48_000, 16, 2).is_err());
        assert_eq!(bits.position(), 3);
    }
}
