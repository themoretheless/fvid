use super::bits::BitReader;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Front,
    Side,
    Back,
    Lfe,
}
/// PCE's explicit vertical speaker layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HeightLayer {
    Normal,
    Top,
    Bottom,
}
/// One PCM channel's PCE placement. Within each layer/position, index follows
/// PCE element order (and left/right order within a pair).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelPosition {
    pub height: HeightLayer,
    pub position: Position,
    pub index: u8,
    pub group_channels: u8,
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
    /// Height per configured element, including normal-height LFE elements.
    /// Ordinary comments and short comments without a complete sync header do
    /// not constitute a height extension. Trailing application comment is kept.
    pub fn height_layers(&self) -> Result<Vec<HeightLayer>> {
        let mut result = vec![HeightLayer::Normal; self.elements.len()];
        if self.comment.first() != Some(&0xac) || self.comment.len() < 3 {
            return Ok(result);
        }
        let count = self
            .elements
            .iter()
            .filter(|e| e.position != Position::Lfe)
            .count();
        let payload_bytes = (count * 2).div_ceil(8);
        let end = 1 + payload_bytes;
        if self.comment.len() <= end {
            return Err(invalid("truncated AAC PCE height extension"));
        }
        let mut crc = 0xffu8;
        for &byte in &self.comment[..end] {
            crc ^= byte;
            for _ in 0..8 {
                crc = if crc & 0x80 != 0 {
                    (crc << 1) ^ 0x07
                } else {
                    crc << 1
                };
            }
        }
        if crc != self.comment[end] {
            return Err(invalid("AAC PCE height CRC mismatch"));
        }
        let mut bits = BitReader::new(&self.comment[1..end]);
        // PCE syntax groups front, side, back; manually constructed programs
        // need the same interpretation even if their vector order differs.
        for position in [Position::Front, Position::Side, Position::Back] {
            for (i, element) in self.elements.iter().enumerate() {
                if element.position == position {
                    result[i] = match bits.read(2)? {
                        0 => HeightLayer::Normal,
                        1 => HeightLayer::Top,
                        2 => HeightLayer::Bottom,
                        _ => return Err(invalid("invalid AAC PCE height layer")),
                    };
                }
            }
        }
        Ok(result)
    }
    /// WAVE speaker order if all positions have WAVE bits. Otherwise mask zero
    /// denotes explicitly positioned PCM in normal/top/bottom, front/side/back/
    /// LFE order. Use pcm_positions() to retain placements absent from WAVE.
    pub fn pcm_layout(&self) -> Result<(u32, Vec<usize>)> {
        let (mask, mapping, _) = self.layout()?;
        Ok((mask, mapping))
    }
    /// Positions in emitted PCM channel order, including layers WAVE cannot name.
    pub fn pcm_positions(&self) -> Result<Vec<ChannelPosition>> {
        Ok(self.layout()?.2)
    }
    fn layout(&self) -> Result<(u32, Vec<usize>, Vec<ChannelPosition>)> {
        if self.channels() == 0 || self.channels() > 64 {
            return Err(invalid("AAC PCE requires 1..=64 channels"));
        }
        let heights = self.height_layers()?;
        let offsets: Vec<_> = self
            .elements
            .iter()
            .scan(0usize, |at, e| {
                let offset = *at;
                *at += if e.pair { 2 } else { 1 };
                Some(offset)
            })
            .collect();
        let mut channels = Vec::new();
        for height in [HeightLayer::Normal, HeightLayer::Top, HeightLayer::Bottom] {
            for position in [
                Position::Front,
                Position::Side,
                Position::Back,
                Position::Lfe,
            ] {
                let elements: Vec<_> = self
                    .elements
                    .iter()
                    .enumerate()
                    .filter(|(i, e)| e.position == position && heights[*i] == height)
                    .collect();
                let count: usize = elements
                    .iter()
                    .map(|(_, e)| if e.pair { 2 } else { 1 })
                    .sum();
                let expected_pairs: Option<&[bool]> = match count {
                    0 => Some(&[]), 1 => Some(&[false]), 2 => Some(&[true]),
                    3 => Some(&[false,true]), 5 => Some(&[false,true,true]),
                    _ => None,
                };
                let identified = expected_pairs.is_some_and(|expected|
                    elements.iter().map(|(_,e)| e.pair).eq(expected.iter().copied()));
                let bits: Option<&[u8]> = match (height, position, count) {
                    (_, _, 0) => Some(&[]),
                    (HeightLayer::Normal, Position::Front, 1) => Some(&[2]),
                    (HeightLayer::Normal, Position::Front, 2) => Some(&[0, 1]),
                    (HeightLayer::Normal, Position::Front, 3) => Some(&[2, 0, 1]),
                    (HeightLayer::Normal, Position::Front, 5) => Some(&[2, 6, 7, 0, 1]),
                    (HeightLayer::Normal, Position::Side, 2) => Some(&[9, 10]),
                    (HeightLayer::Normal, Position::Back, 1) => Some(&[8]),
                    (HeightLayer::Normal, Position::Back, 2) => Some(&[4, 5]),
                    (HeightLayer::Normal, Position::Back, 3) => Some(&[8, 4, 5]),
                    (HeightLayer::Normal, Position::Lfe, 1) => Some(&[3]),
                    (HeightLayer::Top, Position::Front, 1) => Some(&[13]),
                    (HeightLayer::Top, Position::Front, 2) => Some(&[12, 14]),
                    (HeightLayer::Top, Position::Front, 3) => Some(&[13, 12, 14]),
                    (HeightLayer::Top, Position::Back, 1) => Some(&[16]),
                    (HeightLayer::Top, Position::Back, 2) => Some(&[15, 17]),
                    (HeightLayer::Top, Position::Back, 3) => Some(&[16, 15, 17]),
                    _ => None,
                };
                let bits = if identified { bits } else { None };
                let mut index = 0;
                for (i, e) in elements {
                    for ch in 0..if e.pair { 2 } else { 1 } {
                        channels.push((
                            offsets[i] + ch,
                            bits.map(|b| b[index]),
                            ChannelPosition {
                                height,
                                position,
                                index: index as u8,
                                group_channels: count as u8,
                            },
                        ));
                        index += 1;
                    }
                }
            }
        }
        let mask = if channels.iter().all(|c| c.1.is_some()) {
            channels.sort_by_key(|c| c.1.unwrap());
            channels.iter().fold(0u32, |m, c| m | (1 << c.1.unwrap()))
        } else {
            0
        };
        let mut mapping = vec![0; self.channels()];
        for (out, c) in channels.iter().enumerate() {
            mapping[c.0] = out;
        }
        Ok((mask, mapping, channels.into_iter().map(|c| c.2).collect()))
    }
    /// Serialize 1024-sample Main/LC/SSR/LTP initialization with this explicit program.
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
        if !matches!(self.object_type, 1 | 2 | 3 | 4) {
            return Err(invalid("PCE initialization requires AAC Main, LC, SSR or LTP"));
        }
        let mut fields = Vec::<bool>::new();
        let put = |fields: &mut Vec<bool>, value: u32, count: u8| {
            for shift in (0..count).rev() {
                fields.push(value & (1 << shift) != 0);
            }
        };
        put(&mut fields, u32::from(self.object_type), 5);
        put(&mut fields, index as u32, 4);
        put(&mut fields, 0, 4);
        put(&mut fields, 0, 3);
        put(&mut fields, u32::from(self.tag), 4);
        put(&mut fields, u32::from(self.object_type - 1), 2);
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
        while !fields.len().is_multiple_of(8) {
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
        result.height_layers()?;
        *bits = input;
        Ok(result)
    }
}

/// Skip a data_stream_element after its three-bit element ID.
/// Parsing is transactional and byte alignment is relative to the raw block.
pub(crate) fn skip_data_stream(bits: &mut super::bits::BitReader<'_>) -> Result<()> {
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

/// Skip supported fill/fill-data extensions after ID_FIL without committing a
/// partial read. Other extensions carry codec tools and cannot be discarded.
pub(crate) fn skip_fill(bits: &mut super::bits::BitReader<'_>) -> Result<()> {
    read_fill(bits, |_, _, _| Err(unsupported("AAC fill extension tool is not implemented")))
}
/// The SBR owner supplies transactional parsing for extension types 13/14;
/// all other fill/ancillary syntax uses the same bounded parser as AAC-LC.
pub(crate) fn read_fill(
    bits: &mut super::bits::BitReader<'_>,
    sbr: impl FnMut(&mut super::bits::BitReader<'_>, usize, bool) -> Result<()>,
) -> Result<()> {
    let mut input = bits.clone();
    let mut count = input.read(4)? as usize;
    if count == 15 {
        count += input.read(8)? as usize;
        count -= 1;
    }
    read_extension_bytes(&mut input, count, false, sbr)?;
    *bits = input;
    Ok(())
}

/// ER top-level payload has no FIL element/count header. Remaining whole
/// bytes contain extension_payload() records; final alignment stays outside.
pub(crate) fn skip_er_extensions(bits: &mut super::bits::BitReader<'_>) -> Result<()> {
    read_er_extensions(bits, |_, _, _| Err(unsupported("ER AAC SBR extension synthesis is not yet implemented")))
}

pub(crate) fn read_er_extensions(
    bits: &mut super::bits::BitReader<'_>,
    sbr: impl FnMut(&mut super::bits::BitReader<'_>, usize, bool) -> Result<()>,
) -> Result<()> {
    read_extension_bytes(bits, bits.remaining()/8, true, sbr)
}

fn read_extension_bytes(
    bits: &mut super::bits::BitReader<'_>,
    count: usize,
    er: bool,
    mut sbr: impl FnMut(&mut super::bits::BitReader<'_>, usize, bool) -> Result<()>,
) -> Result<()> {
    let mut input = bits.clone();
    let end = input.position() + count * 8;
    if count * 8 > input.remaining() {
        return Err(invalid("truncated AAC fill payload"));
    }
    let mut sbr_started=false;
    while input.position() < end {
        let read = |input: &mut super::bits::BitReader<'_>, width: u8| -> Result<u32> {
            if usize::from(width) > end - input.position() {
                return Err(invalid("AAC extension exceeds fill payload"));
            }
            input.read(width)
        };
        let kind=read(&mut input, 4)?;
        if er && kind==14 {return Err(invalid("ER AAC SBR CRC is forbidden"));}
        if er && sbr_started && kind!=13 {return Err(invalid("ER AAC extensions must precede SBR"));}
        match kind {
            0 => input.skip(end - input.position())?,
            1 => {
                if read(&mut input, 4)? != 0 {
                    return Err(invalid("invalid AAC fill-data nibble"));
                }
                while input.position() < end {
                    if read(&mut input, 8)? != 0xa5 {
                        return Err(invalid("invalid AAC fill-data byte"));
                    }
                }
            }
            2 => {
                if read(&mut input, 4)? != 0 {
                    return Err(unsupported("AAC ancillary data version is not implemented"));
                }
                let mut length = 0usize;
                loop {
                    let part = read(&mut input, 8)? as usize;
                    length += part;
                    if part != 255 {
                        break;
                    }
                }
                if length > (end - input.position()) / 8 {
                    return Err(invalid("AAC ancillary data exceeds fill payload"));
                }
                input.skip(length * 8)?;
            }
            11 => {
                // ISO 14496-3 tables 4.58/4.59. DRC evaluation is optional
                // (4.5.2.7.2); default decoding preserves original dynamics.
                if read(&mut input, 1)? != 0 {
                    read(&mut input, 4)?; // PCE instance tag
                    read(&mut input, 4)?; // reserved
                }
                if read(&mut input, 1)? != 0 {
                    loop {
                        read(&mut input, 7)?; // excluded channel mask
                        if read(&mut input, 1)? == 0 { break; }
                    }
                }
                let bands = if read(&mut input, 1)? != 0 {
                    let bands = read(&mut input, 4)? as usize + 1;
                    read(&mut input, 4)?; // interpolation scheme
                    for _ in 0..bands { read(&mut input, 8)?; }
                    bands
                } else { 1 };
                if read(&mut input, 1)? != 0 {
                    read(&mut input, 7)?; // program reference level
                    read(&mut input, 1)?; // reserved
                }
                for _ in 0..bands { read(&mut input, 8)?; } // sign + gain
            }
            kind @ (13 | 14) => {
                sbr_started=true;
                let start = input.position();
                sbr(&mut input, end, kind == 14)?;
                if input.position() <= start || input.position() > end {
                    return Err(invalid("invalid SBR fill extension consumption"));
                }
            }
            _ => return Err(unsupported("AAC fill extension tool is not implemented")),
        }
    }
    *bits = input;
    Ok(())
}

#[cfg(test)]
mod fill_tests {
    use super::*;
    #[test]
    fn dynamic_range_fill_is_bounded_at_every_bit_offset() {
        for offset in 0..8 {
            for (payload, accepted) in [
                (&[0xb0, 0x00][..], true),
                (&[0xb0, 0x7f, 0xb0, 0xff][..], true),
                (&[0xb0][..], false),
                (&[0xb8, 0x00][..], false),
                (&[0xb4, 0xff][..], false),
                (&[0xb2, 0xf0][..], false),
                (&[0xb1, 0x00][..], false),
            ] {
                let mut fields = vec![false; offset];
                for shift in (0..4).rev() {
                    fields.push(payload.len() & (1 << shift) != 0);
                }
                for byte in payload {
                    for shift in (0..8).rev() {
                        fields.push(byte & (1 << shift) != 0);
                    }
                }
                fields.extend([true; 64]); // later bytes cannot rescue a short FIL
                fields.resize(fields.len().next_multiple_of(8), false);
                let bytes: Vec<_> = fields.chunks_exact(8)
                    .map(|c| c.iter().fold(0u8, |n,b| n*2+u8::from(*b))).collect();
                let mut bits = BitReader::new(&bytes);
                bits.skip(offset).unwrap();
                assert_eq!(skip_fill(&mut bits).is_ok(), accepted, "{payload:x?}");
                assert_eq!(bits.position(), if accepted {offset+4+payload.len()*8} else {offset});
            }
        }
    }
    #[test]
    fn ancillary_fill_is_bounded_and_transactional_at_every_bit_offset() {
        for offset in 0..8 {
            for (payload, accepted) in [
                (&[0x20, 1, 0x55, 0x00][..], true),
                (&[0x20, 7, 0x55][..], false),
                (&[0x20, 255][..], false),
                (&[0x20, 0, 0xd0][..], false),
                (&[0x10, 0xa4][..], false),
            ] {
                let mut fields = vec![false; offset];
                for shift in (0..4).rev() {
                    fields.push(payload.len() & (1 << shift) != 0);
                }
                for byte in payload {
                    for shift in (0..8).rev() {
                        fields.push(byte & (1 << shift) != 0);
                    }
                }
                // Bytes after the declared FIL must never rescue its length.
                fields.extend([true; 32]);
                fields.resize(fields.len().next_multiple_of(8), false);
                let bytes: Vec<_> = fields
                    .chunks_exact(8)
                    .map(|c| c.iter().fold(0u8, |n, b| n * 2 + u8::from(*b)))
                    .collect();
                let mut bits = BitReader::new(&bytes);
                bits.skip(offset).unwrap();
                assert_eq!(skip_fill(&mut bits).is_ok(), accepted);
                assert_eq!(
                    bits.position(),
                    if accepted {
                        offset + 4 + payload.len() * 8
                    } else {
                        offset
                    }
                );
            }
        }
    }
}
