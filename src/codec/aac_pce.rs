//! Owned AAC Program Config Element syntax and tagged channel descriptions.
use super::bits::BitReader;
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Front,
    Side,
    Back,
    Lfe,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Element {
    pub position: Position,
    pub pair: bool,
    pub tag: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramConfig {
    pub tag: u8,
    pub object_type: u8,
    pub sample_rate: u32,
    pub elements: Vec<Element>,
    pub associated_data: Vec<u8>,
    pub coupling: Vec<(bool, u8)>,
    pub mono_mixdown: Option<u8>,
    pub stereo_mixdown: Option<u8>,
    pub matrix_mixdown: Option<(u8, bool)>,
    pub comment: Vec<u8>,
}
impl ProgramConfig {
    pub fn channels(&self) -> usize {
        self.elements
            .iter()
            .map(|e| if e.pair { 2 } else { 1 })
            .sum()
    }
    /// Includes element_instance_tag, but not the raw_data_block element ID.
    /// Alignment is relative to the caller's containing payload start.
    /// On malformed input, the caller's bit position remains unchanged.
    pub fn read(bits: &mut BitReader<'_>, alignment_origin: usize) -> Result<Self> {
        let mut input = bits.clone();
        if alignment_origin > input.position() {
            return Err(invalid("PCE alignment origin exceeds current position"));
        }
        let tag = input.read(4)? as u8;
        let object_type = input.read(2)? as u8 + 1;
        let index = input.read(4)? as usize;
        let sample_rate = *[
            96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
        ]
        .get(index)
        .ok_or_else(|| invalid("reserved AAC PCE frequency index"))?;
        let front = input.read(4)?;
        let side = input.read(4)?;
        let back = input.read(4)?;
        let lfe = input.read(2)?;
        let associated = input.read(3)?;
        let coupled = input.read(4)?;
        let mono_mixdown = if input.bit()? {
            Some(input.read(4)? as u8)
        } else {
            None
        };
        let stereo_mixdown = if input.bit()? {
            Some(input.read(4)? as u8)
        } else {
            None
        };
        let matrix_mixdown = if input.bit()? {
            Some((input.read(2)? as u8, input.bit()?))
        } else {
            None
        };
        let mut elements = Vec::new();
        let mut tags = std::collections::HashSet::new();
        for (position, count) in [
            (Position::Front, front),
            (Position::Side, side),
            (Position::Back, back),
            (Position::Lfe, lfe),
        ] {
            for _ in 0..count {
                let pair = position != Position::Lfe && input.bit()?;
                let tag = input.read(4)? as u8;
                let kind = if position == Position::Lfe {
                    3
                } else {
                    u8::from(pair)
                };
                if !tags.insert((kind, tag)) {
                    return Err(invalid("duplicate AAC PCE element tag"));
                }
                elements.push(Element {
                    position,
                    pair,
                    tag,
                });
            }
        }
        let mut associated_data = Vec::new();
        for _ in 0..associated {
            associated_data.push(input.read(4)? as u8);
        }
        let mut coupling = Vec::new();
        for _ in 0..coupled {
            coupling.push((input.bit()?, input.read(4)? as u8));
        }
        input.skip((8 - (input.position() - alignment_origin) % 8) % 8)?;
        let length = input.read(8)?;
        let mut comment = Vec::with_capacity(length as usize);
        for _ in 0..length {
            comment.push(input.read(8)? as u8);
        }
        let result = Self {
            tag,
            object_type,
            sample_rate,
            elements,
            associated_data,
            coupling,
            mono_mixdown,
            stereo_mixdown,
            matrix_mixdown,
            comment,
        };
        if result.channels() == 0 || result.channels() > 64 {
            return Err(invalid("AAC PCE requires 1..=64 channels"));
        }
        *bits = input;
        Ok(result)
    }
}
