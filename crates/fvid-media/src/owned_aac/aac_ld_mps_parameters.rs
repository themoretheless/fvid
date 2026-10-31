//! Stateful LD-MPS parameter indices, before energy quantization/dequantization.
//! One instance owns one CLD/ICC/CPC band region. It is cloneable for decoder
//! checkpoints; this is not a spatial frame parser or a PCM renderer.
use super::{
    Result,
    aac_ld_mps_entropy::{self as entropy, Kind, Options},
    bits::BitReader,
    invalid,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct History {
    indices: Vec<i16>,
    coarse: bool,
}

/// Parse history and mapped history are separate: pair decoding advances to
/// the second vector before the first vector is mapped for output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameters {
    kind: Kind,
    default: i16,
    parsed: History,
    mapped: History,
}

fn bounds(kind: Kind, coarse: bool) -> (i16, i16) {
    match (kind, coarse) {
        (Kind::Cld, false) => (-15, 15),
        (Kind::Cld, true) => (-7, 7),
        (Kind::Icc, false) => (0, 7),
        (Kind::Icc, true) => (0, 3),
        (Kind::Cpc, false) => (-20, 30),
        (Kind::Cpc, true) => (-10, 15),
    }
}
fn convert(value: i16, kind: Kind, to_coarse: bool) -> i16 {
    if to_coarse {
        if kind == Kind::Cld {
            value / 2
        } else {
            value >> 1
        }
    } else {
        match (kind, value) {
            (Kind::Cld, -7) => -15,
            (Kind::Cld, 7) => 15,
            _ => value * 2,
        }
    }
}
impl History {
    fn requantize(&mut self, kind: Kind, coarse: bool) {
        if self.coarse != coarse {
            for v in &mut self.indices {
                *v = convert(*v, kind, coarse);
            }
            self.coarse = coarse;
        }
    }
    fn fine(&self, kind: Kind) -> Vec<i16> {
        self.indices
            .iter()
            .map(|&v| {
                if self.coarse {
                    convert(v, kind, false)
                } else {
                    v
                }
            })
            .collect()
    }
}

/// Group widths differ by at most one. Short groups precede long groups;
/// notably a five-band region at stride two maps to widths [1, 2, 2].
fn groups(bands: usize, stride: usize) -> Vec<std::ops::Range<usize>> {
    if bands == 0 {
        return Vec::new();
    }
    let count = bands.div_ceil(stride);
    let deficit = count * stride - bands;
    let mut start = 0;
    (0..count)
        .map(|i| {
            let width = stride - deficit / count - usize::from(i < deficit % count);
            let range = start..start + width;
            start += width;
            range
        })
        .collect()
}
// Entropy history uses the stride boundaries, not the balanced output map.
// Each interior boundary moves left once per preceding shortened group; any
// remaining shortage is absorbed at the final boundary.
fn history_groups(bands: usize, stride: usize) -> Vec<std::ops::Range<usize>> {
    if bands == 0 {
        return Vec::new();
    }
    let count = bands.div_ceil(stride);
    let deficit = count * stride - bands;
    (0..count)
        .map(|i| {
            let start = i * stride - deficit.min(i);
            let end = if i + 1 == count {
                bands
            } else {
                (i + 1) * stride - deficit.min(i + 1)
            };
            start..end
        })
        .collect()
}
fn expand(values: &[i16], groups: &[std::ops::Range<usize>]) -> Vec<i16> {
    values
        .iter()
        .zip(groups)
        .flat_map(|(&value, group)| std::iter::repeat_n(value, group.len()))
        .collect()
}

/// Mapped fine-domain values plus the original quantization domain of each set.
/// Residual ICC correction is applied in that domain, without changing history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub values: Vec<Vec<i16>>,
    pub coarse: Vec<bool>,
}
impl Parameters {
    /// The band region is fixed for this instance. Zero-width regions still
    /// carry data modes. Unit-stride empty PCM consumes only its flag; grouped
    /// empty regions consume one discarded datum. Unit-stride empty Huffman
    /// remains invalid. No discarded symbol enters the visible band history.
    pub fn new(kind: Kind, bands: usize, default: i16) -> Result<Self> {
        let (low, high) = bounds(kind, false);
        if bands > 23 || !(low..=high).contains(&default) {
            return Err(invalid("invalid LD-MPS parameter region or default"));
        }
        let history = History {
            indices: vec![default; bands],
            coarse: false,
        };
        Ok(Self {
            kind,
            default,
            parsed: history.clone(),
            mapped: history,
        })
    }
    pub fn reset(&mut self) {
        self.parsed.indices.fill(self.default);
        self.parsed.coarse = false;
        self.mapped = self.parsed.clone();
    }
    /// Decode one ecData block: all two-bit data modes precede coded sets.
    /// Slots are zero-based and strictly increasing within a 1..32-slot frame.
    /// Returns fine indices; energy quantization, dequantization, smoothing and
    /// matrix interpolation belong to subsequent stages. Both state and caller
    /// cursor commit only after the entire block succeeds.
    pub fn decode(
        &mut self,
        bits: &mut BitReader<'_>,
        slots: &[u8],
        frame_slots: u8,
        independent: bool,
    ) -> Result<Vec<Vec<i16>>> {
        Ok(self
            .decode_with_quantization(bits, slots, frame_slots, independent)?
            .values)
    }
    pub fn decode_with_quantization(
        &mut self,
        bits: &mut BitReader<'_>,
        slots: &[u8],
        frame_slots: u8,
        independent: bool,
    ) -> Result<Decoded> {
        if !(1..=32).contains(&frame_slots)
            || !(1..=2).contains(&slots.len())
            || slots.iter().any(|&slot| slot >= frame_slots)
            || slots.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(invalid("invalid LD-MPS parameter-set slots"));
        }
        let mut b = bits.clone();
        let mut next = self.clone();
        let rows = next.decode_inner(&mut b, slots, independent)?;
        *bits = b;
        *self = next;
        Ok(rows)
    }
    fn decode_inner(
        &mut self,
        b: &mut BitReader<'_>,
        slots: &[u8],
        independent: bool,
    ) -> Result<Decoded> {
        let modes = (0..slots.len())
            .map(|_| b.read(2))
            .collect::<Result<Vec<_>>>()?;
        if independent && matches!(modes[0], 1 | 2) {
            return Err(invalid(
                "independent LD-MPS set requires default or coded data",
            ));
        }
        if modes.last() == Some(&2) {
            return Err(invalid("LD-MPS interpolation lacks a following anchor"));
        }
        let mut coded: Vec<Option<History>> = vec![None; slots.len()];
        for i in 0..slots.len() {
            if modes[i] == 0 {
                self.parsed.indices.fill(self.default);
                self.parsed.coarse = false;
            } else if modes[i] == 3 && coded[i].is_none() {
                let paired = b.bit()?;
                let coarse = b.bit()?;
                let stride = [1, 2, 5, 23][b.read(2)? as usize];
                if paired && (i + 1 >= slots.len() || modes[i + 1] != 3) {
                    return Err(invalid("LD-MPS data pair lacks a second coded set"));
                }
                self.parsed.requantize(self.kind, coarse);
                let map = history_groups(self.parsed.indices.len(), stride);
                let output_map = groups(self.parsed.indices.len(), stride);
                let options = Options {
                    kind: self.kind,
                    coarse,
                    paired,
                    independent,
                    set_index: i as u8,
                };
                // Signed C stride division yields one datum for a grouped
                // empty region. Its [start,start) expansion writes no bands.
                // The numeric result can depend on another region's history,
                // so consume the syntax without materializing an unused index.
                let pair = if self.parsed.indices.is_empty() && stride != 1 {
                    entropy::discard_empty_group(b, options)?
                } else {
                    let history = map
                        .iter()
                        .map(|r| self.parsed.indices[r.start])
                        .collect::<Vec<_>>();
                    entropy::decode(b, &history, options)?
                };
                let first = History {
                    indices: expand(&pair.first, &output_map),
                    coarse,
                };
                self.parsed = History {
                    indices: expand(&pair.first, &map),
                    coarse,
                };
                coded[i] = Some(first);
                if let Some(second) = pair.second {
                    self.parsed = History {
                        indices: expand(&second, &map),
                        coarse,
                    };
                    let second = History {
                        indices: expand(&second, &output_map),
                        coarse,
                    };
                    coded[i + 1] = Some(second);
                }
            }
        }
        let mut rows = Vec::with_capacity(slots.len());
        let mut coarse = Vec::with_capacity(slots.len());
        for (i, &mode) in modes.iter().enumerate() {
            match mode {
                0 => {
                    self.mapped.indices.fill(self.default);
                    self.mapped.coarse = false;
                }
                3 => {
                    self.mapped = coded[i]
                        .take()
                        .ok_or_else(|| invalid("missing LD-MPS coded set"))?
                }
                _ => {}
            }
            rows.push(self.mapped.fine(self.kind));
            coarse.push(self.mapped.coarse);
        }
        // LD has at most two sets. The only possible interpolated set is the
        // first; its left anchor is its retained row at the same slot, so the
        // interpolation fraction is zero. A final interpolated row was refused
        // above. Do not invent a virtual previous-frame time slot here.
        Ok(Decoded {
            values: rows,
            coarse,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_sequences_preserve_entropy_history_distinct_from_mapped_output() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/aac-ld-mps-parameters");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
        let packets = std::fs::read(root.join("packets.bin")).unwrap();
        let mut distinct = 0;
        for c in manifest["cases"].as_array().unwrap() {
            let kind = match c["kind"].as_str().unwrap() {
                "CLD" => Kind::Cld,
                "ICC" => Kind::Icc,
                "CPC" => Kind::Cpc,
                _ => unreachable!(),
            };
            let mut state = Parameters::new(
                kind,
                c["bands"].as_u64().unwrap() as usize,
                c["default"].as_i64().unwrap() as i16,
            )
            .unwrap();
            for f in c["frames"].as_array().unwrap() {
                let offset = f["offset"].as_u64().unwrap() as usize;
                let length = f["bytes"].as_u64().unwrap() as usize;
                let mut bits = BitReader::with_bit_len(
                    &packets[offset..offset + length],
                    f["bits"].as_u64().unwrap() as usize,
                )
                .unwrap();
                let slots = f["slots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect::<Vec<_>>();
                state
                    .decode(&mut bits, &slots, 16, f["independent"].as_bool().unwrap())
                    .unwrap();
                let expected = f["parsed"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_i64().unwrap() as i16)
                    .collect::<Vec<_>>();
                assert_eq!(
                    state.parsed.indices, expected,
                    "{} {:?}",
                    c["name"], f["modes"]
                );
                assert_eq!(state.parsed.coarse, f["parse_coarse"].as_bool().unwrap());
                distinct += usize::from(state.parsed != state.mapped);
            }
        }
        assert!(
            distinct > 0,
            "corpus must expose distinct parse/output maps"
        );
    }
}

impl Parameters {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        f.vector(&self.parsed.indices)?;
        f.vector(&self.mapped.indices)?;
        Ok(())
    }
}
