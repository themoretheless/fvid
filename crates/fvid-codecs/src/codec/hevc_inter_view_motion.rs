//! Annex I temporal inter-view MV scaling, I-125..I-134.
//! View distances use view_id_val, not ViewIdx or NAL layer identifiers.
use super::{Vector, scale, texture};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distance {
    ShortTerm(i32),
    LongTerm { view_difference: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    /// Includes wide retained DI; scaling clips only when required by the
    /// normative process, while equal/zero distances transfer it unchanged.
    QuarterSample,
    /// Input/output retain the shared quarter-sample storage representation;
    /// scaling and clipping operate on signed 16-bit integer depth motion.
    IntegerDepth,
}

/// I.8.5.3.2.6: mismatched long-term classifications make the candidate unavailable.
/// A zero source or target distance transfers the vector unchanged (I-129).
/// Ordinary Annex A scaling has different zero-distance rules and is separate.
pub fn temporal_vector(
    mv: [i64; 2],
    source: Distance,
    target: Distance,
    inter_view_scaling: bool,
    domain: Domain,
) -> Result<Option<[i64; 2]>> {
    let coded = match domain {
        Domain::QuarterSample => {
            mv
        }
        Domain::IntegerDepth => texture::integer_motion([Some(Vector { reference: 0, mv }), None])?
            [0]
        .unwrap()
        .samples
        .map(i64::from),
    };
    // view_id_len is u(4), so coded view identifiers have at most 15 bits.
    for distance in [source, target] {
        if let Distance::LongTerm { view_difference } = distance {
            if !(-32767..=32767).contains(&view_difference) {
                return Err(invalid(
                    "HEVC inter-view distance exceeds view identifier domain",
                ));
            }
        }
    }
    let (col_diff, curr_diff) = match (source, target) {
        (Distance::ShortTerm(a), Distance::ShortTerm(b)) => (a, b),
        (Distance::LongTerm { view_difference: a }, Distance::LongTerm { view_difference: b }) => {
            if inter_view_scaling {
                (a, b)
            } else {
                (0, 0)
            }
        }
        _ => return Ok(None),
    };
    let result = if col_diff == curr_diff || col_diff == 0 || curr_diff == 0 {
        coded
    } else {
        scale(coded, col_diff, curr_diff)?
    };
    Ok(Some(match domain {
        Domain::QuarterSample => result,
        Domain::IntegerDepth => result.map(|v| v * 4),
    }))
}

/// Bound Annex I context. Ordinary Annex A/F callers must not construct this.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    graph: &'a super::super::hevc_vps_extension::LayerDependencyPrefix,
    current_layer: u8,
    scaling: bool,
}
impl<'a> Context<'a> {
    pub fn from_layered(
        binding: &'a super::super::hevc_slice::LayeredSliceContext<'_>,
    ) -> Result<Option<Self>> {
        let Some(tools) = binding.sps.sps.three_d.as_ref() else {
            return Ok(None);
        };
        let graph = &binding
            .vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("HEVC inter-view motion requires VPS extension"))?
            .graph;
        let layer = binding.sps.layer_id;
        Ok(Some(Self::from_graph(
            graph,
            layer,
            tools.inter_view_mv_scaling[usize::from(binding.sps.sps.depth_layer)],
        )?))
    }
    pub(super) fn from_graph(
        graph: &'a super::super::hevc_vps_extension::LayerDependencyPrefix,
        current_layer: u8,
        raw_scaling: bool,
    ) -> Result<Self> {
        let view = graph.view_index(current_layer)?;
        Ok(Self {
            graph,
            current_layer,
            scaling: raw_scaling && view != 0,
        })
    }
    pub fn predict(
        self,
        mv: [i64; 2],
        origins: [u8; 2],
        references: [u8; 2],
        poc_distances: [i32; 2],
        long_term: [bool; 2],
        domain: Domain,
    ) -> Result<Option<[i64; 2]>> {
        if origins[1] != self.current_layer {
            return Err(invalid(
                "HEVC temporal motion current layer disagrees with binding",
            ));
        }
        let mut distances = [Distance::ShortTerm(0); 2];
        for i in 0..2 {
            // Validate every layer identity, even when view scaling is inactive.
            self.graph.view_index(origins[i])?;
            self.graph.view_index(references[i])?;
            distances[i] = if long_term[i] {
                let difference = if self.scaling {
                    i32::from(self.graph.view_id_value(origins[i])?)
                        - i32::from(self.graph.view_id_value(references[i])?)
                } else {
                    0
                };
                Distance::LongTerm {
                    view_difference: difference,
                }
            } else {
                Distance::ShortTerm(poc_distances[i])
            };
        }
        temporal_vector(mv, distances[0], distances[1], self.scaling, domain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn long(d: i32) -> Distance {
        Distance::LongTerm { view_difference: d }
    }
    #[test]
    fn signed_view_distances_round_clip_and_reverse_direction() {
        for (a, b, mv, wanted) in [
            (2, 4, [64, -32], [128, -64]),
            (2, -4, [64, -32], [-128, 64]),
            (1, 127, [32767, -32768], [32767, -32768]),
            (-128, 127, [32767, -32768], [-32511, 32512]),
            (127, 1, [32767, -32768], [256, -256]),
            (-1, 1, [1, -1], [-1, 1]),
            (3, 2, [4, -4], [3, -3]),
        ] {
            assert_eq!(
                temporal_vector(mv, long(a), long(b), true, Domain::QuarterSample).unwrap(),
                Some(wanted)
            );
        }
    }
    #[test]
    fn equal_and_zero_distances_bypass_before_clipping() {
        for (a, b) in [(0, 7), (7, 0), (0, 0), (32767, 32767)] {
            assert_eq!(
                temporal_vector(
                    [32767, -32768],
                    long(a),
                    long(b),
                    true,
                    Domain::QuarterSample
                )
                .unwrap(),
                Some([32767, -32768])
            );
        }
        for (a, b) in [(0, 7), (7, 0), (0, 0)] {
            assert_eq!(
                temporal_vector(
                    [11, -9],
                    Distance::ShortTerm(a),
                    Distance::ShortTerm(b),
                    true,
                    Domain::QuarterSample
                )
                .unwrap(),
                Some([11, -9])
            );
        }
        assert_eq!(
            temporal_vector([64, -32], long(2), long(4), false, Domain::QuarterSample).unwrap(),
            Some([64, -32])
        );
    }
    #[test]
    fn short_term_scales_poc_independently_of_inter_view_flag() {
        for enabled in [false, true] {
            assert_eq!(
                temporal_vector(
                    [64, -32],
                    Distance::ShortTerm(2),
                    Distance::ShortTerm(4),
                    enabled,
                    Domain::QuarterSample
                )
                .unwrap(),
                Some([128, -64])
            );
            assert_eq!(
                temporal_vector(
                    [64, -32],
                    long(2),
                    Distance::ShortTerm(4),
                    enabled,
                    Domain::QuarterSample
                )
                .unwrap(),
                None
            );
            assert_eq!(
                temporal_vector(
                    [64, -32],
                    Distance::ShortTerm(2),
                    long(4),
                    enabled,
                    Domain::QuarterSample
                )
                .unwrap(),
                None
            );
        }
    }
    #[test]
    fn depth_scales_in_integer_units_and_preserves_full_publication_range() {
        assert_eq!(
            temporal_vector([4, -4], long(3), long(2), true, Domain::IntegerDepth).unwrap(),
            Some([4, -4])
        );
        assert_eq!(
            temporal_vector(
                [32767 * 4, -32768 * 4],
                long(2),
                long(4),
                true,
                Domain::IntegerDepth
            )
            .unwrap(),
            Some([32767 * 4, -32768 * 4])
        );
        assert_eq!(
            temporal_vector(
                [32767 * 4, -32768 * 4],
                long(0),
                long(4),
                true,
                Domain::IntegerDepth
            )
            .unwrap(),
            Some([32767 * 4, -32768 * 4])
        );
        assert!(temporal_vector([1, 0], long(2), long(4), true, Domain::IntegerDepth).is_err());
        assert!(
            temporal_vector([32768 * 4, 0], long(2), long(4), true, Domain::IntegerDepth).is_err()
        );
    }
    #[test]
    fn invalid_views_refuse_but_wide_unscaled_texture_vectors_are_retained() {
        for invalid_distance in [-32768, 32768] {
            assert!(
                temporal_vector(
                    [1, 0],
                    long(invalid_distance),
                    long(1),
                    true,
                    Domain::QuarterSample
                )
                .is_err()
            );
        }
        for mv in [-32769,32768,i64::MIN,i64::MAX] {
            assert_eq!(temporal_vector([mv,0],long(1),long(1),false,Domain::QuarterSample).unwrap(),Some([mv,0]));
            assert_eq!(temporal_vector([mv,0],long(1),long(2),true,Domain::QuarterSample).unwrap(),
                Some([if mv<0 {-32768}else{32767},0]));
        }
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    #[test]
    fn numeric_view_scaling_and_binding_validation() {
        use super::super::super::hevc_vps_extension::LayerDependencyPrefix;
        let mut dims = vec![[0; 16]; 3];
        dims[1][1] = 1;
        dims[2][1] = 2;
        let mut graph = LayerDependencyPrefix {
            base_additional_profile: None,
            layer_ids: vec![0, 3, 5],
            scalability_ids: dims,
            view_ids: vec![100, 102, 108],
            direct: vec![0; 3],
            transitive: vec![0; 3],
            tree_partitions: vec![],
            end_bit: 0,
        };
        let predict = |graph: &LayerDependencyPrefix, enabled| {
            Context::from_graph(graph, 5, enabled)
                .unwrap()
                .predict(
                    [64, -32],
                    [3, 5],
                    [0, 0],
                    [999, -999],
                    [true, true],
                    Domain::QuarterSample,
                )
                .unwrap()
        };
        assert_eq!(predict(&graph, true), Some([256, -128]));
        assert_eq!(predict(&graph, false), Some([64, -32]));
        graph.view_ids = vec![7, 7, 7];
        assert_eq!(predict(&graph, true), Some([64, -32]));
        graph.view_ids = vec![100, 102, 96];
        assert_eq!(predict(&graph, true), Some([-128, 64]));
        let base = Context::from_graph(&graph, 0, true).unwrap();
        assert_eq!(
            base.predict(
                [64, -32],
                [3, 0],
                [0, 3],
                [7, 9],
                [true, true],
                Domain::QuarterSample
            )
            .unwrap(),
            Some([64, -32])
        );
        assert!(
            base.predict(
                [64, -32],
                [3, 3],
                [0, 0],
                [7, 9],
                [true, true],
                Domain::QuarterSample
            )
            .is_err()
        );
        assert!(
            base.predict(
                [64, -32],
                [3, 0],
                [9, 3],
                [7, 9],
                [true, true],
                Domain::QuarterSample
            )
            .is_err()
        );
        assert!(Context::from_graph(&graph, 9, true).is_err());
    }
}
