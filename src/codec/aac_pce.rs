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
    /// Canonical WAVE speaker order for unambiguous horizontal PCE layouts.
    pub fn pcm_layout(&self) -> Result<(u32, Vec<usize>)> {
        if !self.coupling.is_empty() || self.comment.first() == Some(&0xac) {
            return Err(invalid(
                "AAC PCE coupling or height layout is not implemented",
            ));
        }
        let mut speakers = Vec::new();
        for position in [
            Position::Front,
            Position::Side,
            Position::Back,
            Position::Lfe,
        ] {
            let elements: Vec<_> = self
                .elements
                .iter()
                .filter(|e| e.position == position)
                .collect();
            let count: usize = elements.iter().map(|e| if e.pair { 2 } else { 1 }).sum();
            let group: &[u8] = match (position, count) {
                (_, 0) => &[],
                (Position::Front, 1) => &[2],
                (Position::Front, 2) => &[0, 1],
                (Position::Front, 3) => &[2, 0, 1],
                (Position::Front, 5) => &[2, 6, 7, 0, 1],
                (Position::Side, 2) => &[9, 10],
                (Position::Back, 1) => &[8],
                (Position::Back, 2) => &[4, 5],
                (Position::Back, 3) => &[8, 4, 5],
                (Position::Lfe, 1) => &[3],
                _ => return Err(invalid("ambiguous AAC PCE speaker layout")),
            };
            let expected_pairs: &[bool] = match count {
                0 => &[],
                1 => &[false],
                2 => &[true],
                3 => &[false, true],
                5 => &[false, true, true],
                _ => unreachable!(),
            };
            if elements
                .iter()
                .map(|e| e.pair)
                .ne(expected_pairs.iter().copied())
            {
                return Err(invalid(
                    "AAC PCE element grouping does not identify speakers",
                ));
            }
            speakers.extend_from_slice(group);
        }
        let mask = speakers.iter().fold(0u32, |m, s| m | (1 << s));
        let mapping = speakers
            .iter()
            .map(|s| (mask & ((1 << s) - 1)).count_ones() as usize)
            .collect();
        Ok((mask, mapping))
    }
    /// Serialize AAC-LC initialization with this explicit program.
    pub fn audio_specific_config(&self) -> Result<Vec<u8>> {
        if self.elements.len() > 48
            || self.associated_data.len() > 7
            || self.coupling.len() > 15
            || self.comment.len() > 255
            || self.tag > 15
            || self.elements.iter().any(|e| e.tag > 15)
        {
            return Err(invalid("PCE fields exceed their syntax limits"));
        }
        let rates = [
            96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
        ];
        let index = rates
            .iter()
            .position(|r| *r == self.sample_rate)
            .ok_or_else(|| invalid("PCE sample rate has no index"))?;
        if self.object_type != 2 {
            return Err(invalid("PCE initialization requires AAC-LC"));
        }
        let mut fields = Vec::<bool>::new();
        let put = |fields: &mut Vec<bool>, value: u32, count: u8| {
            for shift in (0..count).rev() {
                fields.push(value & (1 << shift) != 0);
            }
        };
        put(&mut fields, 2, 5);
        put(&mut fields, index as u32, 4);
        put(&mut fields, 0, 4);
        put(&mut fields, 0, 3);
        put(&mut fields, u32::from(self.tag), 4);
        put(&mut fields, 1, 2);
        put(&mut fields, index as u32, 4);
        for (position, width) in [
            (Position::Front, 4),
            (Position::Side, 4),
            (Position::Back, 4),
            (Position::Lfe, 2),
        ] {
            put(
                &mut fields,
                self.elements
                    .iter()
                    .filter(|e| e.position == position)
                    .count() as u32,
                width,
            );
        }
        put(&mut fields, self.associated_data.len() as u32, 3);
        put(&mut fields, self.coupling.len() as u32, 4);
        for tag in [self.mono_mixdown, self.stereo_mixdown] {
            put(&mut fields, u32::from(tag.is_some()), 1);
            if let Some(tag) = tag {
                put(&mut fields, u32::from(tag), 4);
            }
        }
        put(&mut fields, u32::from(self.matrix_mixdown.is_some()), 1);
        if let Some((index, pseudo)) = self.matrix_mixdown {
            put(&mut fields, u32::from(index), 2);
            put(&mut fields, u32::from(pseudo), 1);
        }
        for e in &self.elements {
            if e.position != Position::Lfe {
                put(&mut fields, u32::from(e.pair), 1);
            }
            put(&mut fields, u32::from(e.tag), 4);
        }
        for tag in &self.associated_data {
            put(&mut fields, u32::from(*tag), 4);
        }
        for (independent, tag) in &self.coupling {
            put(&mut fields, u32::from(*independent), 1);
            put(&mut fields, u32::from(*tag), 4);
        }
        while fields.len() % 8 != 0 {
            fields.push(false);
        }
        put(&mut fields, self.comment.len() as u32, 8);
        for b in &self.comment {
            put(&mut fields, u32::from(*b), 8);
        }
        let output: Vec<u8> = fields
            .chunks(8)
            .map(|chunk| {
                chunk
                    .iter()
                    .fold(0u8, |byte, bit| (byte << 1) | u8::from(*bit))
            })
            .collect();
        let (config, program) = super::config::AacConfig::parse_with_program(&output)?;
        if config.channels as usize != self.channels() || program.as_ref() != Some(self) {
            return Err(invalid(
                "PCE fields cannot be represented in initialization",
            ));
        }
        Ok(output)
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

/// Skip a data_stream_element after its three-bit element ID.
/// Parsing is transactional and byte alignment is relative to the raw block.
pub(crate) fn skip_data_stream(bits: &mut super::bits::BitReader<'_>) -> crate::Result<()> {
    let mut input = bits.clone();
    input.read(4)?;
    let align = input.bit()?;
    let mut count = input.read(8)? as usize;
    if count == 255 {
        count += input.read(8)? as usize;
    }
    if align {
        input.skip((8 - input.position() % 8) % 8)?;
    }
    input.skip(count * 8)?;
    *bits = input;
    Ok(())
}
