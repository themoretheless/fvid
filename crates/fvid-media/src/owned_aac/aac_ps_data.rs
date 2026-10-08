//! Owned parametric stereo syntax (GOST R 53556.8-2013 tables 9-14).
//! Parses all PS modes and quantized deltas. Hybrid filtering, reconstruction
//! and stereo PCM synthesis are separate; syntax acceptance is not playback.
use super::{Result, aac_ps_huffman::Book, bits::BitReader, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IidMode(u8);
impl IidMode {
    pub fn new(value: u8) -> Result<Self> {
        if value < 6 {
            Ok(Self(value))
        } else {
            Err(invalid("reserved PS IID mode"))
        }
    }
    pub fn value(self) -> u8 {
        self.0
    }
    pub fn bands(self) -> usize {
        [10, 20, 34][usize::from(self.0 % 3)]
    }
    pub fn phase_bands(self) -> usize {
        [5, 11, 17][usize::from(self.0 % 3)]
    }
    pub fn fine(self) -> bool {
        self.0 >= 3
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IccMode(u8);
impl IccMode {
    pub fn new(value: u8) -> Result<Self> {
        if value < 6 {
            Ok(Self(value))
        } else {
            Err(invalid("reserved PS ICC mode"))
        }
    }
    pub fn value(self) -> u8 {
        self.0
    }
    pub fn bands(self) -> usize {
        [10, 20, 34][usize::from(self.0 % 3)]
    }
    pub fn mixing_b(self) -> bool {
        self.0 >= 3
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub iid: Option<IidMode>,
    pub icc: Option<IccMode>,
    pub extension: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeltaRow {
    pub temporal: bool,
    pub values: Vec<i16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    pub enabled: bool,
    pub ipd: Vec<DeltaRow>,
    pub opd: Vec<DeltaRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub header_present: bool,
    pub header: Header,
    pub variable_borders: bool,
    /// QMF last-slot indices for transmitted envelopes. Empty means reuse the
    /// previous parameters. HE-AAC uses 30 or 32 slots; SSC uses 24 slots.
    pub borders: Vec<u8>,
    pub iid: Vec<DeltaRow>,
    pub icc: Vec<DeltaRow>,
    /// ID 0 extensions in transmitted order. Absence transmits no new phase
    /// parameters; the reconstruction stage applies the no-envelope rules.
    pub phase: Vec<Phase>,
    /// Reserved identifiers have no payload in the current PS syntax.
    pub reserved_extensions: Vec<u8>,
    pub consumed_bits: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    header: Header,
    // Disabling a tool does not erase the last transmitted mode field.
    last_iid_mode: u8,
    last_icc_mode: u8,
}

fn field(bits: &mut BitReader<'_>, end: usize, width: u8) -> Result<u8> {
    if end < bits.position()
        || end - bits.position() > bits.remaining()
        || usize::from(width) > end - bits.position()
    {
        return Err(invalid("truncated or invalid PS payload boundary"));
    }
    Ok(bits.read(width)? as u8)
}

fn sized_end(bits: &mut BitReader<'_>, end: usize) -> Result<usize> {
    let mut bytes = usize::from(field(bits, end, 4)?);
    if bytes == 15 {
        bytes += usize::from(field(bits, end, 8)?);
    }
    if bytes * 8 > end.saturating_sub(bits.position()) {
        return Err(invalid("truncated PS extension area"));
    }
    Ok(bits.position() + bytes * 8)
}

fn row(
    bits: &mut BitReader<'_>,
    end: usize,
    bands: usize,
    changed: bool,
    frequency: Book,
    time: Book,
) -> Result<DeltaRow> {
    let temporal = field(bits, end, 1)? != 0;
    if temporal && changed {
        return Err(invalid(
            "PS mode change requires frequency-coded first envelope",
        ));
    }
    let book = if temporal { time } else { frequency };
    let mut values = Vec::with_capacity(bands);
    for _ in 0..bands {
        values.push(book.decode(bits, end)?);
    }
    Ok(DeltaRow { temporal, values })
}

impl State {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Read one unaligned ps_data() within an absolute enclosing bit boundary.
    /// Reader and retained header commit together only on complete success.
    pub fn read(&mut self, bits: &mut BitReader<'_>, end: usize, slots: u8) -> Result<Frame> {
        if !matches!(slots, 24 | 30 | 32) {
            return Err(invalid("invalid PS QMF slot count"));
        }
        let mut trial = self.clone();
        let mut reader = bits.clone();
        let frame = trial.read_inner(&mut reader, end, slots)?;
        *self = trial;
        *bits = reader;
        Ok(frame)
    }

    fn read_inner(&mut self, bits: &mut BitReader<'_>, end: usize, slots: u8) -> Result<Frame> {
        let start = bits.position();
        let header_present = field(bits, end, 1)? != 0;
        let old_iid = self.last_iid_mode;
        let old_icc = self.last_icc_mode;
        if header_present {
            self.header.iid = if field(bits, end, 1)? != 0 {
                let mode = IidMode::new(field(bits, end, 3)?)?;
                self.last_iid_mode = mode.value();
                Some(mode)
            } else {
                None
            };
            self.header.icc = if field(bits, end, 1)? != 0 {
                let mode = IccMode::new(field(bits, end, 3)?)?;
                self.last_icc_mode = mode.value();
                Some(mode)
            } else {
                None
            };
            self.header.extension = field(bits, end, 1)? != 0;
        }
        let variable_borders = field(bits, end, 1)? != 0;
        let count_index = usize::from(field(bits, end, 2)?);
        let count = if variable_borders {
            [1, 2, 3, 4][count_index]
        } else {
            [0, 1, 2, 4][count_index]
        };
        let mut borders = Vec::with_capacity(count);
        for envelope in 0..count {
            let border = if variable_borders {
                field(bits, end, 5)?
            } else {
                ((envelope + 1) * usize::from(slots) / count - 1) as u8
            };
            if border >= slots || borders.last().is_some_and(|&previous| previous >= border) {
                return Err(invalid("invalid PS envelope borders"));
            }
            borders.push(border);
        }
        let mut iid = Vec::with_capacity(count);
        if let Some(mode) = self.header.iid {
            let (frequency, time) = if mode.fine() {
                (Book::IidFineFrequency, Book::IidFineTime)
            } else {
                (Book::IidCoarseFrequency, Book::IidCoarseTime)
            };
            for envelope in 0..count {
                iid.push(row(
                    bits,
                    end,
                    mode.bands(),
                    envelope == 0 && old_iid != mode.value(),
                    frequency,
                    time,
                )?);
            }
        }
        let mut icc = Vec::with_capacity(count);
        if let Some(mode) = self.header.icc {
            for envelope in 0..count {
                icc.push(row(
                    bits,
                    end,
                    mode.bands(),
                    envelope == 0 && old_icc != mode.value(),
                    Book::IccFrequency,
                    Book::IccTime,
                )?);
            }
        }
        let mut phase = Vec::new();
        let mut reserved_extensions = Vec::new();
        if self.header.extension {
            let extension_end = sized_end(bits, end)?;
            while extension_end - bits.position() > 7 {
                let id = field(bits, extension_end, 2)?;
                if id != 0 {
                    // ps_extension() has only an id==0 branch; reserved IDs
                    // consume the identifier itself and no additional fields.
                    reserved_extensions.push(id);
                    continue;
                }
                let enabled = field(bits, extension_end, 1)? != 0;
                let mut ipd = Vec::new();
                let mut opd = Vec::new();
                if enabled {
                    let mode = self
                        .header
                        .iid
                        .ok_or_else(|| invalid("PS phase parameters require IID"))?;
                    for envelope in 0..count {
                        let changed = envelope == 0 && old_iid != mode.value();
                        ipd.push(row(
                            bits,
                            extension_end,
                            mode.phase_bands(),
                            changed,
                            Book::IpdFrequency,
                            Book::IpdTime,
                        )?);
                        opd.push(row(
                            bits,
                            extension_end,
                            mode.phase_bands(),
                            changed,
                            Book::OpdFrequency,
                            Book::OpdTime,
                        )?);
                    }
                }
                if field(bits, extension_end, 1)? != 0 {
                    return Err(invalid("nonzero reserved PS bit"));
                }
                phase.push(Phase { enabled, ipd, opd });
            }
            bits.skip(extension_end - bits.position())?;
        }
        Ok(Frame {
            header_present,
            header: self.header.clone(),
            variable_borders,
            borders,
            iid,
            icc,
            phase,
            reserved_extensions,
            consumed_bits: bits.position() - start,
        })
    }

    /// Interpret the saved SBR extended_data octets. ID 2 carries PS. As
    /// specified by appendix A, any other SBR ID consumes the remaining fill
    /// area. A failed later PS block rolls back earlier headers in this area.
    pub fn read_sbr_extensions(&mut self, data: &[u8], slots: u8) -> Result<Vec<Frame>> {
        if data.len() > 270 || !matches!(slots, 24 | 30 | 32) {
            return Err(invalid("invalid SBR PS extension dimensions"));
        }
        let mut trial = self.clone();
        let mut bits = BitReader::new(data);
        let end = data.len() * 8;
        let mut frames = Vec::new();
        while bits.remaining() > 7 {
            let id = field(&mut bits, end, 2)?;
            if id != 2 {
                break;
            }
            frames.push(trial.read_inner(&mut bits, end, slots)?);
        }
        *self = trial;
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packed(text: &str) -> Vec<u8> {
        let mut bytes = vec![0; text.len().div_ceil(8)];
        for (index, b) in text.bytes().enumerate() {
            if b == b'1' {
                bytes[index / 8] |= 1 << (7 - index % 8);
            }
        }
        bytes
    }

    fn assert_rows(rows: &[DeltaRow], expected: &serde_json::Value) {
        assert_eq!(rows.len(), expected.as_array().unwrap().len());
        for (row, oracle) in rows.iter().zip(expected.as_array().unwrap()) {
            assert_eq!(row.temporal, oracle["temporal"].as_bool().unwrap());
            let values = oracle["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i16)
                .collect::<Vec<_>>();
            assert_eq!(row.values, values);
        }
    }

    #[test]
    fn original_modes_phase_envelopes_retained_headers_and_every_truncation() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ps-syntax.json"
        ))
        .unwrap();
        let binary = include_bytes!("../../../../tests/fixtures/playback-errors/aac-ps-syntax.bin");
        for sequence in manifest["sequences"].as_array().unwrap() {
            let mut state = State::default();
            let slots = sequence["slots"].as_u64().unwrap() as u8;
            for reference in sequence["frames"].as_array().unwrap() {
                let offset = reference["offset"].as_u64().unwrap() as usize;
                let length = reference["bytes"].as_u64().unwrap() as usize;
                let start = reference["start"].as_u64().unwrap() as usize;
                let end = start + reference["bits"].as_u64().unwrap() as usize;
                let mut bits = BitReader::new(&binary[offset..offset + length]);
                bits.skip(start).unwrap();
                let before = state.clone();
                for truncated_end in start..end {
                    assert!(
                        state.read(&mut bits, truncated_end, slots).is_err(),
                        "{sequence}: {truncated_end}"
                    );
                    assert_eq!(bits.position(), start);
                    assert_eq!(state, before);
                }
                let frame = state.read(&mut bits, end, slots).unwrap();
                let expected = &reference["expected"];
                assert_eq!(bits.position(), end);
                assert_eq!(bits.read(8).unwrap(), 0xa5);
                assert_eq!(frame.consumed_bits, end - start);
                assert_eq!(
                    frame.header_present,
                    expected["header_present"].as_bool().unwrap()
                );
                assert_eq!(
                    frame.header.iid.map(|x| u64::from(x.value())),
                    expected["iid_mode"].as_u64()
                );
                assert_eq!(
                    frame.header.icc.map(|x| u64::from(x.value())),
                    expected["icc_mode"].as_u64()
                );
                assert_eq!(
                    frame.header.extension,
                    expected["extension"].as_bool().unwrap()
                );
                assert_eq!(
                    frame.variable_borders,
                    expected["variable_borders"].as_bool().unwrap()
                );
                assert_eq!(
                    frame.borders,
                    expected["borders"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_u64().unwrap() as u8)
                        .collect::<Vec<_>>()
                );
                assert_rows(&frame.iid, &expected["iid"]);
                assert_rows(&frame.icc, &expected["icc"]);
                assert_eq!(
                    frame.phase.len(),
                    expected["phase"].as_array().unwrap().len()
                );
                for (phase, oracle) in frame
                    .phase
                    .iter()
                    .zip(expected["phase"].as_array().unwrap())
                {
                    assert_eq!(phase.enabled, oracle["enabled"].as_bool().unwrap());
                    assert_rows(&phase.ipd, &oracle["ipd"]);
                    assert_rows(&phase.opd, &oracle["opd"]);
                }
                assert!(frame.reserved_extensions.iter().all(|&v| v == 3));
            }
            state.reset();
            assert_eq!(state, State::default());
        }
    }

    #[test]
    fn malformed_modes_borders_phase_and_nested_sizes_preserve_input_and_header() {
        let cases = [
            ("11110".to_owned(), "reserved PS IID mode"),
            ("101110".to_owned(), "reserved PS ICC mode"),
            (
                "1100100".to_owned() + "0011",
                "PS mode change requires frequency-coded first envelope",
            ),
            (
                "1010010".to_owned() + "0011",
                "PS mode change requires frequency-coded first envelope",
            ),
        ];
        // The remaining cases carry complete headers and exact local areas;
        // failure must be this semantic rule, rather than unrelated truncation.
        for (text, expected) in cases {
            let bytes = packed(&text);
            let mut bits = BitReader::new(&bytes);
            let mut state = State::default();
            let before = state.clone();
            assert_eq!(
                state.read(&mut bits, text.len(), 32).unwrap_err().0,
                expected
            );
            assert_eq!(bits.position(), 0);
            assert_eq!(state, before);
        }
        for (text, expected) in [
            ("01011111000000".to_owned(), "invalid PS envelope borders"),
            (
                "10010010001".to_owned() + "00100000",
                "PS phase parameters require IID",
            ),
            (
                "10010010001".to_owned() + "00010000",
                "nonzero reserved PS bit",
            ),
            (
                "10010010010".to_owned() + "00000000",
                "truncated PS extension area",
            ),
            (
                "1001001111100000000".to_owned(),
                "truncated PS extension area",
            ),
        ] {
            let bytes = packed(&text);
            let mut bits = BitReader::new(&bytes);
            let mut state = State::default();
            let before = state.clone();
            assert_eq!(
                state.read(&mut bits, text.len(), 30).unwrap_err().0,
                expected,
                "{text}"
            );
            assert_eq!(bits.position(), 0);
            assert_eq!(state, before);
        }
    }

    #[test]
    fn maximum_extension_reserved_ids_and_outer_sbr_fill_are_bounded() {
        let text = "1001001111111111111".to_owned() + &"1".repeat(270 * 8);
        let bytes = packed(&text);
        let mut state = State::default();
        let mut bits = BitReader::new(&bytes);
        let frame = state.read(&mut bits, text.len(), 32).unwrap();
        assert_eq!(frame.reserved_extensions.len(), (270 * 8 - 6) / 2);
        assert_eq!(frame.phase.len(), 0);
        assert_eq!(bits.position(), text.len());
        assert!(state.read_sbr_extensions(&[0; 271], 32).is_err());
        assert!(state.read_sbr_extensions(&[0; 270], 31).is_err());
        for prefix in ["00", "01", "11"] {
            let bytes = packed(&(prefix.to_owned() + "11111111111111"));
            assert!(state.read_sbr_extensions(&bytes, 32).unwrap().is_empty());
        }
        // ps_data without a header, no new envelopes. Two outer PS elements
        // fit into the unaligned octet stream; the final two bits are fill.
        let mut state = State::default();
        let frames = state
            .read_sbr_extensions(&packed("10000010000011"), 32)
            .unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|f| f.borders.is_empty()));
        // A valid first PS header followed by an invalid second PS header may
        // not partially update this area. First mode=5; second reserved mode=6.
        let before = state.clone();
        let two = packed("10111010000010111100000000");
        assert_eq!(
            state.read_sbr_extensions(&two, 32).unwrap_err().0,
            "reserved PS IID mode"
        );
        assert_eq!(state, before);
        let empty_area = packed("10010010000");
        let mut bits = BitReader::new(&empty_area);
        let frame = state.read(&mut bits, 11, 32).unwrap();
        assert!(frame.header.extension && frame.phase.is_empty());
    }

    #[test]
    fn both_phase_time_flags_require_frequency_rows_after_iid_mode_change() {
        // Header: IID mode 1, ICC disabled, extensions enabled; one envelope.
        // IID frequency row has twenty zero deltas (coarse book word "0").
        let base = "11001010010".to_owned() + &"0".repeat(20);
        for inner in [
            "0011".to_owned(),                         // enabled IPD, then ipd_dt=1
            "0010".to_owned() + &"1".repeat(11) + "1", // ipd_dt=0, eleven zero words, opd_dt=1
        ] {
            let count = inner.len().div_ceil(8);
            let text = base.clone()
                + &format!("{count:04b}")
                + &inner
                + &"0".repeat(count * 8 - inner.len());
            let bytes = packed(&text);
            let mut bits = BitReader::new(&bytes);
            let mut state = State::default();
            assert_eq!(
                state.read(&mut bits, text.len(), 32).unwrap_err().0,
                "PS mode change requires frequency-coded first envelope"
            );
            assert_eq!(bits.position(), 0);
            assert_eq!(state, State::default());
        }
    }
}
