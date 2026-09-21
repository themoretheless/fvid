//! Native single-slice HEVC IDR reconstruction. Unsupported tools fail explicitly.
use super::{
    hevc_block,
    hevc_cabac::{HevcCabac, SliceType, Syntax},
    hevc_intra, hevc_intra_syntax,
    hevc_plane::Plane,
    hevc_pps::Pps,
    hevc_qp, hevc_sao,
    hevc_slice::SliceHeader,
    hevc_sps::Sps,
    hevc_transform_tree,
    hevc_tree::{self, Node, Visitor},
};
use crate::{Result, invalid};

pub struct Picture {
    pub dimensions: [u32; 2],
    pub crop: [u32; 4],
    pub depth: [u8; 2],
    pub planes: [Plane; 3],
    /// Resolved SAO parameters in raster CTU order, including merged values.
    pub sao: Vec<hevc_sao::CtuSao>,
}
/// Decode a complete IDR slice. Currently supports 4:2:0 Main/Main10, fixed QP,
/// one slice, no PCM, tiles, WPP or deblocking. These tools are rejected,
/// never silently skipped. Output planes have coded dimensions; crop is metadata.
/// Budget covers owned pixels, CU/PB metadata and a conservative scratch reserve.
pub fn decode_idr(sps: &Sps, pps: &Pps, slice: &SliceHeader, budget: usize) -> Result<Picture> {
    if sps.chroma_format != 1
        || sps.separate_colour_plane
        || sps.pcm.is_some()
        || pps.tiles.is_some()
        || pps.entropy_sync
        || pps.cu_qp_delta_depth.is_some()
        || (slice.sao != [false, false] && pps.transquant_bypass)
        || !slice.deblocking.disabled
        || !slice.first
        || slice.address != 0
        || !slice.nal.is_idr()
        || slice.nal.layer_id != 0
        || slice.nal.temporal_id != 0
        || !slice.entry_point_offsets.is_empty()
    {
        return Err(invalid("unsupported HEVC IDR picture tools"));
    }
    let [min_cb, max_cb] = sps.coding_block_log2;
    let [min_tb, max_tb] = sps.transform_block_log2;
    if !(4..=6).contains(&max_cb)
        || !(3..=max_cb).contains(&min_cb)
        || !(2..=min_cb.min(5)).contains(&min_tb)
        || !(min_tb..=max_cb.min(5)).contains(&max_tb)
        || sps.transform_hierarchy_depth[1] > max_cb - min_tb
        || sps.depth.iter().any(|d| !(8..=10).contains(d))
        || sps.id != pps.sps_id
        || pps.id != slice.pps_id
    {
        return Err(invalid("invalid HEVC picture parameters"));
    }
    let [w, h] = sps.dimensions;
    if w == 0
        || h == 0
        || w % (1 << min_cb) != 0
        || h % (1 << min_cb) != 0
        || u64::from(sps.crop[0]) + u64::from(sps.crop[1]) >= u64::from(w)
        || u64::from(sps.crop[2]) + u64::from(sps.crop[3]) >= u64::from(h)
    {
        return Err(invalid("invalid HEVC picture dimensions or crop"));
    }
    let count = (w as usize)
        .checked_mul(h as usize)
        .ok_or_else(|| invalid("HEVC picture size overflow"))?;
    let required = count
        .checked_mul(12)
        .and_then(|n| n.checked_add(65536))
        .ok_or_else(|| invalid("HEVC picture budget overflow"))?;
    if required > budget {
        return Err(invalid("HEVC picture exceeds decode budget"));
    }
    let chroma_offsets = slice.chroma_qp_offsets.map(i32::from);
    hevc_qp::components(slice.qp, sps.depth, chroma_offsets)?;
    let bit_offset = slice
        .entropy_byte_offset
        .checked_mul(8)
        .ok_or_else(|| invalid("HEVC entropy offset overflow"))?;
    let mut bins = HevcCabac::new(&slice.rbsp, bit_offset, SliceType::I, false, slice.qp)?;
    let mut decoder = Decoder {
        sps,
        pps,
        qp: slice.qp,
        chroma_offsets,
        leaves: Vec::with_capacity(count / (1usize << (2 * min_cb))),
        modes: Vec::with_capacity(count / (1usize << (2 * (min_cb - 1)))),
        planes: [
            Plane::new(w as usize, h as usize, sps.depth[0], count * 3)?,
            Plane::new(w as usize / 2, h as usize / 2, sps.depth[1], count * 3 / 4)?,
            Plane::new(w as usize / 2, h as usize / 2, sps.depth[1], count * 3 / 4)?,
        ],
    };
    let side = 1u32 << max_cb;
    let columns = w.div_ceil(side);
    let rows = h.div_ceil(side);
    let mut sao = Vec::with_capacity(columns as usize * rows as usize);
    for row in 0..rows {
        for col in 0..columns {
            let index = sao.len();
            let parameters = hevc_sao::read_ctu(
                &mut bins,
                slice.sao,
                sps.depth,
                if col > 0 { sao.get(index - 1) } else { None },
                if row > 0 {
                    sao.get(index - columns as usize)
                } else {
                    None
                },
            )?;
            sao.push(parameters);
            hevc_tree::read_ctu(
                &mut bins,
                &mut decoder,
                [w, h],
                [col * side, row * side],
                max_cb,
                min_cb,
            )?;
            let end = bins.terminate()?;
            let last = row == rows - 1 && col == columns - 1;
            if end != last {
                return Err(invalid(
                    "HEVC slice termination does not match picture extent",
                ));
            }
        }
    }
    if !decoder.planes.iter().all(Plane::complete) {
        return Err(invalid("incomplete HEVC picture"));
    }
    if slice.sao != [false, false] {
        for (component, plane) in decoder.planes.iter_mut().enumerate() {
            let parameters: Vec<_> = sao.iter().map(|p| p[component]).collect();
            plane.apply_sao(max_cb - u8::from(component != 0), &parameters)?;
        }
    }
    Ok(Picture {
        dimensions: [w, h],
        crop: sps.crop,
        depth: sps.depth,
        planes: decoder.planes,
        sao,
    })
}
struct Decoder<'a> {
    sps: &'a Sps,
    pps: &'a Pps,
    qp: i32,
    chroma_offsets: [i32; 2],
    leaves: Vec<Node>,
    planes: [Plane; 3],
    modes: Vec<(Node, u8)>,
}
impl<'a> Visitor<HevcCabac<'a>> for Decoder<'_> {
    fn neighbouring_depths(&self, n: Node) -> [Option<u8>; 2] {
        let at = |x: u32, y: u32| {
            self.leaves
                .iter()
                .find(|l| {
                    x >= l.x && y >= l.y && x - l.x < 1 << l.log2_size && y - l.y < 1 << l.log2_size
                })
                .map(|l| l.depth)
        };
        [
            n.x.checked_sub(1).and_then(|x| at(x, n.y)),
            n.y.checked_sub(1).and_then(|y| at(n.x, y)),
        ]
    }
    fn enter(&mut self, _: Node, _: bool) -> Result<()> {
        Ok(())
    }
    fn leaf(&mut self, b: &mut HevcCabac<'a>, n: Node) -> Result<()> {
        let bypass = self.pps.transquant_bypass && b.decision(Syntax::TransquantBypass, 0)?;
        let nxn =
            n.log2_size == self.sps.coding_block_log2[0] && !b.decision(Syntax::PartMode, 0)?;
        let codes = hevc_intra_syntax::read_luma(b, nxn)?;
        let mut mode = 0;
        for (i, code) in codes.into_iter().enumerate() {
            let log = n.log2_size - u8::from(nxn);
            let p = Node {
                x: n.x + (i as u32 % 2) * (1 << log),
                y: n.y + (i as u32 / 2) * (1 << log),
                log2_size: log,
                depth: n.depth,
            };
            let neighbour = |x: u32, y: u32| {
                self.modes
                    .iter()
                    .find(|(l, _)| {
                        x >= l.x
                            && y >= l.y
                            && x - l.x < 1 << l.log2_size
                            && y - l.y < 1 << l.log2_size
                    })
                    .map(|(_, m)| *m)
            };
            let left = p.x.checked_sub(1).and_then(|x| neighbour(x, p.y));
            let top = if p.y % (1 << self.sps.coding_block_log2[1]) == 0 {
                None
            } else {
                p.y.checked_sub(1).and_then(|y| neighbour(p.x, y))
            };
            let derived = code.resolve(left, top)?;
            if i == 0 {
                mode = derived;
            }
            self.modes.push((p, derived));
        }
        let chroma = hevc_intra::chroma_mode(mode, hevc_intra_syntax::read_chroma(b)?)?;
        let c = hevc_transform_tree::Config {
            log2_cu: n.log2_size,
            log2_min_transform: self.sps.transform_block_log2[0],
            log2_max_transform: self.sps.transform_block_log2[1],
            max_depth: self.sps.transform_hierarchy_depth[1] + u8::from(nxn),
            intra_split: nxn,
        };
        hevc_transform_tree::read_intra(b, [n.x, n.y], c, |b, u| {
            let mode = self
                .modes
                .iter()
                .find(|(p, _)| {
                    u.origin[0] >= p.x
                        && u.origin[1] >= p.y
                        && u.origin[0] - p.x < 1 << p.log2_size
                        && u.origin[1] - p.y < 1 << p.log2_size
                })
                .ok_or_else(|| invalid("HEVC transform has no prediction block"))?
                .1;
            let qps = hevc_qp::components(self.qp, self.sps.depth, self.chroma_offsets)?;
            for component in 0..3 {
                if component != 0 && !u.owns_chroma {
                    continue;
                }
                let c = hevc_block::Config {
                    log2_size: if component == 0 {
                        u.log2_size
                    } else {
                        u.log2_chroma_size
                    },
                    component: component as u8,
                    bit_depth: self.sps.depth[usize::from(component != 0)],
                    qp: qps[component],
                    intra_mode: Some(if component == 0 { mode } else { chroma }),
                    transform_skip_enabled: self.pps.transform_skip,
                    transquant_bypass: bypass,
                    sign_hiding: self.pps.sign_data_hiding,
                };
                let residual = if u.coded[component] {
                    let r = hevc_block::decode(
                        b,
                        c,
                        self.pps
                            .scaling_lists
                            .as_ref()
                            .unwrap_or(&self.sps.scaling_lists),
                    )?;
                    r
                } else {
                    vec![0; 1 << (2 * c.log2_size)]
                };
                let origin = if component == 0 {
                    u.origin
                } else {
                    u.chroma_origin
                };
                self.planes[component].reconstruct_intra(
                    origin.map(|v| v as usize),
                    c.log2_size,
                    c.intra_mode.unwrap(),
                    component != 0,
                    self.sps.strong_intra_smoothing,
                    &residual,
                    |_, _| true,
                )?;
            }
            Ok(())
        })?;
        self.leaves.push(n);
        Ok(())
    }
}
