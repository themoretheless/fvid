//! Base HEVC picture parameter sets, including tile geometry and filtering syntax.
use super::{bits::BitReader, hevc_nal::NalRbsp, hevc_scaling::ScalingLists, hevc_sps::Sps};
use crate::{Result, invalid};
fn ue(b: &mut BitReader<'_>, max: u32) -> Result<u32> {
    let n = b.unsigned_golomb()?;
    if n > max {
        return Err(invalid("HEVC PPS unsigned value exceeds range"));
    }
    Ok(n)
}
fn se(b: &mut BitReader<'_>, min: i32, max: i32) -> Result<i32> {
    let n = b.signed_golomb()?;
    if !(min..=max).contains(&n) {
        return Err(invalid("HEVC PPS signed value exceeds range"));
    }
    Ok(n)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tiles {
    pub column_widths: Vec<u32>,
    pub row_heights: Vec<u32>,
    pub loop_filter_across: bool,
}
fn tile_axis(b: &mut BitReader<'_>, extent: u32, count: u32, uniform: bool) -> Result<Vec<u32>> {
    let mut result = Vec::with_capacity(count as usize);
    let mut consumed = 0u32;
    for i in 0..count - 1 {
        let size = if uniform {
            ((u64::from(i + 1) * u64::from(extent)) / u64::from(count)
                - (u64::from(i) * u64::from(extent)) / u64::from(count)) as u32
        } else {
            ue(b, extent - 1)? + 1
        };
        consumed = consumed
            .checked_add(size)
            .ok_or_else(|| invalid("HEVC tile extent overflow"))?;
        if size == 0 || consumed >= extent {
            return Err(invalid("HEVC tiles exceed picture extent"));
        }
        result.push(size);
    }
    result.push(extent - consumed);
    Ok(result)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Deblocking {
    pub override_enabled: bool,
    pub disabled: bool,
    pub offsets_div2: [i8; 2],
}
/// H.265 7.3.2.3.2 bounded syntax; index zero's inferred [0,0] is not stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChromaQpOffsetList {
    pub depth: u8,
    pub entries: Vec<[i8; 2]>,
}
fn read_chroma_qp_list(b: &mut BitReader<'_>, max_depth: u8) -> Result<ChromaQpOffsetList> {
    let depth = ue(b, u32::from(max_depth))? as u8;
    let count = ue(b, 5)? as usize + 1;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push([se(b, -12, 12)? as i8, se(b, -12, 12)? as i8]);
    }
    Ok(ChromaQpOffsetList { depth, entries })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pps {
    pub id: u8,
    pub sps_id: u8,
    pub dependent_slices: bool,
    pub output_flag_present: bool,
    pub extra_slice_header_bits: u8,
    pub sign_data_hiding: bool,
    pub cabac_init_present: bool,
    pub default_references: [u8; 2],
    pub initial_qp: i32,
    pub constrained_intra: bool,
    pub transform_skip: bool,
    pub transform_skip_max_log2: u8,
    pub sao_offset_scale: [u8; 2],
    pub chroma_qp_offset_list: Option<ChromaQpOffsetList>,
    pub cu_qp_delta_depth: Option<u8>,
    pub chroma_qp_offsets: [i8; 2],
    pub slice_chroma_qp_offsets: bool,
    pub weighted_prediction: bool,
    pub weighted_biprediction: bool,
    pub transquant_bypass: bool,
    pub cross_component_prediction: bool,
    pub entropy_sync: bool,
    pub tiles: Option<Tiles>,
    pub loop_filter_across_slices: bool,
    pub deblocking: Deblocking,
    pub scaling_lists: Option<ScalingLists>,
    pub lists_modification: bool,
    #[cfg(test)]
    pub(crate) lists_modification_bit: usize,
    pub parallel_merge_log2: u8,
    pub slice_header_extension: bool,
    pub scc_extension: bool,
    /// None inherits the SPS; Some(empty) explicitly clears initial entries.
    pub palette_initial: Option<Vec<[u16; 3]>>,
    pub current_picture_reference: bool,
    pub adaptive_colour_transform: bool,
    pub slice_act_qp_offsets: bool,
    pub act_qp_offsets: [i8; 3],
}
impl Pps {
    pub fn parse(nal: &[u8], sps: &Sps, budget: usize) -> Result<Self> {
        let rbsp = NalRbsp::parse(nal, budget)?;
        rbsp.header.require_base_layer()?;
        if rbsp.header.unit_type != 34 {
            return Err(invalid("expected HEVC PPS NAL"));
        }
        let b = &mut BitReader::new(&rbsp.bytes);
        let id = ue(b, 63)? as u8;
        let sps_id = ue(b, 15)? as u8;
        if sps_id != sps.id {
            return Err(invalid("HEVC PPS references another SPS"));
        }
        let dependent_slices = b.bit()?;
        let output_flag_present = b.bit()?;
        let extra_slice_header_bits = b.read(3)? as u8;
        let sign_data_hiding = b.bit()?;
        let cabac_init_present = b.bit()?;
        let default_references = [ue(b, 14)? as u8 + 1, ue(b, 14)? as u8 + 1];
        let initial_qp = se(b, -26 - 6 * (i32::from(sps.depth[0]) - 8), 25)? + 26;
        let constrained_intra = b.bit()?;
        let transform_skip = b.bit()?;
        let cu_qp_delta_depth = if b.bit()? {
            Some(ue(
                b,
                u32::from(sps.coding_block_log2[1] - sps.coding_block_log2[0]),
            )? as u8)
        } else {
            None
        };
        let chroma_qp_offsets = [se(b, -12, 12)? as i8, se(b, -12, 12)? as i8];
        let slice_chroma_qp_offsets = b.bit()?;
        let weighted_prediction = b.bit()?;
        let weighted_biprediction = b.bit()?;
        let transquant_bypass = b.bit()?;
        let tiles_enabled = b.bit()?;
        let entropy_sync = b.bit()?;
        let tiles = if tiles_enabled {
            let side = 1u32 << sps.coding_block_log2[1];
            let width = sps.dimensions[0].div_ceil(side);
            let height = sps.dimensions[1].div_ceil(side);
            let columns = ue(b, width - 1)? + 1;
            let rows = ue(b, height - 1)? + 1;
            if columns > 20 || rows > 22 {
                return Err(invalid("HEVC tile count exceeds supported level limits"));
            }
            let uniform = b.bit()?;
            Some(Tiles {
                column_widths: tile_axis(b, width, columns, uniform)?,
                row_heights: tile_axis(b, height, rows, uniform)?,
                loop_filter_across: b.bit()?,
            })
        } else {
            None
        };
        let loop_filter_across_slices = b.bit()?;
        let deblocking = if b.bit()? {
            let override_enabled = b.bit()?;
            let disabled = b.bit()?;
            let offsets_div2 = if disabled {
                [0, 0]
            } else {
                [se(b, -6, 6)? as i8, se(b, -6, 6)? as i8]
            };
            Deblocking {
                override_enabled,
                disabled,
                offsets_div2,
            }
        } else {
            Deblocking::default()
        };
        let scaling_lists = if b.bit()? {
            if !sps.scaling_lists_enabled {
                return Err(invalid("PPS scaling lists require SPS enablement"));
            }
            Some(ScalingLists::read(b)?)
        } else {
            None
        };
        #[cfg(test)]
        let lists_modification_bit = b.position();
        let lists_modification = b.bit()?;
        let parallel_merge_log2 = ue(b, u32::from(sps.coding_block_log2[1] - 2))? as u8 + 2;
        let slice_header_extension = b.bit()?;
        let mut transform_skip_max_log2 = 2;
        let mut sao_offset_scale = [0; 2];
        let mut chroma_qp_offset_list = None;
        let mut cross_component_prediction = false;
        let mut scc_extension = false;
        let mut future_extension = false;
        let mut palette_initial = None;
        let mut current_picture_reference = false;
        let mut adaptive_colour_transform = false;
        let mut slice_act_qp_offsets = false;
        let mut act_qp_offsets = [-5, -5, -3];
        if b.bit()? {
            let range = b.bit()?;
            let multilayer = b.bit()?;
            let three_d = b.bit()?;
            scc_extension = b.bit()?;
            future_extension = b.read(4)? != 0;
            if multilayer || three_d {
                return Err(crate::unsupported(
                    "HEVC multilayer/3D PPS extensions are not implemented",
                ));
            }
            if range {
                if transform_skip {
                    transform_skip_max_log2 =
                        ue(b, u32::from(sps.transform_block_log2[1] - 2))? as u8 + 2;
                }
                cross_component_prediction = b.bit()?;
                if cross_component_prediction
                    && (sps.chroma_format != 3 || sps.separate_colour_plane)
                {
                    return Err(invalid(
                        "HEVC cross-component prediction requires interleaved 4:4:4",
                    ));
                }
                if b.bit()? {
                    if sps.chroma_format == 0 || sps.separate_colour_plane {
                        return Err(invalid("HEVC chroma QP list requires chroma components"));
                    }
                    chroma_qp_offset_list = Some(read_chroma_qp_list(
                        b,
                        sps.coding_block_log2[1] - sps.coding_block_log2[0],
                    )?);
                }
                for component in 0..2 {
                    sao_offset_scale[component] =
                        ue(b, u32::from(sps.depth[component].saturating_sub(10)))? as u8;
                }
            }
            if scc_extension {
                current_picture_reference = b.bit()?;
                if current_picture_reference && !sps.current_picture_reference {
                    return Err(invalid(
                        "HEVC PPS current-picture reference requires SPS capability",
                    ));
                }
                adaptive_colour_transform = b.bit()?;
                if adaptive_colour_transform {
                    if sps.chroma_format != 3 || sps.separate_colour_plane {
                        return Err(invalid("HEVC ACT requires interleaved 4:4:4"));
                    }
                    slice_act_qp_offsets = b.bit()?;
                    for (value, bias) in act_qp_offsets.iter_mut().zip([5, 5, 3]) {
                        let offset = b
                            .signed_golomb()?
                            .checked_sub(bias)
                            .filter(|v| (-12..=12).contains(v))
                            .ok_or_else(|| invalid("HEVC ACT PPS QP offset outside range"))?;
                        *value = offset as i8;
                    }
                }
                if b.bit()? {
                    let count = ue(b, 128)?;
                    let mut entries = Vec::new();
                    if count > 0 {
                        let palette =
                            sps.palette
                                .as_ref()
                                .filter(|p| p.maximum > 0)
                                .ok_or_else(|| {
                                    invalid("HEVC PPS palette entries require SPS capability")
                                })?;
                        if count > u32::from(palette.predictor_maximum)
                            || count as usize * 6 > budget
                        {
                            return Err(invalid(
                                "HEVC PPS palette initializer count exceeds limit",
                            ));
                        }
                        let mono = b.bit()?;
                        if mono != (sps.chroma_format == 0) {
                            return Err(invalid("HEVC PPS palette chroma disagrees with SPS"));
                        }
                        let y = ue(b, 8)? as u8 + 8;
                        let c = if mono {
                            sps.depth[1]
                        } else {
                            ue(b, 8)? as u8 + 8
                        };
                        if [y, c] != sps.depth {
                            return Err(invalid("HEVC PPS palette depth disagrees with SPS"));
                        }
                        entries.resize(count as usize, [0; 3]);
                        for component in 0..if mono { 1 } else { 3 } {
                            for entry in &mut entries {
                                entry[component] =
                                    b.read(sps.depth[usize::from(component != 0)])? as u16;
                            }
                        }
                    }
                    palette_initial = Some(entries);
                }
            }
        }
        if future_extension {
            // H.265 7.4.3: future extension data does not affect the known
            // decoding process. Preserve validation of rbsp_trailing_bits.
            while b.more_rbsp_data() {
                b.bit()?;
            }
        }
        b.finish_rbsp()?;
        Ok(Self {
            id,
            sps_id,
            dependent_slices,
            output_flag_present,
            extra_slice_header_bits,
            sign_data_hiding,
            cabac_init_present,
            default_references,
            initial_qp,
            constrained_intra,
            transform_skip,
            transform_skip_max_log2,
            sao_offset_scale,
            chroma_qp_offset_list,
            cu_qp_delta_depth,
            chroma_qp_offsets,
            slice_chroma_qp_offsets,
            weighted_prediction,
            weighted_biprediction,
            transquant_bypass,
            cross_component_prediction,
            entropy_sync,
            tiles,
            loop_filter_across_slices,
            deblocking,
            scaling_lists,
            lists_modification,
            #[cfg(test)]
            lists_modification_bit,
            parallel_merge_log2,
            slice_header_extension,
            scc_extension,
            palette_initial,
            current_picture_reference,
            adaptive_colour_transform,
            slice_act_qp_offsets,
            act_qp_offsets,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_pps_and_truncation() {
        let hex =
            "42010101600000030090000003000003001ea020810596566924caf0168080000003008000000c84";
        let nal: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let sps = Sps::parse(&nal, 1024).unwrap();
        let data = [0x44, 1, 0xc1, 0x72, 0xb4, 0x22, 0x40];
        let pps = Pps::parse(&data, &sps, 1024).unwrap();
        assert_eq!(pps.id, 0);
        assert_eq!(pps.sps_id, 0);
        assert_eq!(pps.initial_qp, 26);
        assert_eq!(pps.default_references, [1, 1]);
        assert_eq!(pps.tiles, None);
        assert_eq!(pps.parallel_merge_log2, 2);
        for end in 0..data.len() {
            assert!(Pps::parse(&data[..end], &sps, 1024).is_err());
        }
    }
    #[test]
    fn uniform_and_explicit_tiles_partition_ctus() {
        assert_eq!(
            tile_axis(&mut BitReader::new(&[]), 7, 3, true).unwrap(),
            vec![2, 2, 3]
        );
        // column_width_minus1 values 0,2 -> sizes 1,3,3.
        assert_eq!(
            tile_axis(&mut BitReader::new(&[0xb0]), 7, 3, false).unwrap(),
            vec![1, 3, 3]
        );
        assert!(tile_axis(&mut BitReader::new(&[0x38]), 7, 2, false).is_err());
    }
}

#[cfg(test)]
mod chroma_qp_fixture_tests {
    use super::*;
    fn syntax(depth: u32, count_minus1: u32, entries: &[[i32; 2]]) -> Vec<u8> {
        fn ue(v: u32, bits: &mut Vec<bool>) {
            let v = v + 1;
            let width = 32 - v.leading_zeros();
            bits.extend(std::iter::repeat_n(false, (width - 1) as usize));
            bits.extend((0..width).rev().map(|i| v & (1 << i) != 0));
        }
        let mut bits = Vec::new();
        ue(depth, &mut bits);
        ue(count_minus1, &mut bits);
        for pair in entries {
            for &v in pair {
                ue(
                    if v > 0 {
                        (2 * v - 1) as u32
                    } else {
                        (-2 * v) as u32
                    },
                    &mut bits,
                );
            }
        }
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        bits.chunks_exact(8)
            .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b)))
            .collect()
    }
    #[test]
    fn chroma_qp_list_depth_count_offsets_and_truncation_are_bounded() {
        let entries = [[-12, 12], [-2, 3], [0, 0], [12, -12], [6, 6], [-1, -1]];
        let data = syntax(2, 5, &entries);
        let table = read_chroma_qp_list(&mut BitReader::new(&data), 2).unwrap();
        assert_eq!(table.depth, 2);
        assert_eq!(table.entries.len(), 6);
        assert_eq!(
            table
                .entries
                .iter()
                .map(|p| p.map(i32::from))
                .collect::<Vec<_>>(),
            entries
        );
        for data in [
            syntax(3, 0, &[[0, 0]]),
            syntax(0, 6, &[]),
            syntax(0, 0, &[[13, 0]]),
            syntax(0, 0, &[[0, -13]]),
        ] {
            assert!(read_chroma_qp_list(&mut BitReader::new(&data), 2).is_err());
        }
        for end in 0..data.len() {
            assert!(read_chroma_qp_list(&mut BitReader::new(&data[..end]), 2).is_err());
        }
    }
    #[test]
    fn active_chroma_qp_fixture_parses_table_and_decodes_selection() {
        let data = include_bytes!(
            "../../tests/fixtures/playback-errors/hevc-chroma-qp-list-active-rext8.mp4"
        );
        let mut input =
            crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default())
                .unwrap();
        let configuration = input.tracks()[0].configuration.clone();
        let config = super::super::config::HevcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(
            config
                .arrays
                .iter()
                .find(|a| a.nal_type == 33)
                .unwrap()
                .units[0],
            16 << 20,
        )
        .unwrap();
        let pps = Pps::parse(
            config
                .arrays
                .iter()
                .find(|a| a.nal_type == 34)
                .unwrap()
                .units[0],
            &sps,
            16 << 20,
        )
        .unwrap();
        let table = pps.chroma_qp_offset_list.as_ref().unwrap();
        assert_eq!(table.depth, 0);
        assert_eq!(table.entries, [[6, 6]]);
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let nal = super::super::config::NalUnits::new(&packet, config.length_size)
            .unwrap()
            .map(Result::unwrap)
            .find(|n| {
                super::super::hevc_nal::NalHeader::parse(n)
                    .unwrap()
                    .is_vcl()
            })
            .unwrap();
        let header =
            super::super::hevc_slice::SliceHeader::parse(nal, &sps, &pps, 16 << 20).unwrap();
        assert!(header.cu_chroma_qp_offset_enabled);
        let mut decoder =
            super::super::hevc_decoder::HevcDecoder::from_configuration(&configuration, 16 << 20)
                .unwrap();
        assert!(decoder.decode_packet(&packet).unwrap().is_some());
    }
    #[test]
    fn active_chroma_qp_fixtures_match_hm_pixels_and_reset() {
        for (data, expected, frames) in [
            (include_bytes!("../../tests/fixtures/playback-errors/hevc-chroma-qp-list-active-rext8.mp4").as_slice(),
             include_bytes!("../../tests/fixtures/playback-errors/hevc-chroma-qp-list-active-rext8.yuv").as_slice(), 1),
            (include_bytes!("../../tests/fixtures/playback-errors/hevc-chroma-qp-list-groups-filtered-rext8.mp4").as_slice(),
             include_bytes!("../../tests/fixtures/playback-errors/hevc-chroma-qp-list-groups-filtered-rext8.yuv").as_slice(), 3),
            (
                include_bytes!("../../tests/fixtures/playback-errors/hevc-chroma-qp-list-wpp-rext8.mp4").as_slice(),
                include_bytes!("../../tests/fixtures/playback-errors/hevc-chroma-qp-list-wpp-rext8.yuv").as_slice(),
                3,
            ),
            (
                include_bytes!("../../tests/fixtures/playback-errors/hevc-chroma-qp-list-slices-rext8.mp4").as_slice(),
                include_bytes!("../../tests/fixtures/playback-errors/hevc-chroma-qp-list-slices-rext8.yuv").as_slice(),
                3,
            ),
            (
                include_bytes!("../../tests/fixtures/playback-errors/hevc-chroma-qp-list-dependent-rext8.mp4").as_slice(),
                include_bytes!("../../tests/fixtures/playback-errors/hevc-chroma-qp-list-dependent-rext8.yuv").as_slice(),
                3,
            ),
        ] {
            let mut input = crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default()).unwrap();
            let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration, 16 << 20).unwrap();
            let mut packet = Vec::new();
            for pass in 0..2 {
                if pass != 0 { decoder.reset(); }
                let mut pixels = Vec::new();
                for frame in 0..frames {
                    input.read_packet(0, frame, &mut packet).unwrap();
                    let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                    pixels.extend(decoded.picture.planes.iter().flat_map(|p| p.samples().iter().map(|&v| u8::try_from(v).unwrap())));
                }
                assert_eq!(pixels, expected);
            }
        }
    }
    #[test]
    fn chroma_qp_partition_fixtures_exercise_the_named_entropy_paths() {
        for (data, wpp, dependent, segments) in [
            (
                include_bytes!(
                    "../../tests/fixtures/playback-errors/hevc-chroma-qp-list-wpp-rext8.mp4"
                )
                .as_slice(),
                true,
                false,
                1,
            ),
            (
                include_bytes!(
                    "../../tests/fixtures/playback-errors/hevc-chroma-qp-list-slices-rext8.mp4"
                )
                .as_slice(),
                false,
                false,
                4,
            ),
            (
                include_bytes!(
                    "../../tests/fixtures/playback-errors/hevc-chroma-qp-list-dependent-rext8.mp4"
                )
                .as_slice(),
                false,
                true,
                4,
            ),
        ] {
            let mut input = crate::container::mp4::Mp4Reader::open(
                std::io::Cursor::new(data),
                Default::default(),
            )
            .unwrap();
            let decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration,
                16 << 20,
            )
            .unwrap();
            let (_, pps) = decoder.parameters();
            assert_eq!(pps.entropy_sync, wpp);
            assert_eq!(pps.dependent_slices, dependent);
            assert_eq!(pps.chroma_qp_offset_list.as_ref().unwrap().depth, 1);
            let mut packet = Vec::new();
            for frame in 0..3 {
                input.read_packet(0, frame, &mut packet).unwrap();
                let headers = decoder.slice_headers(&packet).unwrap();
                assert_eq!(headers.len(), segments);
                for (index, header) in headers.iter().enumerate() {
                    assert!(header.cu_chroma_qp_offset_enabled);
                    assert_eq!(header.dependent, dependent && index != 0);
                    assert_eq!(header.entropy_substreams.len(), if wpp { 2 } else { 1 });
                    assert_eq!(header.address, if wpp { 0 } else { index as u32 });
                }
            }
        }
    }

    #[test]
    fn chroma_qp_high_depth_fixtures_match_every_hm_sample_and_reset() {
        for (data, expected, depth) in [
            (
                include_bytes!(
                    "../../tests/fixtures/playback-errors/hevc-chroma-qp-list-high10-rext10.mp4"
                )
                .as_slice(),
                include_bytes!(
                    "../../tests/fixtures/playback-errors/hevc-chroma-qp-list-high10-rext10.yuv"
                )
                .as_slice(),
                10,
            ),
            (
                include_bytes!(
                    "../../tests/fixtures/playback-errors/hevc-chroma-qp-list-high12-rext12.mp4"
                )
                .as_slice(),
                include_bytes!(
                    "../../tests/fixtures/playback-errors/hevc-chroma-qp-list-high12-rext12.yuv"
                )
                .as_slice(),
                12,
            ),
        ] {
            let mut input = crate::container::mp4::Mp4Reader::open(
                std::io::Cursor::new(data),
                Default::default(),
            )
            .unwrap();
            let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration,
                16 << 20,
            )
            .unwrap();
            assert_eq!(decoder.parameters().0.depth, [depth; 2]);
            assert_eq!(
                decoder
                    .parameters()
                    .1
                    .chroma_qp_offset_list
                    .as_ref()
                    .unwrap()
                    .depth,
                1
            );
            let mut packet = Vec::new();
            for pass in 0..2 {
                if pass != 0 {
                    decoder.reset();
                }
                let mut pixels = Vec::new();
                for frame in 0..3 {
                    input.read_packet(0, frame, &mut packet).unwrap();
                    assert!(
                        decoder
                            .slice_headers(&packet)
                            .unwrap()
                            .iter()
                            .all(|h| h.cu_chroma_qp_offset_enabled)
                    );
                    let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                    pixels.extend(
                        decoded
                            .picture
                            .planes
                            .iter()
                            .flat_map(|p| p.samples().iter().flat_map(|v| v.to_le_bytes())),
                    );
                }
                assert_eq!(pixels, expected, "RExt{depth} pass {pass}");
            }
        }
    }
}

#[cfg(test)]
mod pcm_fixture_tests {
    #[test]
    fn deep_pcm_streams_match_hm_with_active_pcm_and_restart() {
        macro_rules! fixture {
            ($stem:literal,$depth:literal,$pcm:literal,$mixed:literal,$mode:literal) => {
                (
                    include_bytes!(concat!(
                        "../../tests/fixtures/playback-errors/",
                        $stem,
                        ".mp4"
                    ))
                    .as_slice(),
                    include_bytes!(concat!(
                        "../../tests/fixtures/playback-errors/",
                        $stem,
                        ".yuv"
                    ))
                    .as_slice(),
                    $depth,
                    $pcm,
                    $mixed,
                    $mode,
                )
            };
        }
        for (data, oracle, depth, pcm_depth, mixed, mode) in [
            fixture!("hevc-pcm-444-deep-input8-rext14", 14, 8, false, 0),
            fixture!("hevc-pcm-444-deep-full-rext14", 14, 14, false, 0),
            fixture!("hevc-pcm-444-deep-mixed-rext14", 14, 14, true, 1),
            fixture!("hevc-pcm-444-deep-dependent-rext14", 14, 14, true, 2),
            fixture!("hevc-pcm-444-deep-parallel-rext14", 14, 14, true, 3),
            fixture!("hevc-pcm-444-deep-input8-rext16", 16, 8, false, 0),
            fixture!("hevc-pcm-444-deep-full-rext16", 16, 16, false, 0),
            fixture!("hevc-pcm-444-deep-mixed-rext16", 16, 16, true, 1),
            fixture!("hevc-pcm-444-deep-dependent-rext16", 16, 16, true, 2),
            fixture!("hevc-pcm-444-deep-parallel-rext16", 16, 16, true, 3),
        ] {
            let mut input = crate::container::mp4::Mp4Reader::open(
                std::io::Cursor::new(data),
                Default::default(),
            )
            .unwrap();
            let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration,
                16 << 20,
            )
            .unwrap();
            assert_eq!(input.tracks()[0].samples.len(), 3);
            assert_eq!(decoder.parameters().0.depth, [depth; 2]);
            assert_eq!(decoder.parameters().0.chroma_format, 3);
            let pcm = decoder.parameters().0.pcm.as_ref().unwrap();
            assert_eq!(pcm.depth, [pcm_depth; 2]);
            assert_eq!(pcm.loop_filter_disabled, mode != 1);
            assert_eq!(decoder.parameters().1.entropy_sync, mode == 3);
            assert!(decoder.parameters().0.extended_precision);
            assert!(decoder.parameters().0.cabac_bypass_alignment);
            for _ in 0..2 {
                let mut actual = Vec::new();
                for i in 0..3 {
                    let mut packet = Vec::new();
                    input.read_packet(0, i, &mut packet).unwrap();
                    if mode == 2 {
                        assert!(
                            decoder
                                .slice_headers(&packet)
                                .unwrap()
                                .iter()
                                .any(|h| h.dependent)
                        );
                    }
                    let frame = decoder.decode_packet(&packet).unwrap().unwrap();
                    assert!(
                        frame.picture.pcm_luma_samples > 0,
                        "depth {depth}, PCM {pcm_depth}, mode {mode}, frame {i}"
                    );
                    let count = frame.picture.planes[0].samples().len();
                    if mixed {
                        assert!(frame.picture.pcm_luma_samples < count);
                    } else {
                        assert_eq!(frame.picture.pcm_luma_samples, count);
                    }
                    for plane in &frame.picture.planes {
                        for v in plane.samples() {
                            actual.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                }
                assert_eq!(actual, oracle);
                decoder.reset();
            }
        }
    }

    #[test]
    fn monochrome_pcm_corpus_matches_hm_and_restarts() {
        macro_rules! fixture {
            ($stem:literal,$depth:literal,$mixed:literal) => {
                (
                    include_bytes!(concat!(
                        "../../tests/fixtures/playback-errors/",
                        $stem,
                        ".mp4"
                    ))
                    .as_slice(),
                    include_bytes!(concat!(
                        "../../tests/fixtures/playback-errors/",
                        $stem,
                        ".yuv"
                    ))
                    .as_slice(),
                    $depth,
                    $mixed,
                )
            };
        }
        for (data, expected, depth, mixed) in [
            fixture!("hevc-pcm-mono-active-rext8", 8, false),
            fixture!("hevc-pcm-mono-small8-rext8", 8, false),
            fixture!("hevc-pcm-mono-small16-rext8", 8, false),
            fixture!("hevc-pcm-mono-mixed-rext8", 8, true),
            fixture!("hevc-pcm-mono-filtered-rext8", 8, false),
            fixture!("hevc-pcm-mono-parallel-rext8", 8, true),
            fixture!("hevc-pcm-mono-high10-rext10", 10, false),
            fixture!("hevc-pcm-mono-high12-rext12", 12, false),
            fixture!("hevc-pcm-mono-full10-rext10", 10, false),
            fixture!("hevc-pcm-mono-full12-rext12", 12, false),
            fixture!("hevc-pcm-mono-wpp-rext8", 8, true),
            fixture!("hevc-pcm-mono-reference-rext8", 8, false),
            fixture!("hevc-pcm-mono-reference-wpp-rext8", 8, false),
            fixture!("hevc-pcm-mono-dependent-rext8", 8, true),
        ] {
            let mut input = crate::container::mp4::Mp4Reader::open(
                std::io::Cursor::new(data),
                Default::default(),
            )
            .unwrap();
            let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration,
                16 << 20,
            )
            .unwrap();
            assert_eq!(decoder.parameters().0.chroma_format, 0);
            assert_eq!(decoder.parameters().0.depth[0], depth);
            assert!(decoder.parameters().0.pcm.is_some());
            for _ in 0..2 {
                let mut pixels = Vec::new();
                for index in 0..input.tracks()[0].samples.len() {
                    let mut packet = Vec::new();
                    input.read_packet(0, index, &mut packet).unwrap();
                    let frame = decoder.decode_packet(&packet).unwrap().unwrap();
                    assert!(
                        frame.picture.planes[1..]
                            .iter()
                            .all(|p| p.samples().is_empty())
                    );
                    if index == 0 {
                        let count = frame.picture.planes[0].samples().len();
                        assert!(frame.picture.pcm_luma_samples > 0);
                        if mixed {
                            assert!(frame.picture.pcm_luma_samples < count);
                        } else {
                            assert_eq!(frame.picture.pcm_luma_samples, count);
                        }
                    }
                    for &v in frame.picture.planes[0].samples() {
                        if depth == 8 {
                            pixels.push(v as u8);
                        } else {
                            pixels.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                }
                assert_eq!(pixels.len(), expected.len());
                assert!(
                    pixels == expected,
                    "first mismatch {:?}",
                    pixels.iter().zip(expected).position(|(a, b)| a != b)
                );
                decoder.reset();
            }
        }
    }
    #[test]
    fn subsampled_and_full_chroma_pcm_corpus_matches_hm_and_restarts() {
        macro_rules! fixture {
            ($stem:literal,$depth:literal,$mixed:literal,$format:literal) => {
                (
                    include_bytes!(concat!(
                        "../../tests/fixtures/playback-errors/",
                        $stem,
                        ".mp4"
                    ))
                    .as_slice(),
                    include_bytes!(concat!(
                        "../../tests/fixtures/playback-errors/",
                        $stem,
                        ".yuv"
                    ))
                    .as_slice(),
                    $depth,
                    $mixed,
                    $format,
                )
            };
        }
        for (data, expected, depth, mixed, format) in [
            fixture!("hevc-pcm-422-active-rext8", 8, false, 2),
            fixture!("hevc-pcm-422-small8-rext8", 8, false, 2),
            fixture!("hevc-pcm-422-small16-rext8", 8, false, 2),
            fixture!("hevc-pcm-422-mixed-rext8", 8, true, 2),
            fixture!("hevc-pcm-422-filtered-rext8", 8, false, 2),
            fixture!("hevc-pcm-422-parallel-rext8", 8, true, 2),
            fixture!("hevc-pcm-422-high10-rext10", 10, false, 2),
            fixture!("hevc-pcm-422-high12-rext12", 12, false, 2),
            fixture!("hevc-pcm-422-full10-rext10", 10, false, 2),
            fixture!("hevc-pcm-422-full12-rext12", 12, false, 2),
            fixture!("hevc-pcm-422-wpp-rext8", 8, true, 2),
            fixture!("hevc-pcm-422-reference-rext8", 8, false, 2),
            fixture!("hevc-pcm-422-reference-wpp-rext8", 8, false, 2),
            fixture!("hevc-pcm-422-dependent-rext8", 8, true, 2),
            fixture!("hevc-pcm-444-active-rext8", 8, false, 3),
            fixture!("hevc-pcm-444-small8-rext8", 8, false, 3),
            fixture!("hevc-pcm-444-small16-rext8", 8, false, 3),
            fixture!("hevc-pcm-444-mixed-rext8", 8, true, 3),
            fixture!("hevc-pcm-444-filtered-rext8", 8, false, 3),
            fixture!("hevc-pcm-444-parallel-rext8", 8, true, 3),
            fixture!("hevc-pcm-444-high10-rext10", 10, false, 3),
            fixture!("hevc-pcm-444-high12-rext12", 12, false, 3),
            fixture!("hevc-pcm-444-full10-rext10", 10, false, 3),
            fixture!("hevc-pcm-444-full12-rext12", 12, false, 3),
            fixture!("hevc-pcm-444-wpp-rext8", 8, true, 3),
            fixture!("hevc-pcm-444-reference-rext8", 8, false, 3),
            fixture!("hevc-pcm-444-reference-wpp-rext8", 8, false, 3),
            fixture!("hevc-pcm-444-dependent-rext8", 8, true, 3),
        ] {
            let mut input = crate::container::mp4::Mp4Reader::open(
                std::io::Cursor::new(data),
                Default::default(),
            )
            .unwrap();
            let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration,
                16 << 20,
            )
            .unwrap();
            assert_eq!(decoder.parameters().0.chroma_format, format);
            assert_eq!(decoder.parameters().0.depth[0], depth);
            assert!(decoder.parameters().0.pcm.is_some());
            for _ in 0..2 {
                let mut pixels = Vec::new();
                for index in 0..input.tracks()[0].samples.len() {
                    let mut packet = Vec::new();
                    input.read_packet(0, index, &mut packet).unwrap();
                    let frame = decoder.decode_packet(&packet).unwrap().unwrap();
                    let dims = frame.picture.dimensions.map(|v| v as usize);
                    assert_eq!(
                        frame.picture.planes[1].dimensions(),
                        [dims[0] >> usize::from(format == 2), dims[1]]
                    );
                    assert_eq!(
                        frame.picture.planes[2].dimensions(),
                        frame.picture.planes[1].dimensions()
                    );
                    if index == 0 {
                        let count = frame.picture.planes[0].samples().len();
                        assert!(frame.picture.pcm_luma_samples > 0);
                        if mixed {
                            assert!(frame.picture.pcm_luma_samples < count);
                        } else {
                            assert_eq!(frame.picture.pcm_luma_samples, count);
                        }
                    }
                    for plane in &frame.picture.planes {
                        for &v in plane.samples() {
                            if depth == 8 {
                                pixels.push(v as u8);
                            } else {
                                pixels.extend_from_slice(&v.to_le_bytes());
                            }
                        }
                    }
                }
                assert_eq!(pixels.len(), expected.len());
                assert!(
                    pixels == expected,
                    "first mismatch {:?}",
                    pixels.iter().zip(expected).position(|(a, b)| a != b)
                );
                decoder.reset();
            }
        }
    }
    #[test]
    fn pcm_fixtures_match_hm_samples_restart_and_filter_policy() {
        macro_rules! fixture {
            ($stem:literal, $depth:literal, $mixed:literal, $pcm_depth:literal) => {
                (
                    include_bytes!(concat!(
                        "../../tests/fixtures/playback-errors/",
                        $stem,
                        ".mp4"
                    ))
                    .as_slice(),
                    include_bytes!(concat!(
                        "../../tests/fixtures/playback-errors/",
                        $stem,
                        ".yuv"
                    ))
                    .as_slice(),
                    $depth,
                    $mixed,
                    $pcm_depth,
                )
            };
        }
        for (data, expected, depth, mixed, pcm_depth) in [
            fixture!("hevc-pcm-active-rext8", 8, false, 8),
            fixture!("hevc-pcm-mixed-rext8", 8, true, 8),
            fixture!("hevc-pcm-filtered-rext8", 8, false, 8),
            fixture!("hevc-pcm-parallel-rext8", 8, true, 8),
            fixture!("hevc-pcm-high10-rext10", 10, false, 8),
            fixture!("hevc-pcm-high12-rext12", 12, false, 8),
            fixture!("hevc-pcm-full10-rext10", 10, false, 10),
            fixture!("hevc-pcm-full12-rext12", 12, false, 12),
            fixture!("hevc-pcm-wpp-rext8", 8, true, 8),
            fixture!("hevc-pcm-dependent-rext8", 8, true, 8),
        ] {
            let mut input = crate::container::mp4::Mp4Reader::open(
                std::io::Cursor::new(data),
                Default::default(),
            )
            .unwrap();
            let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration,
                16 << 20,
            )
            .unwrap();
            let (sps, pps) = decoder.parameters();
            assert_eq!(sps.chroma_format, 1);
            assert_eq!(sps.depth, [depth; 2]);
            assert!(!sps.separate_colour_plane && pps.tiles.is_none());
            assert!(pps.chroma_qp_offset_list.is_none());
            assert_eq!(sps.pcm.as_ref().unwrap().block_log2, [5, 5]);
            assert_eq!(sps.pcm.as_ref().unwrap().depth, [pcm_depth; 2]);
            let total = sps
                .dimensions
                .iter()
                .map(|&v| v as usize)
                .product::<usize>();
            let mut packet = Vec::new();
            input.read_packet(0, 0, &mut packet).unwrap();
            for pass in 0..2 {
                if pass != 0 {
                    decoder.reset();
                }
                let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                assert!(decoded.picture.pcm_luma_samples > 0);
                if mixed {
                    assert!(decoded.picture.pcm_luma_samples < total);
                } else {
                    assert_eq!(decoded.picture.pcm_luma_samples, total);
                }
                let pixels: Vec<_> = decoded
                    .picture
                    .planes
                    .iter()
                    .flat_map(|p| {
                        p.samples().iter().flat_map(|&v| {
                            if depth == 8 {
                                vec![u8::try_from(v).unwrap()]
                            } else {
                                v.to_le_bytes().to_vec()
                            }
                        })
                    })
                    .collect();
                assert_eq!(
                    pixels, expected,
                    "PCM {depth}-bit, mixed={mixed}, pass={pass}"
                );
            }
        }
    }
    #[test]
    fn pcm_reference_pictures_feed_inter_motion_with_and_without_wpp() {
        for (data, expected, wpp) in [
            (
                include_bytes!("../../tests/fixtures/playback-errors/hevc-pcm-reference-rext8.mp4")
                    .as_slice(),
                include_bytes!("../../tests/fixtures/playback-errors/hevc-pcm-reference-rext8.yuv")
                    .as_slice(),
                false,
            ),
            (
                include_bytes!(
                    "../../tests/fixtures/playback-errors/hevc-pcm-reference-wpp-rext8.mp4"
                )
                .as_slice(),
                include_bytes!(
                    "../../tests/fixtures/playback-errors/hevc-pcm-reference-wpp-rext8.yuv"
                )
                .as_slice(),
                true,
            ),
        ] {
            let mut input = crate::container::mp4::Mp4Reader::open(
                std::io::Cursor::new(data),
                Default::default(),
            )
            .unwrap();
            let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration,
                16 << 20,
            )
            .unwrap();
            assert_eq!(decoder.parameters().1.entropy_sync, wpp);
            let mut packet = Vec::new();
            for pass in 0..2 {
                if pass != 0 {
                    decoder.reset();
                }
                let mut pixels = Vec::new();
                for frame in 0..3 {
                    input.read_packet(0, frame, &mut packet).unwrap();
                    let headers = decoder.slice_headers(&packet).unwrap();
                    assert_eq!(headers.len(), 1);
                    assert_eq!(headers[0].entropy_substreams.len(), if wpp { 2 } else { 1 });
                    let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                    assert_eq!(decoded.poc, frame as i32);
                    assert_eq!(
                        decoded.picture.pcm_luma_samples,
                        if frame == 0 { 4096 } else { 0 }
                    );
                    if frame != 0 {
                        assert_eq!(
                            headers[0].slice_type,
                            super::super::hevc_cabac::SliceType::B
                        );
                        assert!(decoded.picture.motion.iter().flatten().any(Option::is_some));
                        if frame == 1 {
                            assert!(decoded.picture.motion.iter().any(|motion| {
                                motion.iter().enumerate().any(|(list, vector)| {
                                    vector.is_some_and(|v| {
                                        decoded.picture.reference_pocs[list][v.reference as usize]
                                            == 0
                                    })
                                })
                            }));
                        }
                    }
                    pixels.extend(
                        decoded
                            .picture
                            .planes
                            .iter()
                            .flat_map(|p| p.samples().iter().map(|&v| u8::try_from(v).unwrap())),
                    );
                }
                assert_eq!(pixels, expected, "PCM reference WPP={wpp}, pass={pass}");
            }
        }
    }
    #[test]
    fn small_pcm_coding_units_match_every_hm_sample() {
        for (data, expected, log) in [
            (
                include_bytes!("../../tests/fixtures/playback-errors/hevc-pcm-small8-rext8.mp4")
                    .as_slice(),
                include_bytes!("../../tests/fixtures/playback-errors/hevc-pcm-small8-rext8.yuv")
                    .as_slice(),
                3,
            ),
            (
                include_bytes!("../../tests/fixtures/playback-errors/hevc-pcm-small16-rext8.mp4")
                    .as_slice(),
                include_bytes!("../../tests/fixtures/playback-errors/hevc-pcm-small16-rext8.yuv")
                    .as_slice(),
                4,
            ),
        ] {
            let mut input = crate::container::mp4::Mp4Reader::open(
                std::io::Cursor::new(data),
                Default::default(),
            )
            .unwrap();
            let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration,
                16 << 20,
            )
            .unwrap();
            assert_eq!(
                decoder.parameters().0.pcm.as_ref().unwrap().block_log2,
                [log; 2]
            );
            let mut packet = Vec::new();
            input.read_packet(0, 0, &mut packet).unwrap();
            for pass in 0..2 {
                if pass != 0 {
                    decoder.reset();
                }
                let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                assert_eq!(decoded.picture.pcm_luma_samples, 4096);
                let pixels: Vec<_> = decoded
                    .picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples().iter().map(|&v| u8::try_from(v).unwrap()))
                    .collect();
                assert_eq!(pixels, expected, "PCM block log={log}, pass={pass}");
            }
        }
    }
}
