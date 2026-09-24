use fvid::{
    Result,
    codec::{
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
    },
};
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
struct Decoder<'a> {
    sps: &'a Sps,
    pps: &'a Pps,
    qp: i32,
    leaves: Vec<Node>,
    blocks: Vec<Vec<i32>>,
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
        assert!(self.sps.pcm.is_none());
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
        let mut delta_read = false;
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
                .unwrap()
                .1;
            if u.coded.iter().any(|v| *v) && !delta_read && self.pps.cu_qp_delta_depth.is_some() {
                let delta = hevc_qp::read_delta(b, self.sps.depth[0])?;
                self.qp = hevc_qp::luma(self.qp, [None, None], delta, self.sps.depth[0])?;
                delta_read = true;
            }
            let qps = hevc_qp::components(self.qp, self.sps.depth, [0, 0])?;
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
                    let mut scratch = Vec::new();
                    let mut out = Vec::new();
                    hevc_block::decode(
                        b,
                        c,
                        self.pps
                            .scaling_lists
                            .as_ref()
                            .unwrap_or(&self.sps.scaling_lists),
                        &mut scratch,
                        &mut out,
                    )?;
                    self.blocks.push(out.clone());
                    out
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
                    &mut Vec::new(),
                    |_, _| true,
                )?;
            }
            Ok(())
        })?;
        self.leaves.push(n);
        Ok(())
    }
}
#[test]
fn real_gray_idr_entropy_reaches_termination() {
    // libx265 64x64 gray, one IDR, single-thread encoder; no runtime codec dependency.
    let sps = Sps::parse(
        &hex("42010101600000030090000003000003001ea020810596566924caf0168080000003008000000c84"),
        4096,
    )
    .unwrap();
    let pps = Pps::parse(&hex("4401c172b42240"), &sps, 4096).unwrap();
    let slice = SliceHeader::parse_idr(&hex("2801af1d80f7015bd6beccf0"), &sps, &pps, 4096).unwrap();
    let mut b = HevcCabac::new(
        &slice.rbsp,
        slice.entropy_byte_offset * 8,
        SliceType::I,
        false,
        slice.qp,
    )
    .unwrap();
    let sao = hevc_sao::read_ctu(&mut b, slice.sao, sps.depth, None, None).unwrap();
    let mut d = Decoder {
        sps: &sps,
        pps: &pps,
        qp: slice.qp,
        leaves: Vec::new(),
        blocks: Vec::new(),
        planes: [
            Plane::new(64, 64, 8, 16384).unwrap(),
            Plane::new(32, 32, 8, 4096).unwrap(),
            Plane::new(32, 32, 8, 4096).unwrap(),
        ],
        modes: Vec::new(),
    };
    hevc_tree::read_ctu(
        &mut b,
        &mut d,
        sps.dimensions,
        [0, 0],
        sps.coding_block_log2[1],
        sps.coding_block_log2[0],
    )
    .unwrap();
    assert!(b.terminate().unwrap());
    assert_eq!(sao, [hevc_sao::Sao::Off; 3]);
    assert_eq!(
        d.leaves
            .iter()
            .map(|n| (n.x, n.y, n.log2_size, n.depth))
            .collect::<Vec<_>>(),
        [(0, 0, 5, 1), (32, 0, 5, 1), (0, 32, 5, 1), (32, 32, 5, 1)]
    );
    assert_eq!(d.blocks, [vec![-2; 1024]]);
    assert!(d.planes.iter().all(Plane::complete));
    assert_eq!(d.planes[0].samples(), [126; 4096]);
    assert_eq!(d.planes[1].samples(), [128; 1024]);
    assert_eq!(d.planes[2].samples(), [128; 1024]);
}

fn detailed_fixture(
    sps_bytes: &[u8],
    pps_bytes: &[u8],
    slice_bytes: &[u8],
    expected: Vec<u16>,
    depth: u8,
) {
    let sps = Sps::parse(sps_bytes, 4096).unwrap();
    let pps = Pps::parse(pps_bytes, &sps, 4096).unwrap();
    let slice = SliceHeader::parse_idr(slice_bytes, &sps, &pps, 4096).unwrap();
    assert!(slice.deblocking.disabled);
    assert_eq!(slice.sao, [false, false]);
    assert!(pps.cu_qp_delta_depth.is_none());
    let mut b = HevcCabac::new(
        &slice.rbsp,
        slice.entropy_byte_offset * 8,
        SliceType::I,
        false,
        slice.qp,
    )
    .unwrap();
    let mut d = Decoder {
        sps: &sps,
        pps: &pps,
        qp: slice.qp,
        leaves: Vec::new(),
        blocks: Vec::new(),
        modes: Vec::new(),
        planes: [
            Plane::new(32, 32, depth, 4096).unwrap(),
            Plane::new(16, 16, depth, 1024).unwrap(),
            Plane::new(16, 16, depth, 1024).unwrap(),
        ],
    };
    hevc_tree::read_ctu(
        &mut b,
        &mut d,
        sps.dimensions,
        [0, 0],
        sps.coding_block_log2[1],
        sps.coding_block_log2[0],
    )
    .unwrap();
    assert!(b.terminate().unwrap());
    assert!(d.planes.iter().all(Plane::complete));
    assert!(d.modes.len() > d.leaves.len());
    let actual: Vec<_> = d
        .planes
        .iter()
        .flat_map(|p| p.samples().iter().copied())
        .collect();
    assert_eq!(actual.len(), expected.len());
    let mismatch: Vec<_> = actual
        .iter()
        .zip(&expected)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .take(12)
        .collect();
    assert!(mismatch.is_empty(), "pixel differences: {mismatch:?}");
    let picture = fvid::codec::hevc_picture::decode_idr(&sps, &pps, &slice, 1 << 20).unwrap();
    assert_eq!(picture.dimensions, [32, 32]);
    assert_eq!(picture.depth, [depth, depth]);
    let production: Vec<_> = picture
        .planes
        .iter()
        .flat_map(|p| p.samples().iter().copied())
        .collect();
    assert_eq!(production, expected);
    assert!(fvid::codec::hevc_picture::decode_idr(&sps, &pps, &slice, 100).is_err());
    let mut unsupported = slice.clone();
    unsupported.first = false;
    assert!(fvid::codec::hevc_picture::decode_idr(&sps, &pps, &unsupported, 1 << 20).is_err());
    let mut truncated = slice.clone();
    truncated.rbsp.truncate(truncated.entropy_byte_offset + 2);
    assert!(fvid::codec::hevc_picture::decode_idr(&sps, &pps, &truncated, 1 << 20).is_err());
}

#[test]
fn detailed_idr_matches_pixel_oracle() {
    detailed_fixture(
        include_bytes!("fixtures/hevc/detail-sps.bin"),
        include_bytes!("fixtures/hevc/detail-pps.bin"),
        include_bytes!("fixtures/hevc/detail-idr.bin"),
        include_bytes!("fixtures/hevc/detail-oracle.yuv")
            .iter()
            .map(|&v| u16::from(v))
            .collect(),
        8,
    );
}

#[test]
fn detailed_idr10_matches_pixel_oracle() {
    detailed_fixture(
        include_bytes!("fixtures/hevc/detail10-sps.bin"),
        include_bytes!("fixtures/hevc/detail10-pps.bin"),
        include_bytes!("fixtures/hevc/detail10-idr.bin"),
        include_bytes!("fixtures/hevc/detail10-oracle.yuv")
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect(),
        10,
    );
}

#[test]
fn library_decodes_four_ctus_with_cross_row_prediction() {
    let sps = Sps::parse(include_bytes!("fixtures/hevc/multi-sps.bin"), 4096).unwrap();
    let pps = Pps::parse(include_bytes!("fixtures/hevc/multi-pps.bin"), &sps, 4096).unwrap();
    let slice = SliceHeader::parse_idr(
        include_bytes!("fixtures/hevc/multi-idr.bin"),
        &sps,
        &pps,
        16384,
    )
    .unwrap();
    assert_eq!(sps.dimensions, [64, 64]);
    assert_eq!(sps.coding_block_log2[1], 5);
    let picture = fvid::codec::hevc_picture::decode_idr(&sps, &pps, &slice, 1 << 20).unwrap();
    let actual: Vec<_> = picture
        .planes
        .iter()
        .flat_map(|p| p.samples().iter().copied())
        .collect();
    let expected = include_bytes!("fixtures/hevc/multi-oracle.yuv");
    assert_eq!(actual.len(), expected.len());
    let errors: Vec<_> = actual
        .iter()
        .zip(expected)
        .enumerate()
        .filter(|(_, (a, b))| **a != u16::from(**b))
        .take(12)
        .collect();
    assert!(errors.is_empty(), "pixel differences: {errors:?}");
}

#[test]
fn library_applies_sao_to_four_ctu_picture() {
    let sps = Sps::parse(include_bytes!("fixtures/hevc/sao-sps.bin"), 4096).unwrap();
    let pps = Pps::parse(include_bytes!("fixtures/hevc/sao-pps.bin"), &sps, 4096).unwrap();
    let slice = SliceHeader::parse_idr(
        include_bytes!("fixtures/hevc/sao-idr.bin"),
        &sps,
        &pps,
        16384,
    )
    .unwrap();
    assert_eq!(slice.sao, [true, true]);
    assert_eq!(sps.dimensions, [64, 64]);
    assert_eq!(sps.coding_block_log2[1], 5);
    let picture = fvid::codec::hevc_picture::decode_idr(&sps, &pps, &slice, 1 << 20).unwrap();
    assert!(picture.sao.iter().flatten().any(|p| match p {
        hevc_sao::Sao::Off => false,
        hevc_sao::Sao::Band { offsets, .. } | hevc_sao::Sao::Edge { offsets, .. } =>
            offsets.iter().any(|&v| v != 0),
    }));
    let actual: Vec<_> = picture
        .planes
        .iter()
        .flat_map(|p| p.samples().iter().copied())
        .collect();
    let expected = include_bytes!("fixtures/hevc/sao-oracle.yuv");
    assert_eq!(actual.len(), expected.len());
    let errors: Vec<_> = actual
        .iter()
        .zip(expected)
        .enumerate()
        .filter(|(_, (a, b))| **a != u16::from(**b))
        .take(12)
        .collect();
    assert!(errors.is_empty(), "pixel differences: {errors:?}");
}
