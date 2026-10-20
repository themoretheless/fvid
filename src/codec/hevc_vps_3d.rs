//! Bounded Annex I VPS camera syntax, I.7.3.2.1.7 / I.7.4.3.1.7.
use super::{BitReader, LayerDependencyPrefix};
use crate::{Result, invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraPair {
    pub reference_view: u16,
    /// scale, offset, inverse-scale-plus-scale, inverse-offset-plus-offset.
    /// None means values are deferred to the corresponding slice headers.
    pub parameters: Option<[i32; 4]>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraView {
    pub view_index: u16,
    pub in_slice_header: bool,
    pub pairs: Vec<CameraPair>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreeDExtension {
    pub precision: u8,
    /// Sparse, in ViewOIdxList order, including the implicit empty base view.
    pub views: Vec<CameraView>,
    pub end_bit: usize,
}
struct Reader<'a> {
    bits: BitReader<'a>,
    end: usize,
}
impl Reader<'_> {
    fn take(&mut self, count: u8) -> Result<u32> {
        if self
            .bits
            .position()
            .checked_add(usize::from(count))
            .is_none_or(|p| p > self.end)
        {
            return Err(invalid("truncated HEVC VPS 3D extension"));
        }
        self.bits.read(count)
    }
    fn ue(&mut self, maximum: u32, message: &'static str) -> Result<u32> {
        let mut zeros = 0;
        while self.take(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err(invalid("oversized HEVC camera Golomb code"));
            }
        }
        let value = ((1u64 << zeros) - 1) + u64::from(self.take(zeros)?);
        if value > u64::from(maximum) {
            return Err(invalid(message));
        }
        Ok(value as u32)
    }
    fn se(&mut self) -> Result<i32> {
        let value = self.ue(u32::MAX, "HEVC camera parameter exceeds signed range")?;
        let magnitude = i32::try_from(value / 2 + (value & 1))
            .map_err(|_| invalid("HEVC camera parameter exceeds signed range"))?;
        Ok(if value & 1 == 0 {
            -magnitude
        } else {
            magnitude
        })
    }
}
impl ThreeDExtension {
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.views.capacity() * std::mem::size_of::<CameraView>()
            + self
                .views
                .iter()
                .map(|v| v.pairs.capacity() * std::mem::size_of::<CameraPair>())
                .sum::<usize>()
    }
    /// end_bit excludes RBSP stop/alignment, but may include extension3 syntax.
    /// The cursor is committed only when the entire camera payload is valid.
    pub fn read(
        bits: &mut BitReader<'_>,
        end_bit: usize,
        graph: &LayerDependencyPrefix,
        budget: usize,
    ) -> Result<Self> {
        let _ = graph.depth_layer_ids()?;
        if end_bit < bits.position() || end_bit > bits.position() + bits.remaining() {
            return Err(invalid("invalid HEVC VPS 3D syntax boundary"));
        }
        if graph.scalability_ids[0][1] != 0 {
            return Err(invalid("HEVC camera base view is not zero"));
        }
        let mut indices = [0u8; 63];
        let mut count = 0;
        for d in &graph.scalability_ids {
            if !indices[..count].contains(&d[1]) {
                indices[count] = d[1];
                count += 1;
            }
        }
        let minimum = std::mem::size_of::<Self>() + count * std::mem::size_of::<CameraView>();
        if minimum > budget {
            return Err(invalid("HEVC VPS camera metadata exceeds memory budget"));
        }
        let mut r = Reader {
            bits: bits.clone(),
            end: end_bit,
        };
        let precision = r.ue(5, "HEVC VPS camera precision exceeds range")? as u8;
        let mut views = Vec::new();
        views
            .try_reserve_exact(count)
            .map_err(|_| invalid("HEVC VPS camera allocation failed"))?;
        let mut used =
            std::mem::size_of::<Self>() + views.capacity() * std::mem::size_of::<CameraView>();
        if used > budget {
            return Err(invalid("HEVC VPS camera metadata exceeds memory budget"));
        }
        for (n, &view) in indices[..count].iter().enumerate() {
            let pairs_count = if n == 0 { 0 } else { r.take(6)? as usize };
            let in_slice_header = pairs_count > 0 && r.take(1)? != 0;
            let requested = pairs_count * std::mem::size_of::<CameraPair>();
            if used.checked_add(requested).is_none_or(|v| v > budget) {
                return Err(invalid("HEVC VPS camera metadata exceeds memory budget"));
            }
            let mut pairs: Vec<CameraPair> = Vec::new();
            pairs
                .try_reserve_exact(pairs_count)
                .map_err(|_| invalid("HEVC VPS camera allocation failed"))?;
            used += pairs.capacity() * std::mem::size_of::<CameraPair>();
            if used > budget {
                return Err(invalid("HEVC VPS camera metadata exceeds memory budget"));
            }
            for _ in 0..pairs_count {
                // cp_ref_voi is a 16-bit view index. The syntax does not require
                // it to belong to the current sparse ViewOIdxList.
                let reference = r.ue(65535, "HEVC VPS camera reference exceeds range")? as u16;
                if pairs.iter().any(|p| p.reference_view == reference) {
                    return Err(invalid("duplicate HEVC VPS camera reference"));
                }
                let parameters = if in_slice_header {
                    None
                } else {
                    Some([r.se()?, r.se()?, r.se()?, r.se()?])
                };
                pairs.push(CameraPair {
                    reference_view: reference,
                    parameters,
                });
            }
            views.push(CameraView {
                view_index: u16::from(view),
                in_slice_header,
                pairs,
            });
        }
        let result = Self {
            precision,
            views,
            end_bit: r.bits.position(),
        };
        *bits = r.bits;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn graph() -> LayerDependencyPrefix {
        let mut dims = vec![[0; 16]; 3];
        dims[1][1] = 1;
        dims[2][1] = 2;
        LayerDependencyPrefix {
            base_additional_profile: None,
            layer_ids: vec![0, 3, 5],
            scalability_ids: dims,
            view_ids: vec![100, 102, 108],
            direct: vec![0; 3],
            transitive: vec![0; 3],
            tree_partitions: vec![],
            end_bit: 0,
        }
    }
    #[test]
    fn truncations_and_budget_failure_preserve_nonzero_cursor() {
        // Three unrelated prefix bits, precision=0, two num_cp=0 fields.
        let bytes = [0xf0, 0];
        let graph = graph();
        for end in 3..16 {
            let mut bits = BitReader::new(&bytes);
            bits.skip(3).unwrap();
            assert!(ThreeDExtension::read(&mut bits, end, &graph, 4096).is_err());
            assert_eq!(bits.position(), 3);
        }
        let mut bits = BitReader::new(&bytes);
        bits.skip(3).unwrap();
        let camera = ThreeDExtension::read(&mut bits, 16, &graph, 4096).unwrap();
        assert_eq!(bits.position(), 16);
        assert_eq!(camera.precision, 0);
        assert_eq!(camera.views.len(), 3);
        assert!(camera.views.iter().all(|v| v.pairs.is_empty()));
        let required = camera.retained_bytes();
        let mut bits = BitReader::new(&bytes);
        bits.skip(3).unwrap();
        assert!(ThreeDExtension::read(&mut bits, 16, &graph, required - 1).is_err());
        assert_eq!(bits.position(), 3);
        ThreeDExtension::read(&mut bits, 16, &graph, required).unwrap();
    }
}

#[cfg(test)]
mod static_fixture_tests {
    use super::*;
    #[test]
    fn static_camera_payload_is_bounded_and_rolls_back_at_every_truncation() {
        let data = include_bytes!(
            "../../tests/fixtures/playback-errors/hevc-wedge/camera-parameters-static.bin"
        );
        let end = super::super::super::hevc_3d_syntax::payload_end(data).unwrap();
        let mut dims = vec![[0; 16]; 3];
        dims[1][1] = 1;
        dims[2][1] = 2;
        let graph = LayerDependencyPrefix {
            base_additional_profile: None,
            layer_ids: vec![0, 3, 5],
            scalability_ids: dims,
            view_ids: vec![100, 102, 108],
            direct: vec![0; 3],
            transitive: vec![0; 3],
            tree_partitions: vec![],
            end_bit: 0,
        };
        let mut bits = BitReader::new(data);
        let c = ThreeDExtension::read(&mut bits, end, &graph, 4096).unwrap();
        assert_eq!(c.precision, 5);
        assert_eq!(bits.position(), end);
        assert_eq!(c.views[2].pairs[1].parameters, Some([-11, 12, -13, 14]));
        let needed = c.retained_bytes();
        for budget in [0, needed - 1] {
            let mut bits = BitReader::new(data);
            assert!(ThreeDExtension::read(&mut bits, end, &graph, budget).is_err());
            assert_eq!(bits.position(), 0);
        }
        let mut bits = BitReader::new(data);
        ThreeDExtension::read(&mut bits, end, &graph, needed).unwrap();
        for cut in 0..end {
            let mut bits = BitReader::new(data);
            assert!(
                ThreeDExtension::read(&mut bits, cut, &graph, needed).is_err(),
                "cut {cut}"
            );
            assert_eq!(bits.position(), 0);
        }
    }
}
