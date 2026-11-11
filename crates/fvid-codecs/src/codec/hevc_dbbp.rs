//! Annex I DBBP contour construction and component composition (I-276..I-279).
use super::super::{hevc_inter_syntax::Partition, hevc_residual::ResidualBins};
use crate::{Result, invalid};

#[derive(Clone, Copy)]
pub struct CuSyntax {
    pub inter: bool,
    pub skipped: bool,
    pub enabled: bool,
    pub disparity_available: bool,
    pub log2_size: u8,
    pub partition: Partition,
}
/// Called at cu_extension, after both PUs' coded prediction syntax.
pub fn read_flag(bins: &mut impl ResidualBins, cu: CuSyntax) -> Result<bool> {
    if !(3..=6).contains(&cu.log2_size) {
        return Err(invalid("invalid HEVC DBBP CU size"));
    }
    if !cu.inter
        || cu.skipped
        || !cu.enabled
        || !cu.disparity_available
        || cu.log2_size == 3
        || !matches!(cu.partition, Partition::Horizontal | Partition::Vertical)
    {
        return Ok(false);
    }
    bins.decision(super::super::hevc_cabac::Syntax::DepthBasedPartition, 0)
}

pub struct Contour {
    size: usize,
    threshold: u16,
    labels: Vec<u8>,
}
impl Contour {
    /// `depth` is the padded full-resolution partIdc=1 sample array, raster order.
    pub fn from_depth(depth: &[u16], size: usize, budget: usize) -> Result<Self> {
        if !matches!(size, 16 | 32 | 64) || size.checked_mul(size) != Some(depth.len()) {
            return Err(invalid("invalid HEVC DBBP depth contour geometry"));
        }
        let required = std::mem::size_of::<Self>()
            .checked_add(depth.len())
            .ok_or_else(|| invalid("HEVC DBBP contour storage overflow"))?;
        if required > budget {
            return Err(invalid("HEVC DBBP contour exceeds budget"));
        }
        let threshold = ((u32::from(depth[0])
            + u32::from(depth[size - 1])
            + u32::from(depth[(size - 1) * size])
            + u32::from(depth[size * size - 1]))
            >> 2) as u16;
        let mut labels = Vec::new();
        labels
            .try_reserve_exact(depth.len())
            .map_err(|_| invalid("cannot allocate HEVC DBBP contour"))?;
        if labels
            .capacity()
            .checked_add(std::mem::size_of::<Self>())
            .is_none_or(|n| n > budget)
        {
            return Err(invalid("HEVC DBBP contour capacity exceeds budget"));
        }
        labels.extend(depth.iter().map(|&v| u8::from(v > threshold)));
        Ok(Self {
            size,
            threshold,
            labels,
        })
    }
    pub fn from_plane(
        plane: &super::super::hevc_plane::Plane,
        origin: [u32; 2],
        size: usize,
        disparity: [i64; 2],
        budget: usize,
    ) -> Result<Self> {
        let dims = plane.dimensions();
        if dims.contains(&0)
            || dims[0].checked_mul(dims[1]) != Some(plane.samples().len())
            || !plane.complete()
        {
            return Err(invalid("HEVC DBBP reference depth plane is incomplete"));
        }
        if !matches!(size, 16 | 32 | 64) {
            return Err(invalid("invalid HEVC DBBP depth block size"));
        }
        let base = [0, 1].map(|a| i128::from(origin[a]) + ((i128::from(disparity[a]) + 2) >> 2));
        let at = |x: usize, y: usize| {
            let xx = (base[0] + x as i128).clamp(0, dims[0] as i128 - 1) as usize;
            let yy = (base[1] + y as i128).clamp(0, dims[1] as i128 - 1) as usize;
            plane.samples()[yy * dims[0] + xx]
        };
        let required = std::mem::size_of::<Self>() + size * size;
        if required > budget {
            return Err(invalid("HEVC DBBP contour exceeds budget"));
        }
        let threshold = ((u32::from(at(0, 0))
            + u32::from(at(size - 1, 0))
            + u32::from(at(0, size - 1))
            + u32::from(at(size - 1, size - 1)))
            >> 2) as u16;
        let mut labels = Vec::new();
        labels
            .try_reserve_exact(size * size)
            .map_err(|_| invalid("cannot allocate HEVC DBBP contour"))?;
        if labels.capacity() + std::mem::size_of::<Self>() > budget {
            return Err(invalid("HEVC DBBP contour capacity exceeds budget"));
        }
        for y in 0..size {
            for x in 0..size {
                labels.push(u8::from(at(x, y) > threshold));
            }
        }
        Ok(Self {
            size,
            threshold,
            labels,
        })
    }
    pub fn size(&self) -> usize {
        self.size
    }
    pub fn threshold(&self) -> u16 {
        self.threshold
    }
    pub fn labels(&self) -> &[u8] {
        &self.labels
    }
    pub fn metadata_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.labels.capacity()
    }
    /// Both PU predictors cover the complete CU component. Budget covers the
    /// two newly allocated component buffers; borrowed contour/predictors are
    /// charged by their owner. Chroma takes each subsampling cell's top-left bit.
    pub fn compose(
        &self,
        predictions: [&[u16]; 2],
        component_size: usize,
        partition: Partition,
        budget: usize,
    ) -> Result<Vec<u16>> {
        if !matches!(partition, Partition::Horizontal | Partition::Vertical)
            || ![self.size, self.size / 2].contains(&component_size)
        {
            return Err(invalid("invalid HEVC DBBP component geometry or partition"));
        }
        let count = component_size * component_size;
        if predictions.iter().any(|p| p.len() != count) {
            return Err(invalid("HEVC DBBP predictor sizes disagree"));
        }
        if count.checked_mul(4).is_none_or(|n| n > budget) {
            return Err(invalid("HEVC DBBP component exceeds budget"));
        }
        let scale = self.size / component_size;
        let flag = |x: usize, y: usize| self.labels[(y * scale) * self.size + x * scale];
        let mut combined = Vec::new();
        let mut output = Vec::new();
        combined
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate HEVC DBBP composite"))?;
        output
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate HEVC DBBP filtered output"))?;
        if combined
            .capacity()
            .checked_add(output.capacity())
            .and_then(|n| n.checked_mul(2))
            .is_none_or(|n| n > budget)
        {
            return Err(invalid("HEVC DBBP component capacities exceed budget"));
        }
        for y in 0..component_size {
            for x in 0..component_size {
                combined.push(
                    predictions[usize::from(flag(x, y) != flag(0, 0))][y * component_size + x],
                );
            }
        }
        let vertical = partition == Partition::Vertical;
        for y in 0..component_size {
            for x in 0..component_size {
                let (px, py, nx, ny) = if vertical {
                    (x.saturating_sub(1), y, (x + 1).min(component_size - 1), y)
                } else {
                    (x, y.saturating_sub(1), x, (y + 1).min(component_size - 1))
                };
                // Chroma's next contour sample can lie beyond the last component
                // sample's top-left site: clamp in luma coordinates as I-279 states.
                let previous = self.labels[(py * scale) * self.size + px * scale];
                let next = self.labels[((y * scale + if vertical { 0 } else { scale })
                    .min(self.size - 1))
                    * self.size
                    + (x * scale + if vertical { scale } else { 0 }).min(self.size - 1)];
                let center = combined[y * component_size + x];
                output.push(if previous != next {
                    ((u32::from(combined[py * component_size + px])
                        + 2 * u32::from(center)
                        + u32::from(combined[ny * component_size + nx]))
                        >> 2) as u16
                } else {
                    center
                });
            }
        }
        Ok(output)
    }
}

pub struct Prediction {
    pub contour: Contour,
    pub motions: [super::Motion; 2],
    pub rect: [u32; 4],
    pub partition: Partition,
}
impl Prediction {
    pub fn predict(
        &self,
        lists: &[Vec<super::Reference>; 2],
        component: usize,
        depth: u8,
        weights: Option<&super::super::hevc_slice::Weights>,
        scratch: &mut Vec<i32>,
        format: u8,
        current: (&[super::super::hevc_plane::Plane; 3], [u8; 2]),
        budget: usize,
    ) -> Result<Vec<u16>> {
        if component > 2
            || format > 3
            || format == 0 && component != 0
            || self.rect[2] != self.rect[3]
            || self.rect[2] as usize != self.contour.size()
        {
            return Err(invalid("invalid HEVC DBBP prediction representation"));
        }
        let shifts = super::component_shifts(component, format);
        if shifts[0] != shifts[1] {
            return Err(invalid("HEVC DBBP requires square component sampling"));
        }
        let side = (self.rect[2] >> shifts[0]) as usize;
        if side * side * 8 > budget {
            return Err(invalid("HEVC DBBP prediction exceeds buffer budget"));
        }
        for motion in self.motions {
            if motion.iter().flatten().count() != 1 {
                return Err(invalid("HEVC DBBP predictor must be unidirectional"));
            }
        }
        let a = super::predict_with_current(
            lists,
            self.motions[0],
            self.rect,
            component,
            depth,
            weights,
            scratch,
            format,
            Some(current),
        )?;
        let b = super::predict_with_current(
            lists,
            self.motions[1],
            self.rect,
            component,
            depth,
            weights,
            scratch,
            format,
            Some(current),
        )?;
        let held = a
            .capacity()
            .checked_add(b.capacity())
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(|| invalid("HEVC DBBP predictor storage overflow"))?;
        self.contour.compose(
            [&a, &b],
            side,
            self.partition,
            budget
                .checked_sub(held)
                .ok_or_else(|| invalid("HEVC DBBP predictors exceed budget"))?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn depth_plane_padding_signed_rounding_and_extreme_disparities() {
        use super::super::super::hevc_plane::Plane;
        let mut plane=Plane::new(32,32,16,8192).unwrap();
        assert!(Contour::from_plane(&plane,[0,0],16,[0,0],8192).is_err());
        let pixels:Vec<u16>=(0..1024).map(|i| (i*61%65536)as u16).collect();
        plane.reconstruct_inter([0,0,32,32],&pixels).unwrap();
        for disparity in [[-3,-2],[-2,-3],[2,3],[i64::MIN,i64::MAX],[i64::MAX,i64::MIN]] {
            let origin=[7,9];
            let base=[0,1].map(|a|i128::from(origin[a])+((i128::from(disparity[a])+2)>>2));
            let mut padded=Vec::new();
            for y in 0..16 {for x in 0..16 {
                let xx=(base[0]+x).clamp(0,31)as usize;
                let yy=(base[1]+y).clamp(0,31)as usize;
                padded.push(pixels[yy*32+xx]);
            }}
            let expected=Contour::from_depth(&padded,16,8192).unwrap();
            let actual=Contour::from_plane(&plane,origin,16,disparity,8192).unwrap();
            assert_eq!(actual.threshold(),expected.threshold());
            assert_eq!(actual.labels(),expected.labels(),"{disparity:?}");
            assert!(Contour::from_plane(&plane,origin,16,disparity,actual.metadata_bytes()-1).is_err());
        }
    }
    #[test]
    fn contour_uses_wide_corner_mean_strict_threshold_and_checks_storage() {
        let max = vec![u16::MAX; 256];
        let contour = Contour::from_depth(&max, 16, 4096).unwrap();
        assert_eq!(contour.threshold(), u16::MAX);
        assert!(contour.labels().iter().all(|&v| v == 0));
        let mut data = vec![100u16; 256];
        data[0] = 0;
        data[15] = 100;
        data[240] = 100;
        data[255] = 200;
        let contour = Contour::from_depth(&data, 16, 4096).unwrap();
        assert_eq!(contour.threshold(), 100);
        assert_eq!(contour.labels()[1], 0);
        assert_eq!(contour.labels()[255], 1);
        assert!(Contour::from_depth(&data, 16, contour.metadata_bytes() - 1).is_err());
        assert!(Contour::from_depth(&data[..255], 16, 4096).is_err());
        assert!(Contour::from_depth(&[0; 64], 8, 4096).is_err());
    }
    #[test]
    fn top_left_is_part_zero_and_filter_reads_unmodified_component_samples() {
        for partition in [Partition::Vertical, Partition::Horizontal] {
            let depth: Vec<u16> = (0..256)
                .map(|i| {
                    if (if partition == Partition::Vertical {
                        i % 16
                    } else {
                        i / 16
                    }) < 8
                    {
                        200
                    } else {
                        0
                    }
                })
                .collect();
            let contour = Contour::from_depth(&depth, 16, 4096).unwrap();
            assert_eq!(contour.labels()[0], 1);
            for side in [16, 8] {
                let a = vec![0u16; side * side];
                let b = vec![100u16; side * side];
                let output = contour
                    .compose([&a, &b], side, partition, side * side * 4)
                    .unwrap();
                for y in 0..side {
                    for x in 0..side {
                        let at = if partition == Partition::Vertical {
                            x
                        } else {
                            y
                        };
                        let expected = if at < side / 2 - 1 {
                            0
                        } else if at == side / 2 - 1 {
                            25
                        } else if at == side / 2 {
                            75
                        } else {
                            100
                        };
                        assert_eq!(
                            output[y * side + x],
                            expected,
                            "{partition:?} side{side} {x},{y}"
                        );
                    }
                }
                assert!(
                    contour
                        .compose([&a, &b], side, partition, side * side * 4 - 1)
                        .is_err()
                );
            }
        }
    }
    #[test]
    fn last_odd_depth_site_controls_chroma_boundary_even_if_unselected_by_decimation() {
        for partition in [Partition::Vertical, Partition::Horizontal] {
            let depth: Vec<u16> = (0..256)
                .map(|i| {
                    if (if partition == Partition::Vertical {
                        i % 16
                    } else {
                        i / 16
                    }) == 15
                    {
                        200
                    } else {
                        0
                    }
                })
                .collect();
            let contour = Contour::from_depth(&depth, 16, 4096).unwrap();
            let a: Vec<u16> = (0..64)
                .map(|i| {
                    (if partition == Partition::Vertical {
                        i % 8
                    } else {
                        i / 8
                    }) as u16
                        * 4
                })
                .collect();
            let b = vec![100u16; 64];
            let output = contour.compose([&a, &b], 8, partition, 256).unwrap();
            for y in 0..8 {
                for x in 0..8 {
                    let at = if partition == Partition::Vertical {
                        x
                    } else {
                        y
                    };
                    assert_eq!(output[y * 8 + x], if at == 7 { 27 } else { at as u16 * 4 });
                }
            }
        }
    }
    #[test]
    fn syntax_only_reads_one_context_zero_bin_for_eligible_half_partitions() {
        struct Bins {
            calls: usize,
            value: bool,
        }
        impl ResidualBins for Bins {
            fn decision(
                &mut self,
                s: super::super::super::hevc_cabac::Syntax,
                inc: usize,
            ) -> Result<bool> {
                assert!(matches!(
                    s,
                    super::super::super::hevc_cabac::Syntax::DepthBasedPartition
                ));
                assert_eq!(inc, 0);
                self.calls += 1;
                Ok(self.value)
            }
            fn bypass(&mut self) -> Result<bool> {
                panic!("DBBP flag has no bypass bins")
            }
        }
        let base = CuSyntax {
            inter: true,
            skipped: false,
            enabled: true,
            disparity_available: true,
            log2_size: 4,
            partition: Partition::Vertical,
        };
        for cu in [
            CuSyntax {
                inter: false,
                ..base
            },
            CuSyntax {
                skipped: true,
                ..base
            },
            CuSyntax {
                enabled: false,
                ..base
            },
            CuSyntax {
                disparity_available: false,
                ..base
            },
            CuSyntax {
                log2_size: 3,
                ..base
            },
            CuSyntax {
                partition: Partition::Full,
                ..base
            },
            CuSyntax {
                partition: Partition::Left,
                ..base
            },
            CuSyntax {
                partition: Partition::Quarters,
                ..base
            },
        ] {
            let mut bins = Bins {
                calls: 0,
                value: true,
            };
            assert!(!read_flag(&mut bins, cu).unwrap());
            assert_eq!(bins.calls, 0);
        }
        for value in [false, true] {
            for partition in [Partition::Vertical, Partition::Horizontal] {
                let mut bins = Bins { calls: 0, value };
                assert_eq!(
                    read_flag(&mut bins, CuSyntax { partition, ..base }).unwrap(),
                    value
                );
                assert_eq!(bins.calls, 1);
            }
        }
    }
}
