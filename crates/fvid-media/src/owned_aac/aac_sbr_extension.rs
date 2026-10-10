//! Transactional legacy non-scalable sbr_extension_data framing and CRC.
use super::{Result, aac_sbr_data::Data, aac_sbr_header::Header, bits::BitReader, invalid};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    header: Option<Header>,
    format: Option<(u32, u8, usize)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub header: Header,
    pub header_reset: bool,
    /// A rate, frame-size or channel-count change requires full stream reset,
    /// distinct from the SBR header reset that preserves some DSP phases.
    pub format_reset: bool,
    pub data: Data,
    pub crc: Option<u16>,
    /// Includes the four-bit extension type already consumed by the caller.
    pub consumed_bytes: usize,
}

fn crc10(mut bits: BitReader<'_>, end: usize) -> Result<u16> {
    if end < bits.position() || end - bits.position() > bits.remaining() {
        return Err(invalid("invalid SBR CRC range"));
    }
    let mut crc = 0u16;
    while bits.position() < end {
        let feedback = ((crc >> 9) ^ bits.read(1)? as u16) & 1;
        crc = (crc << 1) & 0x3ff;
        if feedback != 0 {
            crc ^= 0x233;
        }
    }
    Ok(crc)
}

impl State {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Read after EXT_SBR_DATA/EXT_SBR_DATA_CRC's four-bit type. `end` is
    /// the absolute FIL boundary, possibly including subsequent extensions.
    /// The caller has decoded the AAC element type to one/two channels.
    /// CRC covers header flag through this SBR element's fill bits, excluding
    /// its checksum, type and subsequent extensions. Both reader and retained
    /// header commit together only after syntax and CRC verification succeed.
    pub fn read(
        &mut self,
        bits: &mut BitReader<'_>,
        end: usize,
        crc_present: bool,
        sbr_rate: u32,
        slots: u8,
        channels: usize,
    ) -> Result<Frame> {
        let start = bits.position();
        if end < start || end - start > bits.remaining() {
            return Err(invalid("invalid SBR extension boundary"));
        }
        let available = end - start;
        // Ordinary FIL limits are enforced by its count parser. ER carries
        // multiple self-delimited SBR elements in one larger remaining region.
        // Preserve byte geometry and the checked packet boundary for both.
        if (available + 4) % 8 != 0 {
            return Err(invalid("invalid SBR extension byte count"));
        }
        let mut trial = bits.clone();
        let crc = if crc_present {
            if end - trial.position() < 10 {
                return Err(invalid("truncated SBR CRC"));
            }
            Some(trial.read(10)? as u16)
        } else {
            None
        };
        let crc_start = trial.clone();
        if trial.position() == end {
            return Err(invalid("missing SBR header flag"));
        }
        let header = if trial.read(1)? != 0 {
            Header::read(&mut trial, end)?
        } else {
            self.header
                .clone()
                .ok_or_else(|| invalid("missing initial SBR header"))?
        };
        let data = Data::read(&mut trial, end, &header, sbr_rate, slots, channels)?;
        let fill = (end - trial.position()) % 8;
        trial.skip(fill)?;
        if let Some(expected) = crc {
            if crc10(crc_start, trial.position())? != expected {
                return Err(invalid("SBR CRC mismatch"));
            }
        }
        let format = (sbr_rate, slots, channels);
        let result = Frame {
            header_reset: header.requires_reset(self.header.as_ref()),
            format_reset: self.format != Some(format),
            header: header.clone(),
            data,
            crc,
            consumed_bytes: (trial.position() - start + 4) / 8,
        };
        self.header = Some(header);
        self.format = Some(format);
        *bits = trial;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    const BINARY: &[u8] =
        include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-extension-syntax.bin");
    const DATA_BINARY: &[u8] =
        include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-data-syntax.bin");
    fn manifests() -> (Value, Value) {
        (
            serde_json::from_slice(include_bytes!(
                "../../../../tests/fixtures/playback-errors/aac-sbr-extension-syntax.json"
            ))
            .unwrap(),
            serde_json::from_slice(include_bytes!(
                "../../../../tests/fixtures/playback-errors/aac-sbr-data-syntax.json"
            ))
            .unwrap(),
        )
    }
    fn bytes(case: &Value) -> &[u8] {
        &BINARY[case["offset"].as_u64().unwrap() as usize..]
            [..case["byte_length"].as_u64().unwrap() as usize]
    }
    fn arguments(spec: &Value) -> (u8, usize) {
        (
            spec["slots"].as_u64().unwrap() as u8,
            spec["channels"].as_array().unwrap().len(),
        )
    }
    fn expected_header(spec: &Value) -> Header {
        Header {
            amplitude_resolution: spec["amplitude"].as_bool().unwrap(),
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
    fn initial_state(case: &Value, manifest: &Value, spec: &Value) -> State {
        let mut state = State::default();
        if let Some(index) = case["preceding_header"].as_u64() {
            let previous = &manifest["vectors"][index as usize];
            assert!(previous["header_present"].as_bool().unwrap());
            let raw = bytes(previous);
            let mut reader = BitReader::new(raw);
            reader.skip(4).unwrap();
            let (slots, count) = arguments(spec);
            let first = state
                .read(
                    &mut reader,
                    raw.len() * 8,
                    !previous["crc"].is_null(),
                    48_000,
                    slots,
                    count,
                )
                .unwrap();
            assert!(first.header_reset && first.format_reset);
        }
        state
    }
    fn shifted(raw: &[u8], offset: usize) -> Vec<u8> {
        let mut bytes = vec![255; (offset + raw.len() * 8 + 16).div_ceil(8)];
        for i in 0..raw.len() * 8 {
            if raw[i / 8] & (1 << (7 - i % 8)) == 0 {
                let p = offset + i;
                bytes[p / 8] &= !(1 << (7 - p % 8));
            }
        }
        bytes
    }

    #[test]
    fn original_extension_crc_and_header_reuse_vectors_are_transactional() {
        let (manifest, data_manifest) = manifests();
        assert_eq!(manifest["vectors"].as_array().unwrap().len(), 280);
        for (index, case) in manifest["vectors"].as_array().unwrap().iter().enumerate() {
            let spec = &data_manifest["vectors"][case["source_index"].as_u64().unwrap() as usize];
            let header = expected_header(spec);
            let (slots, count) = arguments(spec);
            let data_len = spec["bit_length"].as_u64().unwrap() as usize;
            let source =
                &DATA_BINARY[spec["offset"].as_u64().unwrap() as usize..][..data_len.div_ceil(8)];
            let expected = Data::read(
                &mut BitReader::new(source),
                data_len,
                &header,
                48_000,
                slots,
                count,
            )
            .unwrap();
            let checkpoint = initial_state(case, &manifest, spec);
            for offset in 0..8 {
                let raw = shifted(bytes(case), offset);
                let mut reader = BitReader::new(&raw);
                reader.skip(offset + 4).unwrap();
                let mut state = checkpoint.clone();
                let end = offset + bytes(case).len() * 8;
                for cut in offset + 4..end {
                    assert!(
                        state
                            .read(
                                &mut reader,
                                cut,
                                !case["crc"].is_null(),
                                48_000,
                                slots,
                                count
                            )
                            .is_err(),
                        "case {index}, cut {cut}"
                    );
                    assert_eq!(reader.position(), offset + 4);
                    assert_eq!(state, checkpoint);
                }
                let frame = state
                    .read(
                        &mut reader,
                        end,
                        !case["crc"].is_null(),
                        48_000,
                        slots,
                        count,
                    )
                    .unwrap_or_else(|e| panic!("case {index}: {e}"));
                assert_eq!(frame.header, header);
                assert_eq!(frame.data, expected);
                assert_eq!(frame.crc, case["crc"].as_u64().map(|v| v as u16));
                assert_eq!(frame.consumed_bytes, bytes(case).len());
                assert_eq!(
                    frame.header_reset,
                    case["header_present"].as_bool().unwrap()
                );
                assert_eq!(
                    frame.format_reset,
                    case["header_present"].as_bool().unwrap()
                );
                assert_eq!(reader.position(), end);
                assert_eq!(reader.read(8).unwrap(), 255);
                // A subsequent extension is outside both consumed size and CRC.
                let mut reader = BitReader::new(&raw);
                reader.skip(offset + 4).unwrap();
                let mut following = checkpoint.clone();
                assert_eq!(
                    following
                        .read(
                            &mut reader,
                            end + 8,
                            !case["crc"].is_null(),
                            48_000,
                            slots,
                            count
                        )
                        .unwrap(),
                    frame
                );
                assert_eq!(reader.position(), end);
                assert_eq!(reader.read(8).unwrap(), 255);
            }
        }
    }

    #[test]
    fn checksum_and_fill_corruption_do_not_commit_state() {
        let (manifest, data_manifest) = manifests();
        let mut checked_fill = 0;
        for case in manifest["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| !c["crc"].is_null())
        {
            let spec = &data_manifest["vectors"][case["source_index"].as_u64().unwrap() as usize];
            let (slots, count) = arguments(spec);
            let checkpoint = initial_state(case, &manifest, spec);
            let mut mutations = vec![4]; // first transmitted CRC bit
            if case["fill_count"].as_u64().unwrap() != 0 {
                mutations.push(bytes(case).len() * 8 - 1);
                checked_fill += 1;
            }
            for bit in mutations {
                let mut raw = bytes(case).to_vec();
                raw[bit / 8] ^= 1 << (7 - bit % 8);
                let mut reader = BitReader::new(&raw);
                reader.skip(4).unwrap();
                let mut state = checkpoint.clone();
                let error = state
                    .read(&mut reader, raw.len() * 8, true, 48_000, slots, count)
                    .unwrap_err();
                assert!(error.to_string().contains("SBR CRC mismatch"), "{error}");
                assert_eq!(state, checkpoint);
                assert_eq!(reader.position(), 4);
            }
        }
        assert!(checked_fill > 100);
    }

    #[test]
    fn initialization_replay_seek_and_format_reset() {
        let (manifest, data_manifest) = manifests();
        let case = &manifest["vectors"][0];
        let spec = &data_manifest["vectors"][case["source_index"].as_u64().unwrap() as usize];
        let (slots, count) = arguments(spec);
        let mut state = State::default();
        let raw = bytes(case);
        let mut reader = BitReader::new(raw);
        reader.skip(4).unwrap();
        let first = state
            .read(&mut reader, raw.len() * 8, false, 48_000, slots, count)
            .unwrap();
        assert!(first.header_reset && first.format_reset);
        let checkpoint = state.clone();
        for _ in 0..2 {
            let mut reader = BitReader::new(raw);
            reader.skip(4).unwrap();
            let frame = state
                .read(&mut reader, raw.len() * 8, false, 48_000, slots, count)
                .unwrap();
            assert!(!frame.header_reset && !frame.format_reset);
            assert_eq!(state, checkpoint);
        }
        let mut reader = BitReader::new(raw);
        reader.skip(4).unwrap();
        let changed = state
            .read(&mut reader, raw.len() * 8, false, 64_000, 16, count)
            .unwrap();
        assert!(!changed.header_reset && changed.format_reset);
        state.reset();
        assert_eq!(state, State::default());
        let raw = bytes(&manifest["vectors"][1]);
        let mut reader = BitReader::new(raw);
        reader.skip(4).unwrap();
        assert!(
            state
                .read(&mut reader, raw.len() * 8, false, 48_000, slots, count)
                .unwrap_err()
                .to_string()
                .contains("missing initial SBR header")
        );
        assert_eq!(reader.position(), 4);
        assert_eq!(state, State::default());
    }
}
