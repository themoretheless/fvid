//! Bounded, unfiltered HEVC plane reconstruction with decoded-sample availability.
use super::hevc_intra::References;
use crate::{Result, invalid};
use std::sync::Arc;
#[derive(Clone)]
pub struct Plane {
    width: usize,
    height: usize,
    depth: u8,
    samples: Arc<Vec<u16>>,
    ready: Arc<Vec<bool>>,
}
impl Plane {
    pub(crate) fn absent(depth: u8) -> Self {
        Self {
            width: 0,
            height: 0,
            depth,
            samples: Arc::new(Vec::new()),
            ready: Arc::new(Vec::new()),
        }
    }
    pub(crate) fn ready_rect(&self, rect: [usize; 4]) -> bool {
        let [x, y, w, h] = rect;
        if w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|n| n > self.width)
            || y.checked_add(h).is_none_or(|n| n > self.height)
        {
            return false;
        }
        (y..y + h).all(|row| {
            self.ready[row * self.width + x..row * self.width + x + w]
                .iter()
                .all(|&r| r)
        })
    }
    pub fn dimensions(&self) -> [usize; 2] {
        [self.width, self.height]
    }

    /// Apply single-slice, single-tile SAO after reconstruction/deblocking.
    /// Neighbours always come from the unchanged input plane. Extra workspace
    /// is two bytes per sample and is accounted for by the picture decoder.
    pub fn apply_sao(&mut self, log2_ctu: u8, parameters: &[super::hevc_sao::Sao]) -> Result<()> {
        self.apply_sao_with_boundaries(log2_ctu, parameters, |_, _| true)
    }
    pub(crate) fn apply_sao_with_boundaries(
        &mut self,
        log2_ctu: u8,
        parameters: &[super::hevc_sao::Sao],
        available: impl FnMut([usize; 2], [usize; 2]) -> bool,
    ) -> Result<()> {
        self.apply_sao_with_exclusions(log2_ctu, parameters, available, |_| false)
    }
    pub(crate) fn apply_sao_with_exclusions(
        &mut self,
        log2_ctu: u8,
        parameters: &[super::hevc_sao::Sao],
        available: impl FnMut([usize; 2], [usize; 2]) -> bool,
        excluded: impl FnMut([usize; 2]) -> bool,
    ) -> Result<()> {
        self.apply_sao_rectangular([log2_ctu; 2], parameters, available, excluded)
    }
    pub(crate) fn apply_sao_rectangular(
        &mut self,
        log2_ctu: [u8; 2],
        parameters: &[super::hevc_sao::Sao],
        mut available: impl FnMut([usize; 2], [usize; 2]) -> bool,
        mut excluded: impl FnMut([usize; 2]) -> bool,
    ) -> Result<()> {
        if log2_ctu.iter().any(|v| !(3..=6).contains(v)) || !self.complete() {
            return Err(invalid("invalid SAO plane state"));
        }
        let [ctu_width, ctu_height] = log2_ctu.map(|v| 1usize << v);
        let columns = self.width.div_ceil(ctu_width);
        let rows = self.height.div_ceil(ctu_height);
        if columns.checked_mul(rows) != Some(parameters.len()) {
            return Err(invalid("invalid SAO parameter grid"));
        }
        use super::hevc_sao::Sao;
        // Validate once per CTU; reconstructed plane samples already obey depth.
        for &sao in parameters {
            sao.apply(0, None, self.depth)?;
        }
        let mut output = self.samples.as_ref().clone();
        let max = (1i32 << self.depth) - 1;
        for (index, &sao) in parameters.iter().enumerate() {
            let x0 = (index % columns) * ctu_width;
            let y0 = (index / columns) * ctu_height;
            let x1 = (x0 + ctu_width).min(self.width);
            let y1 = (y0 + ctu_height).min(self.height);
            match sao {
                Sao::Off => {}
                Sao::Band { position, offsets } => {
                    let mut bands = [0i32; 32];
                    for (i, &offset) in offsets.iter().enumerate() {
                        bands[(position as usize + i) & 31] = i32::from(offset);
                    }
                    for y in y0..y1 {
                        for x in x0..x1 {
                            if excluded([x, y]) {
                                continue;
                            }
                            let k = y * self.width + x;
                            let value = self.samples[k];
                            output[k] = (i32::from(value)
                                + bands[(value >> (self.depth - 5)) as usize])
                                .clamp(0, max) as u16;
                        }
                    }
                }
                Sao::Edge { offsets, .. } => {
                    let [a, b] = sao.neighbours()?.unwrap();
                    let offsets = [
                        i32::from(offsets[0]),
                        i32::from(offsets[1]),
                        0,
                        i32::from(offsets[2]),
                        i32::from(offsets[3]),
                    ];
                    let x0 = x0.max(usize::from(a[0] < 0 || b[0] < 0));
                    let y0 = y0.max(usize::from(a[1] < 0 || b[1] < 0));
                    let x1 = x1.min(self.width - usize::from(a[0] > 0 || b[0] > 0));
                    let y1 = y1.min(self.height - usize::from(a[1] > 0 || b[1] > 0));
                    let delta = |v: [i32; 2]| v[1] as isize * self.width as isize + v[0] as isize;
                    let da = delta(a);
                    let db = delta(b);
                    for y in y0..y1 {
                        for x in x0..x1 {
                            if excluded([x, y]) {
                                continue;
                            }
                            let neighbour = |v: [i32; 2]| {
                                [
                                    x.checked_add_signed(v[0] as isize).unwrap(),
                                    y.checked_add_signed(v[1] as isize).unwrap(),
                                ]
                            };
                            if !available([x, y], neighbour(a)) || !available([x, y], neighbour(b))
                            {
                                continue;
                            }
                            let k = y * self.width + x;
                            let value = i32::from(self.samples[k]);
                            let a = i32::from(self.samples[k.checked_add_signed(da).unwrap()]);
                            let b = i32::from(self.samples[k.checked_add_signed(db).unwrap()]);
                            let category =
                                ((value - a).signum() + (value - b).signum() + 2) as usize;
                            output[k] = (value + offsets[category]).clamp(0, max) as u16;
                        }
                    }
                }
            }
        }
        self.samples = Arc::new(output);
        Ok(())
    }
    pub fn new(width: usize, height: usize, depth: u8, budget: usize) -> Result<Self> {
        let count = width
            .checked_mul(height)
            .filter(|&n| n > 0 && n <= budget / 3)
            .ok_or_else(|| invalid("HEVC plane exceeds memory budget"))?;
        if !(8..=16).contains(&depth) {
            return Err(invalid("unsupported HEVC plane depth"));
        }
        Ok(Self {
            width,
            height,
            depth,
            samples: Arc::new(vec![0; count]),
            ready: Arc::new(vec![false; count]),
        })
    }
    pub(crate) fn reconstruct_inter(&mut self, rect: [usize; 4], prediction: &[u16]) -> Result<()> {
        let [x, y, w, h] = rect;
        if w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|v| v > self.width)
            || y.checked_add(h).is_none_or(|v| v > self.height)
            || w.checked_mul(h) != Some(prediction.len())
        {
            return Err(invalid("invalid HEVC inter block geometry"));
        }
        for j in 0..h {
            let start = (y + j) * self.width + x;
            let ready = &mut Arc::make_mut(&mut self.ready)[start..start + w];
            if ready.iter().fold(false, |seen, &v| seen | v) {
                return Err(invalid("overlapping HEVC inter prediction"));
            }
            Arc::make_mut(&mut self.samples)[start..start + w]
                .copy_from_slice(&prediction[j * w..(j + 1) * w]);
            ready.fill(true);
        }
        Ok(())
    }
    pub(crate) fn add_residual(
        &mut self,
        origin: [usize; 2],
        log: u8,
        residual: &[i32],
    ) -> Result<()> {
        let [x, y] = origin;
        let n = 1usize
            .checked_shl(u32::from(log))
            .ok_or_else(|| invalid("invalid HEVC residual size"))?;
        if x.checked_add(n).is_none_or(|v| v > self.width)
            || y.checked_add(n).is_none_or(|v| v > self.height)
            || n.checked_mul(n) != Some(residual.len())
        {
            return Err(invalid("invalid HEVC residual geometry"));
        }
        let max = (1i32 << self.depth) - 1;
        for j in 0..n {
            let start = (y + j) * self.width + x;
            if !self.ready[start..start + n]
                .iter()
                .fold(true, |ready, &v| ready & v)
            {
                return Err(invalid("HEVC residual has no prediction"));
            }
            for (pixel, &delta) in Arc::make_mut(&mut self.samples)[start..start + n]
                .iter_mut()
                .zip(&residual[j * n..(j + 1) * n])
            {
                *pixel = i32::from(*pixel).saturating_add(delta).clamp(0, max) as u16;
            }
        }
        Ok(())
    }
    pub fn samples(&self) -> &[u16] {
        &self.samples
    }
    pub(crate) fn storage_bytes(&self) -> Option<usize> {
        self.samples
            .capacity()
            .checked_mul(std::mem::size_of::<u16>())?
            .checked_add(self.ready.capacity())?
            .checked_add(2 * (std::mem::size_of::<Vec<u16>>() + 2 * std::mem::size_of::<usize>()))
    }
    pub(crate) fn samples_mut(&mut self) -> &mut [u16] {
        Arc::make_mut(&mut self.samples).as_mut_slice()
    }
    pub fn complete(&self) -> bool {
        self.ready.iter().fold(true, |ready, &v| ready & v)
    }
    /// `available` applies slice/tile/constrained-intra restrictions in addition
    /// to the plane's bounds and already-reconstructed sample checks.
    pub fn reconstruct_intra(
        &mut self,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        chroma: bool,
        strong: bool,
        residual: &[i32],
        pred_scratch: &mut Vec<u16>,
        available: impl Fn(usize, usize) -> bool,
    ) -> Result<()> {
        self.reconstruct_intra_with_reference_filtering(
            origin,
            log,
            mode,
            chroma,
            strong,
            true,
            residual,
            pred_scratch,
            available,
        )
    }

    pub fn reconstruct_intra_with_reference_filtering(
        &mut self,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        chroma: bool,
        strong: bool,
        filter_references: bool,
        residual: &[i32],
        pred_scratch: &mut Vec<u16>,
        available: impl Fn(usize, usize) -> bool,
    ) -> Result<()> {
        self.reconstruct_intra_with_filters(
            origin,
            log,
            mode,
            chroma,
            strong,
            filter_references,
            true,
            residual,
            pred_scratch,
            available,
        )
    }
    pub fn reconstruct_intra_with_filters(
        &mut self,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        chroma: bool,
        strong: bool,
        filter_references: bool,
        filter_boundary: bool,
        residual: &[i32],
        pred_scratch: &mut Vec<u16>,
        available: impl Fn(usize, usize) -> bool,
    ) -> Result<()> {
        self.reconstruct_intra_with_full_chroma_filters(
            origin,
            log,
            mode,
            chroma,
            false,
            strong,
            filter_references,
            filter_boundary,
            residual,
            pred_scratch,
            available,
        )
    }
    pub fn reconstruct_intra_with_full_chroma_filters(
        &mut self,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        chroma: bool,
        full_chroma: bool,
        strong: bool,
        filter_references: bool,
        filter_boundary: bool,
        residual: &[i32],
        pred_scratch: &mut Vec<u16>,
        available: impl Fn(usize, usize) -> bool,
    ) -> Result<()> {
        if !(2..=5).contains(&log) {
            return Err(invalid("invalid HEVC plane block size"));
        }
        let n = 1usize << log;
        let [x, y] = origin;
        if x.checked_add(n).is_none_or(|end| end > self.width)
            || y.checked_add(n).is_none_or(|end| end > self.height)
            || x % n != 0
            || y % n != 0
            || !(residual.is_empty() || residual.len() == n * n)
        {
            return Err(invalid("invalid HEVC plane block geometry"));
        }
        for yy in y..y + n {
            if self.ready[yy * self.width + x..yy * self.width + x + n]
                .iter()
                .fold(false, |seen, &v| seen | v)
            {
                return Err(invalid("HEVC block overlaps reconstructed samples"));
            }
        }
        let sample = |xx: Option<usize>, yy: Option<usize>| {
            let (xx, yy) = (xx?, yy?);
            if xx >= self.width
                || yy >= self.height
                || !self.ready[yy * self.width + xx]
                || !available(xx, yy)
            {
                None
            } else {
                Some(self.samples[yy * self.width + xx])
            }
        };
        let top: Vec<_> = (0..2 * n)
            .map(|i| sample(x.checked_add(i), y.checked_sub(1)))
            .collect();
        let left: Vec<_> = (0..2 * n)
            .map(|i| sample(x.checked_sub(1), y.checked_add(i)))
            .collect();
        let references = References::new(
            log,
            self.depth,
            sample(x.checked_sub(1), y.checked_sub(1)),
            &top,
            &left,
        )?;
        pred_scratch.resize(n * n, 0);
        references.predict_with_full_chroma_filters(
            mode,
            chroma,
            full_chroma,
            strong,
            filter_references,
            filter_boundary,
            pred_scratch,
        )?;
        let max = (1i32 << self.depth) - 1;
        for yy in 0..n {
            let start = (y + yy) * self.width + x;
            for (xx, (pixel, &prediction)) in Arc::make_mut(&mut self.samples)[start..start + n]
                .iter_mut()
                .zip(&pred_scratch[yy * n..(yy + 1) * n])
                .enumerate()
            {
                let delta = if residual.is_empty() {
                    0
                } else {
                    residual[yy * n + xx]
                };
                *pixel = i32::from(prediction).saturating_add(delta).clamp(0, max) as u16;
            }
            Arc::make_mut(&mut self.ready)[start..start + n].fill(true);
        }
        Ok(())
    }
}
#[cfg(test)]
mod shared_storage_tests {
    use super::*;

    #[test]
    fn cloned_plane_shares_storage_until_reconstruction_mutates_it() {
        let mut original = Plane::new(4, 4, 8, 1024).unwrap();
        let held = original.clone();
        assert!(Arc::ptr_eq(&original.samples, &held.samples));
        assert!(Arc::ptr_eq(&original.ready, &held.ready));
        original.reconstruct_inter([0, 0, 4, 4], &[77; 16]).unwrap();
        assert_eq!(held.samples(), &[0; 16]);
        assert!(!held.complete());
        assert_eq!(original.samples(), &[77; 16]);
        assert!(original.complete());
        assert!(!Arc::ptr_eq(&original.samples, &held.samples));
        assert!(!Arc::ptr_eq(&original.ready, &held.ready));
        let held = original.clone();
        original.add_residual([0, 0], 2, &[3; 16]).unwrap();
        assert_eq!(held.samples(), &[77; 16]);
        assert_eq!(original.samples(), &[80; 16]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangular_sao_matches_scalar_for_partial_ctus_and_boundaries() {
        use super::super::hevc_sao::Sao;
        for depth in [8, 10, 12] {
            let (width, height) = (19usize, 35usize);
            let samples: Vec<u16> = (0..width * height)
                .map(|i| ((i * 73 + i / width * 31) & ((1 << depth) - 1)) as u16)
                .collect();
            for class in 0..5 {
                let parameters: Vec<_> = (0..9)
                    .map(|index| {
                        if class == 4 {
                            Sao::Band {
                                position: (index * 3) as u8,
                                offsets: [2, -2, 1, -1],
                            }
                        } else {
                            Sao::Edge {
                                class,
                                offsets: [3, 1, -1, -3],
                            }
                        }
                    })
                    .collect();
                let mut p = Plane::new(width, height, depth, width * height * 3).unwrap();
                Arc::make_mut(&mut p.samples).clone_from(&samples);
                Arc::make_mut(&mut p.ready).fill(true);
                let mut expected = samples.clone();
                for y in 0..height {
                    for x in 0..width {
                        if x == 4 {
                            continue;
                        }
                        let mode = parameters[(y / 16) * 3 + x / 8];
                        let neighbours = if let Some([a, b]) = mode.neighbours().unwrap() {
                            let point = |v: [i32; 2]| -> Option<[usize; 2]> {
                                let nx = x.checked_add_signed(v[0] as isize)?;
                                let ny = y.checked_add_signed(v[1] as isize)?;
                                (nx < width && ny < height && ny / 16 == y / 16).then_some([nx, ny])
                            };
                            let (Some(a), Some(b)) = (point(a), point(b)) else {
                                continue;
                            };
                            Some([samples[a[1] * width + a[0]], samples[b[1] * width + b[0]]])
                        } else {
                            None
                        };
                        expected[y * width + x] = mode
                            .apply(samples[y * width + x], neighbours, depth)
                            .unwrap();
                    }
                }
                p.apply_sao_rectangular(
                    [3, 4],
                    &parameters,
                    |a, b| a[1] / 16 == b[1] / 16,
                    |p| p[0] == 4,
                )
                .unwrap();
                assert_eq!(p.samples(), expected);
                let saved = p.samples.clone();
                assert!(
                    p.apply_sao_rectangular([3, 4], &parameters[..8], |_, _| true, |_| false)
                        .is_err()
                );
                assert_eq!(p.samples, saved);
            }
        }
    }
    #[test]
    fn sao_preserves_protected_samples_but_uses_them_as_neighbours() {
        use super::super::hevc_sao::Sao;
        let samples: Vec<u16> = (0..64).map(|i| if i % 2 == 0 { 10 } else { 20 }).collect();
        for (mode, expected) in [
            (
                Sao::Edge {
                    class: 0,
                    offsets: [7, 0, 0, -7],
                },
                [10, 20, 10, 20, 17, 13, 17, 20],
            ),
            (
                Sao::Band {
                    position: 1,
                    offsets: [2, 0, 0, 0],
                },
                [10, 20, 10, 20, 12, 20, 12, 20],
            ),
        ] {
            let mut plane = Plane::new(8, 8, 8, 4096).unwrap();
            plane.reconstruct_inter([0, 0, 8, 8], &samples).unwrap();
            plane
                .apply_sao_with_exclusions(3, &[mode], |_, _| true, |p| p[0] < 4)
                .unwrap();
            for row in plane.samples().chunks_exact(8) {
                assert_eq!(row, expected);
            }
        }
    }
    #[test]
    fn residual_addition_matches_wide_clipping_at_integer_limits() {
        let residual = [
            i32::MIN,
            i32::MAX,
            -2048,
            2048,
            -128,
            127,
            0,
            1,
            -1,
            255,
            -255,
            1023,
            -1023,
            32,
            -32,
            42,
        ];
        for depth in [8, 10, 12] {
            let prediction = 1u16 << (depth - 1);
            let expected: Vec<u16> = residual
                .iter()
                .map(|&v| {
                    (i64::from(prediction) + i64::from(v)).clamp(0, (1i64 << depth) - 1) as u16
                })
                .collect();
            let mut inter = Plane::new(4, 4, depth, 48).unwrap();
            inter
                .reconstruct_inter([0, 0, 4, 4], &[prediction; 16])
                .unwrap();
            inter.add_residual([0, 0], 2, &residual).unwrap();
            assert_eq!(inter.samples(), expected);
            let mut intra = Plane::new(4, 4, depth, 48).unwrap();
            let mut pred_scratch = Vec::new();
            intra
                .reconstruct_intra(
                    [0, 0],
                    2,
                    1,
                    false,
                    false,
                    &residual,
                    &mut pred_scratch,
                    |_, _| true,
                )
                .unwrap();
            assert_eq!(intra.samples(), expected);
        }
    }
    #[test]
    fn sao_uses_original_neighbours_and_commits_only_valid_output() {
        use super::super::hevc_sao::Sao;
        let mut p = Plane::new(4, 4, 8, 48).unwrap();
        p.samples = Arc::new([100, 101, 100, 99].repeat(4));
        Arc::make_mut(&mut p.ready).fill(true);
        p.apply_sao(
            3,
            &[Sao::Edge {
                class: 0,
                offsets: [7, 3, -3, -7],
            }],
        )
        .unwrap();
        assert_eq!(p.samples(), [100, 94, 100, 99].repeat(4));
        let saved = p.samples.clone();
        assert!(
            p.apply_sao(
                3,
                &[Sao::Band {
                    position: 32,
                    offsets: [0; 4]
                }]
            )
            .is_err()
        );
        assert_eq!(p.samples, saved);
        assert!(p.apply_sao(3, &[]).is_err());
    }
    #[test]
    fn published_blocks_feed_prediction_but_future_samples_do_not() {
        let mut p = Plane::new(8, 4, 8, 96).unwrap();
        let mut pred_scratch = Vec::new();
        p.reconstruct_intra(
            [0, 0],
            2,
            1,
            false,
            false,
            &[12; 16],
            &mut pred_scratch,
            |_, _| true,
        )
        .unwrap();
        p.reconstruct_intra(
            [4, 0],
            2,
            10,
            false,
            false,
            &[0; 16],
            &mut pred_scratch,
            |_, _| true,
        )
        .unwrap();
        assert!(p.complete());
        assert_eq!(p.samples(), [140; 32]);
        let saved = p.samples().to_vec();
        assert!(
            p.reconstruct_intra(
                [0, 0],
                2,
                1,
                false,
                false,
                &[0; 16],
                &mut pred_scratch,
                |_, _| true
            )
            .is_err()
        );
        assert_eq!(p.samples(), saved);
        assert!(Plane::new(8, 4, 8, 95).is_err());
    }
    #[test]
    fn unavailable_boundaries_and_extreme_residuals_are_safe() {
        let mut p = Plane::new(8, 4, 10, 96).unwrap();
        let mut pred_scratch = Vec::new();
        p.reconstruct_intra(
            [0, 0],
            2,
            1,
            true,
            false,
            &[i32::MAX; 16],
            &mut pred_scratch,
            |_, _| true,
        )
        .unwrap();
        p.reconstruct_intra(
            [4, 0],
            2,
            10,
            true,
            false,
            &[0; 16],
            &mut pred_scratch,
            |_, _| false,
        )
        .unwrap();
        for row in p.samples().chunks_exact(8) {
            assert_eq!(&row[..4], &[1023; 4]);
            assert_eq!(&row[4..], &[512; 4]);
        }
    }
}

#[cfg(test)]
mod slice_filter_tests {
    use std::sync::Arc;
    #[test]
    fn sao_edge_neighbours_obey_independent_slice_boundaries() {
        use super::{super::hevc_sao::Sao, Plane};
        let samples: Vec<u16> = (0..16)
            .flat_map(|y| vec![if y % 2 == 0 { 200 } else { 100 }; 16])
            .collect();
        let mut plane = Plane::new(16, 16, 8, 768).unwrap();
        plane.samples = Arc::new(samples.clone());
        Arc::make_mut(&mut plane.ready).fill(true);
        let sao = Sao::Edge {
            class: 1,
            offsets: [1, 2, -1, -2],
        };
        plane
            .apply_sao_with_boundaries(3, &[sao; 4], |a, b| a[1] / 8 == b[1] / 8)
            .unwrap();
        for y in 0..16 {
            for x in 0..16 {
                let neighbours = if y == 0 || y == 7 || y == 8 || y == 15 {
                    None
                } else {
                    Some([samples[(y - 1) * 16 + x], samples[(y + 1) * 16 + x]])
                };
                assert_eq!(
                    plane.samples[y * 16 + x],
                    sao.apply(samples[y * 16 + x], neighbours, 8).unwrap()
                );
            }
        }
    }
}
