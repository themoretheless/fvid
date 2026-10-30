//! Owned LDSpatialSpecificConfig metadata, ISO/IEC 23003-2 Annex B.
//! Parsing is separate from spatial reconstruction: accepting this structure
//! does not authorize decoding its downmix as the spatial output.
use super::{Result, bits::BitReader, invalid};
const RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];
const BANDS: [u8; 8] = [0, 23, 15, 12, 9, 7, 5, 4];
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    pub code: u8,
    pub input_channels: u8,
    pub output_channels: u8,
    pub ott_lfe: Vec<bool>,
    pub ttt_boxes: u8,
}
impl Tree {
    pub(super) fn new(code: u8) -> Result<Self> {
        let (input, output, lfe, ttt): (_, _, &[bool], _) = match code {
            0 => (1, 6, &[false, false, false, false, true], 0),
            1 => (1, 6, &[false, false, true, false, false], 0),
            2 => (2, 6, &[true, false, false], 1),
            3 | 4 => (2, 8, &[true, false, false, false, false], 1),
            5 | 6 => (6, 8, &[false, false], 0),
            7 => (1, 2, &[false], 0),
            _ => return Err(invalid("reserved LD-MPS tree configuration")),
        };
        Ok(Self {
            code,
            input_channels: input,
            output_channels: output,
            ott_lfe: lfe.to_vec(),
            ttt_boxes: ttt,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ttt {
    pub low_mode: u8,
    pub high: Option<(u8, u8)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HrtfChannel {
    pub left: Vec<u8>,
    pub right: Vec<u8>,
    pub phase: Option<Vec<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hrtf {
    pub bands: u8,
    pub asymmetric: bool,
    pub channels: Vec<HrtfChannel>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArbitraryOtt {
    pub default_cld: bool,
    pub lfe: bool,
    pub bands: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtensionValue {
    Residual {
        rate: u32,
        frames_code: u8,
        bands: Vec<Option<u8>>,
    },
    DownmixResidual {
        rate: u32,
        frames_code: u8,
        bands: u8,
    },
    ArbitraryTree {
        nodes: Vec<Vec<bool>>,
        ott: Vec<ArbitraryOtt>,
        positions: Vec<u8>,
    },
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extension {
    pub kind: u8,
    pub payload: Vec<u8>,
    pub value: ExtensionValue,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub sample_rate: u32,
    pub frame_slots: u8,
    pub parameter_bands: u8,
    pub tree: Tree,
    pub quantization_mode: u8,
    pub one_icc: Option<bool>,
    pub arbitrary_downmix: bool,
    pub surround_gain: Option<u8>,
    pub lfe_gain: Option<u8>,
    pub downmix_gain: u8,
    pub matrix_mode: Option<bool>,
    pub temporal_shape: u8,
    pub decorrelation: u8,
    /// Only LFE OTTs carry an explicit band count; normal OTTs use all bands.
    pub ott_bands: Vec<u8>,
    pub ttt: Vec<Ttt>,
    pub hrtf: Option<Hrtf>,
    pub extensions: Vec<Extension>,
}
fn rate(index: u32) -> Result<u32> {
    RATES
        .get(index as usize)
        .copied()
        .ok_or_else(|| invalid("reserved LD-MPS frequency index"))
}
fn band_count(bits: &mut BitReader<'_>) -> Result<u8> {
    let bands = BANDS[bits.read(3)? as usize];
    if bands == 0 {
        return Err(invalid("reserved LD-MPS frequency resolution"));
    }
    Ok(bands)
}
fn limited(bits: &mut BitReader<'_>, width: u8, max: u8, message: &str) -> Result<u8> {
    let value = bits.read(width)? as u8;
    if value > max {
        return Err(invalid(message));
    }
    Ok(value)
}
fn residual_rate(bits: &mut BitReader<'_>) -> Result<u32> {
    rate(bits.read(4)?)
}
impl Config {
    /// Byte-bounded SSC prefix of an ancillary HeaderAndFrame message.
    /// Alignment delay is returned to the timeline owner, never discarded.
    /// A failed header leaves the caller cursor unchanged.
    pub fn read_in_band(bits: &mut BitReader<'_>) -> Result<(Self, Option<u16>)> {
        let mut reader = bits.clone();
        if reader.remaining() < 8 {
            return Err(invalid("truncated in-band LD-MPS header"));
        }
        let aligned = reader.bit()?;
        let mut bytes = reader.read(7)? as usize;
        if bytes == 127 {
            if reader.remaining() < 16 {
                return Err(invalid("truncated extended in-band LD-MPS length"));
            }
            bytes += reader.read(16)? as usize;
        }
        if bytes == 0 {
            return Err(invalid("empty in-band LD-MPS configuration"));
        }
        let config = Self::read(&mut reader, bytes)?;
        let delay = if aligned {
            if reader.remaining() < 16 {
                return Err(invalid("truncated in-band LD-MPS time alignment"));
            }
            Some(reader.read(16)? as u16)
        } else {
            None
        };
        *bits = reader;
        Ok((config, delay))
    }
    pub fn parse(payload: &[u8]) -> Result<Self> {
        Self::read(&mut BitReader::new(payload), payload.len())
    }
    pub fn read_prefix(bytes: &[u8], offset: usize, count: usize) -> Result<(Self, usize)> {
        let mut bits = BitReader::new(bytes);
        bits.skip(offset)?;
        let value = Self::read(&mut bits, count)?;
        Ok((value, bits.position() - offset))
    }
    /// A configuration is a byte-length field, even when embedded at a non-byte
    /// offset. Its internal alignment is relative to the configuration origin.
    /// Commit the caller cursor only after all bounded children succeed.
    pub fn read(bits: &mut BitReader<'_>, bytes: usize) -> Result<Self> {
        let count = bytes
            .checked_mul(8)
            .ok_or_else(|| invalid("oversized LD-MPS configuration"))?;
        if count > bits.remaining() {
            return Err(invalid("truncated LD-MPS configuration"));
        }
        let start = bits.position();
        let end = start
            .checked_add(count)
            .ok_or_else(|| invalid("oversized LD-MPS configuration"))?;
        let mut b = BitReader::with_bit_len(bits.source_bytes(), end)?;
        b.skip(start)?;
        let value = Self::body(&mut b, start)?;
        bits.skip(count)?;
        Ok(value)
    }
    fn body(b: &mut BitReader<'_>, start: usize) -> Result<Self> {
        let index = b.read(4)?;
        let sample_rate = if index == 15 {
            b.read(24)?
        } else {
            rate(index)?
        };
        if sample_rate == 0 {
            return Err(invalid("zero LD-MPS frequency"));
        }
        let frame_slots = b.read(5)? as u8 + 1;
        let parameter_bands = band_count(b)?;
        let tree = Tree::new(b.read(4)? as u8)?;
        let quantization_mode = limited(b, 2, 2, "reserved LD-MPS quantization mode")?;
        let one_icc = if tree.code == 7 { None } else { Some(b.bit()?) };
        let arbitrary_downmix = b.bit()?;
        let (surround_gain, lfe_gain) = if tree.code == 7 {
            (None, None)
        } else {
            (
                Some(limited(b, 3, 4, "reserved LD-MPS surround gain")?),
                Some(limited(b, 3, 4, "reserved LD-MPS LFE gain")?),
            )
        };
        let downmix_gain = b.read(3)? as u8;
        let matrix_mode = if tree.code == 7 { None } else { Some(b.bit()?) };
        let temporal_shape = limited(b, 2, 2, "reserved LD-MPS temporal shape")?;
        let decorrelation = limited(b, 2, 2, "reserved LD-MPS decorrelation")?;
        let binaural = tree.code != 7 && b.bit()?;
        let mut ott_bands = Vec::new();
        for &lfe in &tree.ott_lfe {
            ott_bands.push(if lfe {
                limited(b, 5, parameter_bands, "LD-MPS OTT bands exceed resolution")?
            } else {
                parameter_bands
            });
        }
        let mut ttt = Vec::new();
        for _ in 0..tree.ttt_boxes {
            let dual = b.bit()?;
            let low_mode = limited(b, 3, 5, "reserved LD-MPS TTT mode")?;
            let high = if dual {
                Some((
                    limited(b, 3, 5, "reserved LD-MPS TTT mode")?,
                    limited(b, 5, parameter_bands, "LD-MPS TTT bands exceed resolution")?,
                ))
            } else {
                None
            };
            ttt.push(Ttt { low_mode, high });
        }
        if temporal_shape == 2 && b.bit()? {
            return Err(invalid("reserved LD-MPS envelope quantization"));
        }
        let hrtf = if binaural {
            if b.read(2)? != 0 {
                return Err(invalid("reserved LD-MPS HRTF set"));
            }
            let bands = band_count(b)?;
            let asymmetric = b.bit()?;
            let mut channels = Vec::new();
            for _ in 0..5 {
                let left = (0..bands)
                    .map(|_| b.read(6).map(|v| v as u8))
                    .collect::<Result<Vec<_>>>()?;
                let right = if asymmetric {
                    (0..bands)
                        .map(|_| b.read(6).map(|v| v as u8))
                        .collect::<Result<Vec<_>>>()?
                } else {
                    left.clone()
                };
                let phase = if b.bit()? {
                    Some(
                        (0..bands)
                            .map(|_| b.read(6).map(|v| v as u8))
                            .collect::<Result<Vec<_>>>()?,
                    )
                } else {
                    None
                };
                channels.push(HrtfChannel { left, right, phase });
            }
            Some(Hrtf {
                bands,
                asymmetric,
                channels,
            })
        } else {
            None
        };
        b.skip((8 - (b.position() - start) % 8) % 8)?;
        let mut extensions = Vec::new();
        while b.remaining() > 0 {
            let kind = b.read(4)? as u8;
            let mut bytes = b.read(4)? as usize;
            if bytes == 15 {
                let extra = b.read(8)? as usize;
                bytes += extra;
                if extra == 255 {
                    bytes += b.read(16)? as usize;
                }
            }
            if bytes > b.remaining() / 8 {
                return Err(invalid("truncated LD-MPS extension configuration"));
            }
            let payload = (0..bytes)
                .map(|_| b.read(8).map(|v| v as u8))
                .collect::<Result<Vec<_>>>()?;
            let value = Self::extension(kind, &payload, &tree, parameter_bands)?;
            extensions.push(Extension {
                kind,
                payload,
                value,
            });
        }
        Ok(Self {
            sample_rate,
            frame_slots,
            parameter_bands,
            tree,
            quantization_mode,
            one_icc,
            arbitrary_downmix,
            surround_gain,
            lfe_gain,
            downmix_gain,
            matrix_mode,
            temporal_shape,
            decorrelation,
            ott_bands,
            ttt,
            hrtf,
            extensions,
        })
    }
    pub(super) fn extension(
        kind: u8,
        payload: &[u8],
        tree: &Tree,
        parameter_bands: u8,
    ) -> Result<ExtensionValue> {
        let mut b = BitReader::new(payload);
        Ok(match kind {
            0 => {
                let rate = residual_rate(&mut b)?;
                let frames_code = b.read(2)? as u8;
                let mut bands = Vec::new();
                for _ in 0..tree.ott_lfe.len() + usize::from(tree.ttt_boxes) {
                    bands.push(if b.bit()? {
                        Some(limited(
                            &mut b,
                            5,
                            parameter_bands,
                            "LD-MPS residual bands exceed resolution",
                        )?)
                    } else {
                        None
                    });
                }
                ExtensionValue::Residual {
                    rate,
                    frames_code,
                    bands,
                }
            }
            1 => ExtensionValue::DownmixResidual {
                rate: residual_rate(&mut b)?,
                frames_code: b.read(2)? as u8,
                bands: limited(
                    &mut b,
                    5,
                    parameter_bands,
                    "LD-MPS residual bands exceed resolution",
                )?,
            },
            2 => {
                let mut nodes = Vec::new();
                let mut boxes = 0usize;
                let mut leaves = 0usize;
                for _ in 0..tree.output_channels {
                    let mut row = Vec::new();
                    let mut open = 1usize;
                    while open != 0 {
                        let split = b.bit()?;
                        row.push(split);
                        if split {
                            open += 1;
                            boxes += 1;
                        } else {
                            open -= 1;
                            leaves += 1;
                        }
                    }
                    nodes.push(row);
                }
                let mut ott = Vec::new();
                for _ in 0..boxes {
                    let default_cld = b.bit()?;
                    let lfe = b.bit()?;
                    let bands = if lfe {
                        limited(
                            &mut b,
                            5,
                            parameter_bands,
                            "LD-MPS arbitrary OTT bands exceed resolution",
                        )?
                    } else {
                        parameter_bands
                    };
                    ott.push(ArbitraryOtt {
                        default_cld,
                        lfe,
                        bands,
                    });
                }
                let positions = (0..leaves)
                    .map(|_| b.read(5).map(|v| v as u8))
                    .collect::<Result<Vec<_>>>()?;
                ExtensionValue::ArbitraryTree {
                    nodes,
                    ott,
                    positions,
                }
            }
            _ => ExtensionValue::Unknown,
        })
    }
}

impl Config {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        f.vector(&self.tree.ott_lfe)?;
        f.vector(&self.ott_bands)?;
        f.vector(&self.ttt)?;
        if let Some(hrtf) = &self.hrtf {
            f.vector(&hrtf.channels)?;
            for channel in &hrtf.channels {
                f.vector(&channel.left)?;
                f.vector(&channel.right)?;
                if let Some(phase) = &channel.phase {
                    f.vector(phase)?;
                }
            }
        }
        f.vector(&self.extensions)?;
        for ext in &self.extensions {
            f.vector(&ext.payload)?;
            match &ext.value {
                ExtensionValue::Residual { bands, .. } => f.vector(bands)?,
                ExtensionValue::ArbitraryTree {
                    nodes,
                    ott,
                    positions,
                } => {
                    f.vector(nodes)?;
                    for row in nodes {
                        f.vector(row)?;
                    }
                    f.vector(ott)?;
                    f.vector(positions)?;
                }
                ExtensionValue::DownmixResidual { .. } | ExtensionValue::Unknown => {}
            }
        }
        Ok(())
    }
}
