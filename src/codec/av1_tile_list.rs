//! AV1 large-scale tile-list syntax and externally anchored raster assembly.
use super::{
    av1_cdfs::Cdfs,
    av1_frame::Header,
    av1_picture::{self, Picture, Plane},
    av1_sequence::Sequence,
};
use crate::{Result, invalid};
#[derive(Clone, Debug)]
pub struct Entry<'a> {
    pub anchor: usize,
    pub row: usize,
    pub column: usize,
    pub data: &'a [u8],
}
#[derive(Clone, Debug)]
pub struct TileList<'a> {
    pub grid: [usize; 2],
    pub entries: Vec<Entry<'a>>,
}
impl<'a> TileList<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < 4 {
            return Err(invalid("truncated AV1 tile list header"));
        }
        let grid = [usize::from(data[0]) + 1, usize::from(data[1]) + 1];
        let count = usize::from(u16::from_be_bytes([data[2], data[3]])) + 1;
        if count > 512 || count > grid[0] * grid[1] {
            return Err(invalid("invalid AV1 tile list count"));
        }
        let mut at = 4;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let info = data
                .get(at..at + 5)
                .ok_or_else(|| invalid("truncated AV1 tile list entry"))?;
            if info[0] > 127 {
                return Err(invalid("invalid AV1 tile list anchor index"));
            }
            let length = usize::from(u16::from_be_bytes([info[3], info[4]])) + 1;
            at += 5;
            let payload = data
                .get(at..at + length)
                .ok_or_else(|| invalid("truncated AV1 coded tile data"))?;
            at += length;
            entries.push(Entry {
                anchor: usize::from(info[0]),
                row: usize::from(info[1]),
                column: usize::from(info[2]),
                data: payload,
            });
        }
        if at != data.len() {
            return Err(invalid("trailing AV1 tile list bytes"));
        }
        Ok(Self { grid, entries })
    }
}
#[derive(Clone, Debug)]
pub struct Output {
    pub decoded_tiles: usize,
    pub inter_blocks: u64,
    pub nonzero_motion_blocks: u64,
    pub fractional_motion_blocks: u64,
    pub size: [usize; 2],
    pub depth: u8,
    pub subsampling: [bool; 2],
    pub planes: [Plane; 3],
}
impl Output {
    pub(crate) fn bytes(&self) -> Result<usize> {
        self.planes.iter().try_fold(0usize, |n, p| {
            p.samples
                .len()
                .checked_mul(2)
                .and_then(|v| n.checked_add(v))
                .ok_or_else(|| invalid("AV1 tile output allocation overflow"))
        })
    }
}
fn geometry(s: &Sequence, h: &Header) -> Result<[usize; 2]> {
    if s.superres
        || s.order_hint_bits != 0
        || s.still_picture
        || s.reduced_header
        || s.film_grain
        || s.timing.is_some()
        || s.decoder_model.is_some()
        || s.operating_points
            .iter()
            .any(|p| p.initial_display_delay.is_some())
        || s.cdef
        || s.restoration
        || s.color.monochrome
        || h.references.iter().any(|i| *i >= 8)
        || h.primary_reference > 7
        || h.size.contains(&0)
        || h.size[0] > s.max_size[0]
        || h.size[1] > s.max_size[1]
        || h.size_override
        || h.grain.is_some()
        || h.intrabc
        || h.frame_type != 1
        || !h.show
        || h.error_resilient
        || !h.disable_cdf_update
        || !h.disable_frame_end_update
        || h.refresh_flags != 0
        || h.reference_mvs
        || h.segmentation_temporal_update
        || h.reference_select
        || h.quant.delta_resolution.is_some()
        || h.filter.delta_resolution.is_some()
        || h.filter.levels[..2] != [0, 0]
        || h.superres_denom != 8
        || h.upscaled_width != h.size[0]
    {
        return Err(invalid("invalid AV1 large-scale tile camera context"));
    }
    let sb = if s.superblock128 { 32 } else { 16 };
    let x = &h.tiles.columns;
    let y = &h.tiles.rows;
    if x.len() < 2
        || y.len() < 2
        || x.len() > 65
        || y.len() > 65
        || x[0] != 0
        || y[0] != 0
        || h.size.iter().any(|v| v % 8 != 0)
        || x.last() != Some(&(h.size[0] / 4))
        || y.last() != Some(&(h.size[1] / 4))
    {
        return Err(invalid("invalid AV1 camera tile layout"));
    }
    let width = x[1]
        .checked_sub(x[0])
        .ok_or_else(|| invalid("invalid AV1 camera tile width"))?;
    if width == 0
        || width % sb != 0
        || x.windows(2).any(|p| p[1].checked_sub(p[0]) != Some(width))
        || y.windows(2).any(|p| p[1].checked_sub(p[0]) != Some(sb))
    {
        return Err(invalid("nonuniform AV1 camera tile geometry"));
    }
    Ok([width as usize * 4, sb as usize * 4])
}
pub(crate) fn decode(
    s: &Sequence,
    h: &Header,
    list: &TileList<'_>,
    anchors: &[&Picture],
    initial: Option<&Cdfs>,
    previous: Option<&Output>,
    budget: usize,
    references: [Option<&Picture>; 8],
) -> Result<Output> {
    let tile_size = geometry(s, h)?;
    let size = [
        tile_size[0]
            .checked_mul(list.grid[0])
            .ok_or_else(|| invalid("AV1 tile output size overflow"))?,
        tile_size[1]
            .checked_mul(list.grid[1])
            .ok_or_else(|| invalid("AV1 tile output size overflow"))?,
    ];
    if anchors.len() > 128 {
        return Err(invalid("too many AV1 tile list anchors"));
    }
    for entry in &list.entries {
        let anchor = anchors
            .get(entry.anchor)
            .ok_or_else(|| invalid("missing AV1 external tile anchor"))?;
        if entry.column >= h.tiles.columns.len() - 1 || entry.row >= h.tiles.rows.len() - 1 {
            return Err(invalid("AV1 camera tile coordinate out of range"));
        }
        if anchor.size != h.size
            || anchor.depth != s.color.depth
            || anchor.subsampling != s.color.subsampling
        {
            return Err(invalid("AV1 external tile anchor geometry mismatch"));
        }
        for (p, plane) in anchor.planes.iter().enumerate() {
            let w = h.size[0] as usize
                >> if p == 0 {
                    0
                } else {
                    usize::from(s.color.subsampling[0])
                };
            let ht = h.size[1] as usize
                >> if p == 0 {
                    0
                } else {
                    usize::from(s.color.subsampling[1])
                };
            if plane.width != w || plane.height != ht || plane.samples.len() != w * ht {
                return Err(invalid("AV1 external tile anchor plane mismatch"));
            }
        }
    }
    let dims: [(usize, usize); 3] = std::array::from_fn(|p| {
        let sx = if p == 0 {
            0
        } else {
            usize::from(s.color.subsampling[0])
        };
        let sy = if p == 0 {
            0
        } else {
            usize::from(s.color.subsampling[1])
        };
        (size[0] >> sx, size[1] >> sy)
    });
    let bytes = dims.iter().try_fold(0usize, |n, (w, h)| {
        w.checked_mul(*h)
            .and_then(|v| v.checked_mul(2))
            .and_then(|v| n.checked_add(v))
            .ok_or_else(|| invalid("AV1 tile output allocation overflow"))
    })?;
    let working = budget
        .checked_sub(bytes)
        .ok_or_else(|| invalid("AV1 tile output exceeds memory budget"))?;
    if let Some(old) = previous {
        if old.size != size
            || old.depth != s.color.depth
            || old.subsampling != s.color.subsampling
            || old
                .planes
                .iter()
                .zip(&dims)
                .any(|(p, (w, h))| p.width != *w || p.height != *h || p.samples.len() != w * h)
        {
            return Err(invalid("AV1 previous tile output geometry mismatch"));
        }
    }
    let mut output = if let Some(old) = previous {
        old.clone()
    } else {
        Output {
            decoded_tiles: 0,
            inter_blocks: 0,
            nonzero_motion_blocks: 0,
            fractional_motion_blocks: 0,
            size,
            depth: s.color.depth,
            subsampling: s.color.subsampling,
            planes: std::array::from_fn(|p| Plane {
                width: dims[p].0,
                height: dims[p].1,
                samples: vec![0; dims[p].0 * dims[p].1],
            }),
        }
    };
    output.decoded_tiles = 0;
    output.inter_blocks = 0;
    output.nonzero_motion_blocks = 0;
    output.fractional_motion_blocks = 0;
    for (index, entry) in list.entries.iter().enumerate() {
        let mut references = references;
        references[h.references[0]] = Some(anchors[entry.anchor]);
        let tile = entry.row * (h.tiles.columns.len() - 1) + entry.column;
        let picture =
            av1_picture::decode_camera_tile(s, h, tile, entry.data, working, initial, references)?;
        output.decoded_tiles += 1;
        output.inter_blocks += u64::from(picture.inter_prediction.single_reference_blocks);
        output.nonzero_motion_blocks += u64::from(picture.inter_prediction.nonzero_motion_blocks);
        output.fractional_motion_blocks +=
            u64::from(picture.inter_prediction.fractional_motion_blocks);
        for p in 0..3 {
            let sx = if p == 0 {
                0
            } else {
                usize::from(s.color.subsampling[0])
            };
            let sy = if p == 0 {
                0
            } else {
                usize::from(s.color.subsampling[1])
            };
            let w = tile_size[0] >> sx;
            let ht = tile_size[1] >> sy;
            let from_x = (h.tiles.columns[entry.column] as usize * 4) >> sx;
            let from_y = (h.tiles.rows[entry.row] as usize * 4) >> sy;
            let to_x = (index % list.grid[0]) * w;
            let to_y = (index / list.grid[0]) * ht;
            let source = &picture.planes[p];
            let dest = &mut output.planes[p];
            for y in 0..ht {
                let from = (from_y + y) * source.width + from_x;
                let to = (to_y + y) * dest.width + to_x;
                dest.samples[to..to + w].copy_from_slice(&source.samples[from..from + w]);
            }
        }
    }
    Ok(output)
}
