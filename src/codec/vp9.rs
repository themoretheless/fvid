//! VP9 packet framing and uncompressed headers (bitstream specification v0.7).
//! Header state is independent of decoded reference pixels; parsing is not decoding.
use super::bits::BitReader;
use crate::{Result, invalid};
use std::ops::Range;

/// Locate up to eight frames without copying compressed data.
pub fn frames(packet: &[u8]) -> Result<Vec<&[u8]>> {
    let marker = *packet.last().ok_or_else(|| invalid("empty VP9 packet"))?;
    if marker & 0xe0 != 0xc0 {
        return Ok(vec![packet]);
    }
    let count = usize::from(marker & 7) + 1;
    let width = usize::from((marker >> 3) & 3) + 1;
    let index_len = 2 + count * width;
    // Both markers must match before treating the suffix as a superframe index.
    let Some(start) = packet.len().checked_sub(index_len) else {
        return Ok(vec![packet]);
    };
    if packet[start] != marker {
        return Ok(vec![packet]);
    }
    let mut result = Vec::with_capacity(count);
    let mut offset = 0usize;
    for entry in packet[start + 1..packet.len() - 1].chunks_exact(width) {
        let size = entry
            .iter()
            .enumerate()
            .fold(0usize, |n, (i, b)| n | (usize::from(*b) << (8 * i)));
        let end = offset
            .checked_add(size)
            .ok_or_else(|| invalid("VP9 superframe size overflow"))?;
        if size == 0 || end > start {
            return Err(invalid("invalid VP9 superframe size"));
        }
        result.push(&packet[offset..end]);
        offset = end;
    }
    if offset != start {
        return Err(invalid("VP9 superframe sizes do not cover payload"));
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub profile: u8,
    pub bit_depth: u8,
    pub color_space: u8,
    pub full_range: bool,
    pub subsampling: [bool; 2],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    pub size: [u32; 2],
    pub render_size: [u32; 2],
    pub format: Format,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopFilter {
    pub level: u8,
    pub sharpness: u8,
    pub delta_enabled: bool,
    pub reference_deltas: [i16; 4],
    pub mode_deltas: [i16; 2],
}
impl Default for LoopFilter {
    fn default() -> Self {
        Self {
            level: 0,
            sharpness: 0,
            delta_enabled: true,
            reference_deltas: [1, 0, -1, -1],
            mode_deltas: [0; 2],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segmentation {
    pub enabled: bool,
    pub update_map: bool,
    pub temporal_update: bool,
    pub tree_probs: [u8; 7],
    pub pred_probs: [u8; 3],
    pub absolute: bool,
    pub features: [[Option<i16>; 4]; 8],
}
impl Default for Segmentation {
    fn default() -> Self {
        Self {
            enabled: false,
            update_map: false,
            temporal_update: false,
            tree_probs: [255; 7],
            pred_probs: [255; 3],
            absolute: false,
            features: [[None; 4]; 8],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub picture: Reference,
    pub show_existing: Option<u8>,
    pub keyframe: bool,
    pub intra_only: bool,
    pub show_frame: bool,
    pub error_resilient: bool,
    pub reset_context: u8,
    pub refresh_context: bool,
    pub parallel: bool,
    /// Context index as signalled, before past-independence forces index zero.
    pub signalled_context: u8,
    pub refresh_references: u8,
    pub references: [u8; 3],
    pub sign_bias: [bool; 3],
    pub high_precision_mv: bool,
    /// Literal filter order: smooth, regular, sharp, bilinear; None is switchable.
    pub interpolation_filter: Option<u8>,
    pub loop_filter: LoopFilter,
    pub base_q: u8,
    pub delta_q: [i16; 3],
    pub segmentation: Segmentation,
    pub tile_log2: [u8; 2],
    pub compressed_header: Range<usize>,
    pub tiles: Range<usize>,
}
impl Header {
    pub fn lossless(&self) -> bool {
        self.base_q == 0 && self.delta_q == [0; 3]
    }
    pub fn is_intra(&self) -> bool {
        self.keyframe || self.intra_only
    }
    pub fn context_index(&self) -> u8 {
        if self.is_intra() || self.error_resilient {
            0
        } else {
            self.signalled_context
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderState {
    references: [Option<Reference>; 8],
    format: Option<Format>,
    loop_filter: LoopFilter,
    segmentation: Segmentation,
}
impl HeaderState {
    /// Advance header-only reference metadata atomically. A pixel decoder should
    /// parse a cloned state and commit it only after successfully decoding a frame.
    pub fn parse(&mut self, data: &[u8]) -> Result<Header> {
        let mut next = self.clone();
        let header = next.parse_inner(data)?;
        *self = next;
        Ok(header)
    }
    fn parse_inner(&mut self, data: &[u8]) -> Result<Header> {
        let mut b = BitReader::new(data);
        if b.read(2)? != 2 {
            return Err(invalid("invalid VP9 frame marker"));
        }
        let profile = b.read(1)? as u8 | ((b.read(1)? as u8) << 1);
        if profile == 3 {
            zero(&mut b)?;
        }
        if b.bit()? {
            let slot = b.read(3)? as usize;
            let picture = self.references[slot]
                .ok_or_else(|| invalid("VP9 show-existing reference is unavailable"))?;
            if picture.format.profile != profile {
                return Err(invalid("VP9 reference profile mismatch"));
            }
            align(&mut b)?;
            if data[b.position() / 8..].iter().any(|&v| v != 0) {
                return Err(invalid("nonzero VP9 show-existing padding"));
            }
            return Ok(Header {
                picture,
                show_existing: Some(slot as u8),
                keyframe: false,
                intra_only: false,
                show_frame: true,
                error_resilient: false,
                reset_context: 0,
                refresh_context: false,
                parallel: true,
                signalled_context: 0,
                refresh_references: 0,
                references: [0; 3],
                sign_bias: [false; 3],
                high_precision_mv: false,
                interpolation_filter: None,
                loop_filter: LoopFilter {
                    level: 0,
                    ..self.loop_filter.clone()
                },
                base_q: 0,
                delta_q: [0; 3],
                segmentation: self.segmentation.clone(),
                tile_log2: [0; 2],
                compressed_header: data.len()..data.len(),
                tiles: data.len()..data.len(),
            });
        }
        let keyframe = !b.bit()?;
        let show_frame = b.bit()?;
        let error_resilient = b.bit()?;
        let mut intra_only = false;
        let mut reset_context = 0;
        let refresh_references;
        let picture;
        let mut references = [0u8; 3];
        let mut sign_bias = [false; 3];
        let mut high_precision_mv = false;
        let mut interpolation_filter = None;
        if keyframe {
            sync(&mut b)?;
            let format = color(&mut b, profile)?;
            picture = dimensions(&mut b, format)?;
            refresh_references = 255;
        } else {
            if !show_frame {
                intra_only = b.bit()?;
            }
            if !error_resilient {
                reset_context = b.read(2)? as u8;
            }
            if intra_only {
                sync(&mut b)?;
                let format = if profile != 0 {
                    color(&mut b, profile)?
                } else {
                    Format {
                        profile: 0,
                        bit_depth: 8,
                        color_space: 1,
                        full_range: false,
                        subsampling: [true; 2],
                    }
                };
                refresh_references = b.read(8)? as u8;
                picture = dimensions(&mut b, format)?;
            } else {
                let format = self
                    .format
                    .ok_or_else(|| invalid("VP9 inter frame precedes an intra frame"))?;
                if format.profile != profile {
                    return Err(invalid("VP9 inter profile changed"));
                }
                refresh_references = b.read(8)? as u8;
                for i in 0..3 {
                    references[i] = b.read(3)? as u8;
                    sign_bias[i] = b.bit()?;
                }
                let mut size = None;
                for &slot in &references {
                    if b.bit()? {
                        size = Some(
                            self.references[usize::from(slot)]
                                .ok_or_else(|| invalid("VP9 size reference is unavailable"))?
                                .size,
                        );
                        break;
                    }
                }
                let size = match size {
                    Some(s) => s,
                    None => size_bits(&mut b)?,
                };
                let render_size = render(&mut b, size)?;
                picture = Reference {
                    size,
                    render_size,
                    format,
                };
                high_precision_mv = b.bit()?;
                if !b.bit()? {
                    interpolation_filter = Some(b.read(2)? as u8);
                }
                for &slot in &references {
                    let reference = self.references[usize::from(slot)]
                        .ok_or_else(|| invalid("VP9 inter reference is unavailable"))?;
                    if reference.format.bit_depth != format.bit_depth
                        || reference.format.subsampling != format.subsampling
                    {
                        return Err(invalid("incompatible VP9 reference format"));
                    }
                }
            }
        }
        let (refresh_context, parallel) = if error_resilient {
            (false, true)
        } else {
            (b.bit()?, b.bit()?)
        };
        let signalled_context = b.read(2)? as u8;
        if keyframe || intra_only || error_resilient {
            self.loop_filter = LoopFilter::default();
            self.segmentation = Segmentation::default();
        }
        let lf = &mut self.loop_filter;
        lf.level = b.read(6)? as u8;
        lf.sharpness = b.read(3)? as u8;
        lf.delta_enabled = b.bit()?;
        if lf.delta_enabled && b.bit()? {
            for delta in lf
                .reference_deltas
                .iter_mut()
                .chain(lf.mode_deltas.iter_mut())
            {
                if b.bit()? {
                    *delta = signed(&mut b, 6)?;
                }
            }
        }
        let base_q = b.read(8)? as u8;
        let mut delta_q = [0; 3];
        for delta in &mut delta_q {
            if b.bit()? {
                *delta = signed(&mut b, 4)?;
            }
        }
        segmentation(&mut b, &mut self.segmentation)?;
        let cols = picture.size[0].div_ceil(64);
        let mut min_cols = 0u8;
        while (64u32 << min_cols) < cols {
            min_cols += 1;
        }
        let mut max_cols = 0u8;
        while (cols >> (max_cols + 1)) >= 4 {
            max_cols += 1;
        }
        let mut tile_cols = min_cols;
        while tile_cols < max_cols && b.bit()? {
            tile_cols += 1;
        }
        let mut tile_rows = b.read(1)? as u8;
        if tile_rows != 0 {
            tile_rows += b.read(1)? as u8;
        }
        let header_size = b.read(16)? as usize;
        if header_size == 0 {
            return Err(invalid("empty VP9 compressed header"));
        }
        align(&mut b)?;
        let start = b.position() / 8;
        let end = start
            .checked_add(header_size)
            .ok_or_else(|| invalid("VP9 header size overflow"))?;
        if end >= data.len() {
            return Err(invalid("truncated VP9 compressed header or tiles"));
        }
        self.format = Some(picture.format);
        for (i, reference) in self.references.iter_mut().enumerate() {
            if refresh_references & (1 << i) != 0 {
                *reference = Some(picture);
            }
        }
        Ok(Header {
            picture,
            show_existing: None,
            keyframe,
            intra_only,
            show_frame,
            error_resilient,
            reset_context,
            refresh_context,
            parallel,
            signalled_context,
            refresh_references,
            references,
            sign_bias,
            high_precision_mv,
            interpolation_filter,
            loop_filter: self.loop_filter.clone(),
            base_q,
            delta_q,
            segmentation: self.segmentation.clone(),
            tile_log2: [tile_cols, tile_rows],
            compressed_header: start..end,
            tiles: end..data.len(),
        })
    }
}
fn zero(b: &mut BitReader<'_>) -> Result<()> {
    if b.bit()? {
        Err(invalid("nonzero reserved VP9 bit"))
    } else {
        Ok(())
    }
}
fn align(b: &mut BitReader<'_>) -> Result<()> {
    while b.position() % 8 != 0 {
        zero(b)?;
    }
    Ok(())
}
fn signed(b: &mut BitReader<'_>, count: u8) -> Result<i16> {
    let v = b.read(count)? as i16;
    Ok(if b.bit()? { -v } else { v })
}
fn sync(b: &mut BitReader<'_>) -> Result<()> {
    if b.read(24)? != 0x498342 {
        Err(invalid("invalid VP9 sync code"))
    } else {
        Ok(())
    }
}
fn color(b: &mut BitReader<'_>, profile: u8) -> Result<Format> {
    let bit_depth = if profile >= 2 {
        if b.bit()? { 12 } else { 10 }
    } else {
        8
    };
    let color_space = b.read(3)? as u8;
    if color_space == 6 {
        return Err(invalid("reserved VP9 color space"));
    }
    let full_range;
    let subsampling;
    if color_space == 7 {
        if profile & 1 == 0 {
            return Err(invalid("VP9 RGB requires profile 1 or 3"));
        }
        full_range = true;
        subsampling = [false; 2];
        zero(b)?;
    } else {
        full_range = b.bit()?;
        subsampling = if profile & 1 != 0 {
            let s = [b.bit()?, b.bit()?];
            zero(b)?;
            if s == [true; 2] {
                return Err(invalid("VP9 4:2:0 requires profile 0 or 2"));
            }
            s
        } else {
            [true; 2]
        };
    }
    Ok(Format {
        profile,
        bit_depth,
        color_space,
        full_range,
        subsampling,
    })
}
fn size_bits(b: &mut BitReader<'_>) -> Result<[u32; 2]> {
    Ok([b.read(16)? + 1, b.read(16)? + 1])
}
fn render(b: &mut BitReader<'_>, size: [u32; 2]) -> Result<[u32; 2]> {
    if b.bit()? { size_bits(b) } else { Ok(size) }
}
fn dimensions(b: &mut BitReader<'_>, format: Format) -> Result<Reference> {
    let size = size_bits(b)?;
    Ok(Reference {
        size,
        render_size: render(b, size)?,
        format,
    })
}
fn probability(b: &mut BitReader<'_>) -> Result<u8> {
    Ok(if b.bit()? { b.read(8)? as u8 } else { 255 })
}
fn segmentation(b: &mut BitReader<'_>, s: &mut Segmentation) -> Result<()> {
    s.enabled = b.bit()?;
    s.update_map = false;
    s.temporal_update = false;
    if !s.enabled {
        return Ok(());
    }
    s.update_map = b.bit()?;
    if s.update_map {
        for p in &mut s.tree_probs {
            *p = probability(b)?;
        }
        s.temporal_update = b.bit()?;
        for p in &mut s.pred_probs {
            *p = if s.temporal_update {
                probability(b)?
            } else {
                255
            };
        }
    }
    if b.bit()? {
        s.absolute = b.bit()?;
        for segment in &mut s.features {
            for (i, feature) in segment.iter_mut().enumerate() {
                *feature = if b.bit()? {
                    Some(if i < 2 {
                        signed(b, [8, 6][i])?
                    } else {
                        b.read([2, 0][i - 2])? as i16
                    })
                } else {
                    None
                };
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Tile<'a> {
    pub mi_rows: Range<u32>,
    pub mi_cols: Range<u32>,
    pub data: &'a [u8],
}
/// Split the tile partition using big-endian lengths and raster tile order.
pub fn tiles<'a>(frame: &'a [u8], header: &Header) -> Result<Vec<Tile<'a>>> {
    if header.show_existing.is_some() {
        return Ok(Vec::new());
    }
    if header.tile_log2[0] > 6
        || header.tile_log2[1] > 2
        || header.picture.size.contains(&0)
        || header.picture.size.iter().any(|&v| v > 65536)
    {
        return Err(invalid("invalid VP9 tile geometry"));
    }
    let payload = frame
        .get(header.tiles.clone())
        .ok_or_else(|| invalid("VP9 tile range exceeds frame"))?;
    let cols = 1u32 << header.tile_log2[0];
    let rows = 1u32 << header.tile_log2[1];
    let mi_cols = header.picture.size[0].div_ceil(8);
    let mi_rows = header.picture.size[1].div_ceil(8);
    let offset = |i: u32, count: u32, log2: u8| (((i * count.div_ceil(8)) >> log2) * 8).min(count);
    let mut result = Vec::with_capacity((cols * rows) as usize);
    let mut cursor = 0usize;
    for row in 0..rows {
        for col in 0..cols {
            let size = if row + 1 == rows && col + 1 == cols {
                payload.len() - cursor
            } else {
                let bytes = payload
                    .get(cursor..cursor + 4)
                    .ok_or_else(|| invalid("truncated VP9 tile size"))?;
                cursor += 4;
                u32::from_be_bytes(bytes.try_into().unwrap()) as usize
            };
            let end = cursor
                .checked_add(size)
                .ok_or_else(|| invalid("VP9 tile size overflow"))?;
            let data = payload
                .get(cursor..end)
                .filter(|d| !d.is_empty())
                .ok_or_else(|| invalid("empty or truncated VP9 tile"))?;
            result.push(Tile {
                mi_rows: offset(row, mi_rows, header.tile_log2[1])
                    ..offset(row + 1, mi_rows, header.tile_log2[1]),
                mi_cols: offset(col, mi_cols, header.tile_log2[0])
                    ..offset(col + 1, mi_cols, header.tile_log2[0]),
                data,
            });
            cursor = end;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<&'static [u8]> {
        let ivf = include_bytes!("../../tests/fixtures/vp9/header.ivf");
        let mut offset = 32;
        let mut packets = Vec::new();
        while offset < ivf.len() {
            let size = u32::from_le_bytes(ivf[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 12;
            packets.push(&ivf[offset..offset + size]);
            offset += size;
        }
        packets
    }
    #[test]
    fn real_key_and_inter_headers_and_atomic_truncation() {
        let packets = fixture();
        assert_eq!(packets.len(), 3);
        let mut state = HeaderState::default();
        assert!(state.parse(packets[1]).is_err());
        assert_eq!(state, HeaderState::default());
        for (i, packet) in packets.iter().enumerate() {
            let before = state.clone();
            let h = state.parse(packet).unwrap();
            assert_eq!(h.keyframe, i == 0);
            assert_eq!(h.picture.size, [32; 2]);
            assert_eq!(h.picture.format.bit_depth, 8);
            assert_eq!(h.picture.format.subsampling, [true; 2]);
            // Independently checked with FFmpeg trace_headers on this fixture.
            assert_eq!(h.base_q, [37, 128, 119][i]);
            assert_eq!(h.loop_filter.level, [0, 4, 8][i]);
            assert_eq!(h.compressed_header.len(), [34, 3, 3][i]);
            assert_eq!(h.compressed_header.start, [18, 10, 10][i]);
            assert!(h.show_frame);
            assert_eq!(tiles(packet, &h).unwrap().len(), 1);
            for end in 0..=h.compressed_header.end {
                let mut truncated = before.clone();
                assert!(
                    truncated.parse(&packet[..end]).is_err(),
                    "frame {i} prefix {end}"
                );
                assert_eq!(truncated, before);
            }
        }
        let existing = state.parse(&[0x88]).unwrap();
        assert_eq!(existing.show_existing, Some(0));
        assert_eq!(existing.picture.size, [32; 2]);
        assert_eq!(existing.refresh_references, 0);
        assert!(state.parse(&[0x88, 1]).is_err());
    }
    #[test]
    fn superframes_are_little_endian_and_bounded() {
        assert!(frames(&[]).is_err());
        let packet = [0x82, 0x83, 0x84, 0xc1, 1, 2, 0xc1];
        assert_eq!(frames(&packet).unwrap(), [&[0x82][..], &[0x83, 0x84][..]]);
        assert!(frames(&[0x82, 0xc0, 2, 0xc0]).is_err());
        assert!(frames(&[0x82, 0xc0, 0, 0xc0]).is_err());
        assert_eq!(frames(&[0x82, 0xc0]).unwrap().len(), 1);
        let mut large = vec![0; 258];
        large.extend_from_slice(&[0xc8, 2, 1, 0xc8]);
        assert_eq!(frames(&large).unwrap()[0].len(), 258);
    }
    #[test]
    fn tile_lengths_cannot_escape_the_partition() {
        let p = fixture();
        let mut state = HeaderState::default();
        let mut h = state.parse(p[0]).unwrap();
        h.picture.size = [512, 64];
        h.tile_log2 = [1, 0];
        let data = [0, 0, 0, 2, 0x10, 0x20, 0x30];
        h.tiles = 0..data.len();
        let t = tiles(&data, &h).unwrap();
        assert_eq!(t[0].mi_cols, 0..32);
        assert_eq!(t[1].mi_cols, 32..64);
        assert_eq!(t[0].data, [0x10, 0x20]);
        assert_eq!(t[1].data, [0x30]);
        for end in 0..data.len() {
            h.tiles = 0..end;
            assert!(tiles(&data, &h).is_err());
        }
        h.tiles = 0..7;
        assert!(tiles(&[255; 7], &h).is_err());
    }
}
