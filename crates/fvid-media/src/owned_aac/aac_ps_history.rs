//! Owned native-band PS delta reconstruction, history and dequantization.
//! Global parameter-band mapping, hybrid filters and complex mixing are later
//! stages. In particular a zero-envelope frame preserves the previous mixing
//! parameters; it must not reinterpret them under a newly transmitted mode.
use super::{
    Result,
    aac_ps_data::{self, DeltaRow, Frame, Header, IccMode, IidMode},
    aac_ps_dequant,
    bits::BitReader,
    invalid,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indices {
    pub iid_mode: IidMode,
    pub icc_mode: IccMode,
    pub phase_mode: IidMode,
    pub iid_enabled: bool,
    pub icc_enabled: bool,
    pub phase_enabled: bool,
    pub iid: Vec<i16>,
    pub icc: Vec<i16>,
    pub ipd: Vec<i16>,
    pub opd: Vec<i16>,
}

impl Default for Indices {
    fn default() -> Self {
        Self {
            iid_mode: IidMode::default(),
            icc_mode: IccMode::default(),
            phase_mode: IidMode::default(),
            iid_enabled: false,
            icc_enabled: false,
            phase_enabled: false,
            iid: vec![0; 10],
            icc: vec![0; 10],
            ipd: vec![0; 5],
            opd: vec![0; 5],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Levels {
    pub iid_db: Vec<f64>,
    pub coherence: Vec<f64>,
    pub ipd_radians: Vec<f64>,
    pub opd_radians: Vec<f64>,
}

impl Indices {
    /// Validate native-band dimensions and grids before allocating levels.
    /// Disabled tools have explicit index zero defaults, not stale parameters.
    pub fn validate(&self) -> Result<()> {
        if self.iid.len() != self.iid_mode.bands()
            || self.icc.len() != self.icc_mode.bands()
            || self.ipd.len() != self.phase_mode.phase_bands()
            || self.opd.len() != self.ipd.len()
            || (!self.iid_enabled && self.iid.iter().any(|&v| v != 0))
            || (!self.icc_enabled && self.icc.iter().any(|&v| v != 0))
            || (self.phase_enabled && (!self.iid_enabled || self.phase_mode != self.iid_mode))
            || (!self.phase_enabled && self.ipd.iter().chain(&self.opd).any(|&v| v != 0))
        {
            return Err(invalid(
                "invalid PS native parameter dimensions or disabled defaults",
            ));
        }
        for &v in &self.iid {
            aac_ps_dequant::iid_db(self.iid_mode, v)?;
        }
        for &v in &self.icc {
            aac_ps_dequant::coherence(v)?;
        }
        for &v in self.ipd.iter().chain(&self.opd) {
            aac_ps_dequant::phase_radians(v)?;
        }
        Ok(())
    }
    pub fn dequantize(&self) -> Result<Levels> {
        self.validate()?;
        let iid_db = self
            .iid
            .iter()
            .map(|&v| aac_ps_dequant::iid_db(self.iid_mode, v))
            .collect::<Result<Vec<_>>>()?;
        let coherence = self
            .icc
            .iter()
            .map(|&v| aac_ps_dequant::coherence(v))
            .collect::<Result<Vec<_>>>()?;
        let ipd_radians = self
            .ipd
            .iter()
            .map(|&v| aac_ps_dequant::phase_radians(v))
            .collect::<Result<Vec<_>>>()?;
        let opd_radians = self
            .opd
            .iter()
            .map(|&v| aac_ps_dequant::phase_radians(v))
            .collect::<Result<Vec<_>>>()?;
        Ok(Levels {
            iid_db,
            coherence,
            ipd_radians,
            opd_radians,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameters {
    pub header: Header,
    /// Most recently transmitted mode fields, even when the tool is currently
    /// disabled or num_env is zero. The later common-band stage needs these
    /// separately from the old physical grids of retained parameters.
    pub iid_mode: IidMode,
    pub icc_mode: IccMode,
    pub phase_enabled: bool,
    pub borders: Vec<u8>,
    /// One set per *transmitted* envelope. Empty means preserve the prior
    /// end-of-frame mix (subject to current enabled/default rules), not invent
    /// a fresh envelope at a new frequency resolution or quantization step.
    pub envelopes: Vec<Indices>,
    pub retained: Indices,
    /// Stereo startup needs a header and independently coded first envelope
    /// for every enabled parameter. Until then the synthesis stage emits mono
    /// to both outputs as specified by 6.5.1.
    pub initialized: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    previous: Indices,
    header: Header,
    last_iid_mode: IidMode,
    last_icc_mode: IccMode,
    initialized: bool,
    slots: Option<u8>,
}

#[derive(Clone, Copy)]
enum Kind {
    Iid(bool),
    Icc,
    Phase,
}
impl Kind {
    fn delta_range(self) -> std::ops::RangeInclusive<i16> {
        match self {
            Self::Iid(true) => -30..=30,
            Self::Iid(false) => -14..=14,
            Self::Icc => -7..=7,
            Self::Phase => 0..=7,
        }
    }
    fn index_range(self) -> std::ops::RangeInclusive<i32> {
        match self {
            Self::Iid(true) => -15..=15,
            Self::Iid(false) => -7..=7,
            Self::Icc | Self::Phase => 0..=7,
        }
    }
}

fn validate_rows(
    rows: &[DeltaRow],
    count: usize,
    bands: usize,
    changed: bool,
    kind: Kind,
) -> Result<()> {
    if rows.len() != count
        || rows.iter().any(|r| {
            r.values.len() != bands || r.values.iter().any(|v| !kind.delta_range().contains(v))
        })
    {
        return Err(invalid("invalid PS delta row dimensions or codebook range"));
    }
    if changed && rows.first().is_some_and(|r| r.temporal) {
        return Err(invalid(
            "PS mode change requires frequency-coded first envelope",
        ));
    }
    Ok(())
}

fn recover(row: &DeltaRow, previous: &[i16], kind: Kind) -> Result<Vec<i16>> {
    let mut output = Vec::with_capacity(row.values.len());
    let mut prefix = 0i32;
    for (band, &delta) in row.values.iter().enumerate() {
        // A nonexistent preceding band/envelope has normative index zero.
        // Mode-change first rows are checked separately; do not silently map
        // time references through a later global-band interpolation stage.
        let base = if row.temporal {
            i32::from(previous.get(band).copied().unwrap_or(0))
        } else {
            prefix
        };
        let mut index = base + i32::from(delta);
        if matches!(kind, Kind::Phase) {
            index = index.rem_euclid(8);
        }
        if !kind.index_range().contains(&index) {
            return Err(invalid("PS reconstructed index exceeds quantization grid"));
        }
        output.push(index as i16);
        prefix = index;
    }
    Ok(output)
}

impl History {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Complete frame commits atomically, including all grids, modes and phase
    /// rows. Public/manual frames are validated as strictly as parsed frames.
    pub fn decode(&mut self, frame: &Frame, slots: u8) -> Result<Parameters> {
        if !matches!(slots, 24 | 30 | 32) || self.slots.is_some_and(|old| old != slots) {
            return Err(invalid(
                "PS parameter history requires valid unchanged QMF slots or reset",
            ));
        }
        let count = frame.borders.len();
        if !frame.header_present && frame.header != self.header {
            return Err(invalid(
                "PS parameters changed configuration without a header",
            ));
        }
        if count > 4
            || (frame.variable_borders && count == 0)
            || (!frame.variable_borders && count == 3)
            || frame.borders.iter().any(|&v| v >= slots)
            || frame.borders.windows(2).any(|v| v[0] >= v[1])
            || (!frame.variable_borders
                && frame
                    .borders
                    .iter()
                    .enumerate()
                    .any(|(i, &v)| usize::from(v) + 1 != (i + 1) * usize::from(slots) / count))
        {
            return Err(invalid("invalid PS parameter envelope geometry"));
        }
        if frame.phase.len() > 540
            || frame.reserved_extensions.len() > 1080
            || frame
                .reserved_extensions
                .iter()
                .any(|&v| !(1..=3).contains(&v))
            || (!frame.header.extension
                && (!frame.phase.is_empty() || !frame.reserved_extensions.is_empty()))
        {
            return Err(invalid("invalid PS parameter extension geometry"));
        }
        if let Some(mode) = frame.header.iid {
            validate_rows(
                &frame.iid,
                count,
                mode.bands(),
                self.last_iid_mode != mode,
                Kind::Iid(mode.fine()),
            )?;
        } else if !frame.iid.is_empty() {
            return Err(invalid("disabled PS IID has transmitted rows"));
        }
        if let Some(mode) = frame.header.icc {
            validate_rows(
                &frame.icc,
                count,
                mode.bands(),
                self.last_icc_mode != mode,
                Kind::Icc,
            )?;
        } else if !frame.icc.is_empty() {
            return Err(invalid("disabled PS ICC has transmitted rows"));
        }
        for phase in &frame.phase {
            if phase.enabled {
                let mode = frame
                    .header
                    .iid
                    .ok_or_else(|| invalid("PS phase parameters require IID"))?;
                let changed = self.last_iid_mode != mode;
                validate_rows(&phase.ipd, count, mode.phase_bands(), changed, Kind::Phase)?;
                validate_rows(&phase.opd, count, mode.phase_bands(), changed, Kind::Phase)?;
            } else if !phase.ipd.is_empty() || !phase.opd.is_empty() {
                return Err(invalid("disabled PS phase has transmitted rows"));
            }
        }
        // Repeated ID 0 assignments replace the same transmitted parameter
        // arrays. Only the final extension's flag/rows enter reconstruction.
        let phase = frame.phase.last().filter(|p| p.enabled);
        let phase_enabled = phase.is_some();
        let mut trial = self.clone();
        trial.slots = Some(slots);
        trial.header = frame.header.clone();
        if let Some(mode) = frame.header.iid {
            trial.last_iid_mode = mode;
        }
        if let Some(mode) = frame.header.icc {
            trial.last_icc_mode = mode;
        }
        if frame.header.iid.is_none() {
            trial.previous.iid.fill(0);
            trial.previous.iid_enabled = false;
        }
        if frame.header.icc.is_none() {
            trial.previous.icc.fill(0);
            trial.previous.icc_enabled = false;
        }
        if !phase_enabled {
            trial.previous.ipd.fill(0);
            trial.previous.opd.fill(0);
            trial.previous.phase_enabled = false;
        }
        let mut envelopes = Vec::with_capacity(count);
        for envelope in 0..count {
            let iid = if let Some(mode) = frame.header.iid {
                recover(
                    &frame.iid[envelope],
                    &trial.previous.iid,
                    Kind::Iid(mode.fine()),
                )?
            } else {
                vec![0; trial.last_iid_mode.bands()]
            };
            let icc = if frame.header.icc.is_some() {
                recover(&frame.icc[envelope], &trial.previous.icc, Kind::Icc)?
            } else {
                vec![0; trial.last_icc_mode.bands()]
            };
            let (ipd, opd) = if let Some(phase) = phase {
                (
                    recover(&phase.ipd[envelope], &trial.previous.ipd, Kind::Phase)?,
                    recover(&phase.opd[envelope], &trial.previous.opd, Kind::Phase)?,
                )
            } else {
                (
                    vec![0; trial.last_iid_mode.phase_bands()],
                    vec![0; trial.last_iid_mode.phase_bands()],
                )
            };
            let indices = Indices {
                iid_mode: trial.last_iid_mode,
                icc_mode: trial.last_icc_mode,
                phase_mode: trial.last_iid_mode,
                iid_enabled: frame.header.iid.is_some(),
                icc_enabled: frame.header.icc.is_some(),
                phase_enabled,
                iid,
                icc,
                ipd,
                opd,
            };
            trial.previous = indices.clone();
            envelopes.push(indices);
        }
        let independent = |rows: &[DeltaRow]| rows.first().is_none_or(|r| !r.temporal);
        if frame.header_present
            && count > 0
            && independent(&frame.iid)
            && independent(&frame.icc)
            && phase.is_none_or(|p| independent(&p.ipd) && independent(&p.opd))
        {
            trial.initialized = true;
        }
        let parameters = Parameters {
            header: frame.header.clone(),
            iid_mode: trial.last_iid_mode,
            icc_mode: trial.last_icc_mode,
            phase_enabled,
            borders: frame.borders.clone(),
            envelopes,
            retained: trial.previous.clone(),
            initialized: trial.initialized,
        };
        *self = trial;
        Ok(parameters)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stream {
    syntax: aac_ps_data::State,
    parameters: History,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedFrame {
    pub syntax: Frame,
    pub parameters: Parameters,
}
impl Stream {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn read(&mut self, bits: &mut BitReader<'_>, end: usize, slots: u8) -> Result<ParsedFrame> {
        let mut trial = self.clone();
        let mut reader = bits.clone();
        let syntax = trial.syntax.read(&mut reader, end, slots)?;
        let parameters = trial.parameters.decode(&syntax, slots)?;
        *self = trial;
        *bits = reader;
        Ok(ParsedFrame { syntax, parameters })
    }
    pub fn read_sbr_extensions(&mut self, data: &[u8], slots: u8) -> Result<Vec<ParsedFrame>> {
        let mut trial = self.clone();
        let frames = trial.syntax.read_sbr_extensions(data, slots)?;
        let mut output = Vec::with_capacity(frames.len());
        for syntax in frames {
            let parameters = trial.parameters.decode(&syntax, slots)?;
            output.push(ParsedFrame { syntax, parameters });
        }
        *self = trial;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    const BINARY: &[u8] =
        include_bytes!("../../../../tests/fixtures/playback-errors/aac-ps-history-syntax.bin");
    fn oracle() -> Value {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ps-history-oracles.json"
        ))
        .unwrap()
    }
    fn assert_indices(actual: &Indices, expected: &Value) {
        assert_eq!(
            u64::from(actual.iid_mode.value()),
            expected["iid_mode"].as_u64().unwrap()
        );
        assert_eq!(
            u64::from(actual.icc_mode.value()),
            expected["icc_mode"].as_u64().unwrap()
        );
        assert_eq!(
            u64::from(actual.phase_mode.value()),
            expected["phase_mode"].as_u64().unwrap()
        );
        assert_eq!(
            actual.iid_enabled,
            expected["iid_enabled"].as_bool().unwrap()
        );
        assert_eq!(
            actual.icc_enabled,
            expected["icc_enabled"].as_bool().unwrap()
        );
        assert_eq!(
            actual.phase_enabled,
            expected["phase_enabled"].as_bool().unwrap()
        );
        for (values, key) in [
            (&actual.iid, "iid"),
            (&actual.icc, "icc"),
            (&actual.ipd, "ipd"),
            (&actual.opd, "opd"),
        ] {
            assert_eq!(
                values,
                &expected[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_i64().unwrap() as i16)
                    .collect::<Vec<_>>(),
                "{key}"
            );
        }
        let levels = actual.dequantize().unwrap();
        assert!(
            levels
                .iid_db
                .iter()
                .chain(&levels.coherence)
                .chain(&levels.ipd_radians)
                .chain(&levels.opd_radians)
                .all(|v| v.is_finite())
        );
        if !actual.iid_enabled {
            assert!(levels.iid_db.iter().all(|&v| v == 0.0));
        }
        if !actual.icc_enabled {
            assert!(levels.coherence.iter().all(|&v| v == 1.0));
        }
        if !actual.phase_enabled {
            assert!(
                levels
                    .ipd_radians
                    .iter()
                    .chain(&levels.opd_radians)
                    .all(|&v| v == 0.0)
            );
        }
    }
    fn bytes(reference: &Value) -> &'static [u8] {
        let offset = reference["offset"].as_u64().unwrap() as usize;
        let length = reference["bytes"].as_u64().unwrap() as usize;
        &BINARY[offset..offset + length]
    }
    fn reader(reference: &Value) -> BitReader<'static> {
        let mut bits = BitReader::new(bytes(reference));
        bits.skip(reference["start"].as_u64().unwrap() as usize)
            .unwrap();
        bits
    }
    fn end(reference: &Value) -> usize {
        (reference["start"].as_u64().unwrap() + reference["bits"].as_u64().unwrap()) as usize
    }

    #[test]
    fn original_absolute_targets_reconstruct_across_rows_packets_modes_and_no_envelopes() {
        let oracle = oracle();
        for sequence in oracle["sequences"].as_array().unwrap() {
            let slots = sequence["slots"].as_u64().unwrap() as u8;
            let mut stream = Stream::default();
            for reference in sequence["frames"].as_array().unwrap() {
                let mut bits = reader(reference);
                let start = bits.position();
                let before = stream.clone();
                // Standalone syntax already has exhaustive bit truncations;
                // here every octet cut and the final bit checks composition.
                for truncated in (start..end(reference))
                    .step_by(8)
                    .chain(std::iter::once(end(reference) - 1))
                {
                    assert!(stream.read(&mut bits, truncated, slots).is_err());
                    assert_eq!(bits.position(), start);
                    assert_eq!(stream, before);
                }
                let frame = stream.read(&mut bits, end(reference), slots).unwrap();
                let expected = &reference["expected"];
                assert_eq!(bits.position(), end(reference));
                assert_eq!(bits.read(8).unwrap(), 0xa5);
                assert_eq!(
                    frame.parameters.initialized,
                    expected["initialized"].as_bool().unwrap()
                );
                assert_eq!(
                    frame.parameters.phase_enabled,
                    expected["phase_enabled"].as_bool().unwrap()
                );
                assert_eq!(
                    u64::from(frame.parameters.iid_mode.value()),
                    expected["iid_mode"].as_u64().unwrap()
                );
                assert_eq!(
                    u64::from(frame.parameters.icc_mode.value()),
                    expected["icc_mode"].as_u64().unwrap()
                );
                assert_eq!(
                    frame.parameters.borders,
                    expected["borders"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_u64().unwrap() as u8)
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    frame.parameters.envelopes.len(),
                    expected["envelopes"].as_array().unwrap().len()
                );
                for (actual, expected) in frame
                    .parameters
                    .envelopes
                    .iter()
                    .zip(expected["envelopes"].as_array().unwrap())
                {
                    assert_indices(actual, expected);
                }
                assert_indices(&frame.parameters.retained, &expected["retained"]);
                // Saved checkpoints are plain owned state, with no shared
                // mutable histories: replaying from a clone must be identical.
                let mut replay = before;
                let mut again = reader(reference);
                assert_eq!(
                    replay.read(&mut again, end(reference), slots).unwrap(),
                    frame
                );
                assert_eq!(replay, stream);
            }
            stream.reset();
            assert_eq!(stream, Stream::default());
        }
    }

    fn seed() -> Frame {
        let reference = oracle()["sequences"][0]["frames"][0].clone();
        let mut state = aac_ps_data::State::default();
        state
            .read(&mut reader(&reference), end(&reference), 24)
            .unwrap()
    }

    #[test]
    fn invalid_public_rows_modes_grids_and_late_numeric_failure_do_not_commit_history() {
        let first = seed();
        let mut history = History::default();
        history.decode(&first, 24).unwrap();
        let before = history.clone();
        let mut mutations = Vec::new();
        let mut bad = first.clone();
        bad.iid[1].values[0] = 30;
        mutations.push(bad);
        let mut bad = first.clone();
        bad.iid[0].values[0] = 14;
        mutations.push(bad);
        let mut bad = first.clone();
        bad.icc[1].values[0] = 7;
        bad.icc[1].values[1] = 7;
        mutations.push(bad);
        let mut bad = first.clone();
        bad.phase[0].opd[1].values[0] = -1;
        mutations.push(bad);
        let mut bad = first.clone();
        bad.phase[0].ipd[1].values.push(0);
        mutations.push(bad);
        let mut bad = first.clone();
        bad.icc.clear();
        mutations.push(bad);
        let mut bad = first.clone();
        bad.header.iid = None;
        mutations.push(bad);
        let mut bad = first.clone();
        bad.phase[0].enabled = false;
        mutations.push(bad);
        let mut bad = first.clone();
        bad.header.extension = false;
        mutations.push(bad);
        let mut bad = first.clone();
        bad.borders[1] = bad.borders[0];
        mutations.push(bad);
        let mut bad = first.clone();
        bad.borders[1] = 24;
        mutations.push(bad);
        let mut bad = first.clone();
        bad.borders[0] = 0;
        mutations.push(bad);
        let mut bad = first.clone();
        bad.header_present = false;
        bad.header.icc = Some(IccMode::new(0).unwrap());
        mutations.push(bad);
        for bad in mutations {
            assert!(history.decode(&bad, 24).is_err());
            assert_eq!(history, before);
        }
        assert!(history.decode(&first, 32).is_err());
        assert_eq!(history, before);
        history.reset();
        assert_eq!(history, History::default());
    }

    #[test]
    fn disabled_defaults_and_public_dequantization_dimensions_are_checked() {
        let mut value = Indices::default();
        let levels = value.dequantize().unwrap();
        assert_eq!(levels.iid_db, vec![0.0; 10]);
        assert_eq!(levels.coherence, vec![1.0; 10]);
        value.iid[0] = 1;
        assert!(value.dequantize().is_err());
        value.iid[0] = 0;
        value.icc[0] = 1;
        assert!(value.dequantize().is_err());
        value.icc[0] = 0;
        value.ipd[0] = 1;
        assert!(value.dequantize().is_err());
        value.ipd[0] = 0;
        value.opd.pop();
        assert!(value.dequantize().is_err());
        value.opd.push(0);
        value.phase_enabled = true;
        assert!(value.dequantize().is_err());
        value.iid_enabled = true;
        value.phase_mode = IidMode::new(1).unwrap();
        assert!(value.dequantize().is_err());
    }

    fn packed(text: &str) -> Vec<u8> {
        let mut output = vec![0; text.len().div_ceil(8)];
        for (i, b) in text.bytes().enumerate() {
            if b == b'1' {
                output[i / 8] |= 1 << (7 - i % 8);
            }
        }
        output
    }
    fn text(reference: &Value) -> String {
        let mut bits = reader(reference);
        (bits.position()..end(reference))
            .map(|_| if bits.read(1).unwrap() == 0 { '0' } else { '1' })
            .collect()
    }
    #[test]
    fn legal_huffman_words_with_invalid_indices_roll_back_composed_reader_and_all_area_headers() {
        let oracle = oracle();
        let seed = &oracle["sequences"][12]["frames"][0];
        let mut stream = Stream::default();
        stream.read(&mut reader(seed), end(seed), 32).unwrap();
        for bad in oracle["malformed"].as_array().unwrap() {
            let mut syntax = aac_ps_data::State::default();
            syntax.read(&mut reader(bad), end(bad), 32).unwrap(); // syntactically legal
            let before = stream.clone();
            let mut bits = reader(bad);
            let start = bits.position();
            assert_eq!(
                stream.read(&mut bits, end(bad), 32).unwrap_err().0,
                bad["error"].as_str().unwrap()
            );
            assert_eq!(bits.position(), start);
            assert_eq!(stream, before);
            // Successful first header must also roll back when a later PS
            // block in the same SBR extension has an invalid recovered index.
            let area = packed(&("10".to_owned() + &text(seed) + "10" + &text(bad)));
            assert_eq!(
                stream.read_sbr_extensions(&area, 32).unwrap_err().0,
                bad["error"].as_str().unwrap()
            );
            assert_eq!(stream, before);
        }
    }
}
