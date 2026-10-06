//! Bounded HEVC CTU raster/tile-scan mapping (6.5.1).
use super::hevc_pps::Tiles;
use crate::{Result, invalid};

#[derive(Debug, PartialEq, Eq)]
pub struct TileLayout {
    /// Indexed by raster CTU address.
    pub raster_to_tile_scan: Vec<u32>,
    /// Indexed by tile-scan CTU address.
    pub tile_scan_to_raster: Vec<u32>,
    /// Indexed by raster CTU address.
    pub tile_ids: Vec<u16>,
    /// First tile-scan CTU address of each tile.
    pub tile_starts: Vec<u32>,
    /// Tile rectangles in CTUs: x, y, width, height.
    pub rectangles: Vec<[u32; 4]>,
}
impl TileLayout {
    pub fn new(tiles: &Tiles, dimensions: [u32; 2], budget: usize) -> Result<Self> {
        let [width, height] = dimensions;
        if width == 0
            || height == 0
            || !(1..=20).contains(&tiles.column_widths.len())
            || !(1..=22).contains(&tiles.row_heights.len())
            || tiles.column_widths.iter().any(|&v| v == 0)
            || tiles.row_heights.iter().any(|&v| v == 0)
            || tiles
                .column_widths
                .iter()
                .map(|&v| u64::from(v))
                .sum::<u64>()
                != u64::from(width)
            || tiles.row_heights.iter().map(|&v| u64::from(v)).sum::<u64>() != u64::from(height)
        {
            return Err(invalid("invalid HEVC tile partition"));
        }
        let count = width
            .checked_mul(height)
            .ok_or_else(|| invalid("HEVC tile CTU count overflow"))?;
        let number = tiles.column_widths.len() * tiles.row_heights.len();
        let required = (count as usize)
            .checked_mul(10)
            .and_then(|v| number.checked_mul(20).and_then(|n| v.checked_add(n)))
            .ok_or_else(|| invalid("HEVC tile layout size overflow"))?;
        if required > budget {
            return Err(invalid("HEVC tile layout exceeds budget"));
        }
        let mut layout = Self {
            raster_to_tile_scan: vec![0; count as usize],
            tile_scan_to_raster: Vec::with_capacity(count as usize),
            tile_ids: vec![0; count as usize],
            tile_starts: Vec::with_capacity(number),
            rectangles: Vec::with_capacity(number),
        };
        let mut y = 0;
        for &h in &tiles.row_heights {
            let mut x = 0;
            for &w in &tiles.column_widths {
                let tile = layout.rectangles.len() as u16;
                layout
                    .tile_starts
                    .push(layout.tile_scan_to_raster.len() as u32);
                layout.rectangles.push([x, y, w, h]);
                for row in y..y + h {
                    for column in x..x + w {
                        let raster = row * width + column;
                        layout.raster_to_tile_scan[raster as usize] =
                            layout.tile_scan_to_raster.len() as u32;
                        layout.tile_ids[raster as usize] = tile;
                        layout.tile_scan_to_raster.push(raster);
                    }
                }
                x += w;
            }
            y += h;
        }
        Ok(layout)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asymmetric_tiles_visit_each_ctu_once_in_tile_scan_order() {
        let tiles = Tiles {
            column_widths: vec![1, 2],
            row_heights: vec![2, 1],
            loop_filter_across: false,
        };
        let layout = TileLayout::new(&tiles, [3, 3], 170).unwrap();
        assert_eq!(layout.tile_scan_to_raster, [0, 3, 1, 2, 4, 5, 6, 7, 8]);
        assert_eq!(layout.raster_to_tile_scan, [0, 2, 3, 1, 4, 5, 6, 7, 8]);
        assert_eq!(layout.tile_ids, [0, 1, 1, 0, 1, 1, 2, 3, 3]);
        assert_eq!(layout.tile_starts, [0, 2, 6, 7]);
        assert_eq!(
            layout.rectangles,
            [[0, 0, 1, 2], [1, 0, 2, 2], [0, 2, 1, 1], [1, 2, 2, 1]]
        );
        for (ts, &rs) in layout.tile_scan_to_raster.iter().enumerate() {
            assert_eq!(layout.raster_to_tile_scan[rs as usize], ts as u32);
        }
        assert!(
            TileLayout::new(&tiles, [3, 3], 169)
                .unwrap_err()
                .to_string()
                .contains("budget")
        );
    }
    #[test]
    fn invalid_geometry_and_overflow_refuse_before_allocation() {
        for tiles in [
            Tiles {
                column_widths: vec![],
                row_heights: vec![1],
                loop_filter_across: true,
            },
            Tiles {
                column_widths: vec![0, 1],
                row_heights: vec![1],
                loop_filter_across: true,
            },
            Tiles {
                column_widths: vec![2],
                row_heights: vec![1],
                loop_filter_across: true,
            },
        ] {
            assert!(TileLayout::new(&tiles, [1, 1], usize::MAX).is_err());
        }
        let tiles = Tiles {
            column_widths: vec![u32::MAX],
            row_heights: vec![2],
            loop_filter_across: false,
        };
        assert!(
            TileLayout::new(&tiles, [u32::MAX, 2], usize::MAX)
                .unwrap_err()
                .to_string()
                .contains("count overflow")
        );
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    const DATA: &[u8] =
        include_bytes!("../../tests/fixtures/playback-errors/hevc-tiles-two-columns-rext8.mp4");
    #[test]
    fn two_column_fixture_decodes_tiles_in_non_raster_order() {
        let mut input =
            crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(DATA), Default::default())
                .unwrap();
        let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
            &input.tracks()[0].configuration,
            16 << 20,
        )
        .unwrap();
        let (sps, pps) = decoder.parameters();
        assert_eq!(sps.chroma_format, 1);
        assert!(!sps.separate_colour_plane && sps.pcm.is_none());
        assert!(!pps.entropy_sync && pps.chroma_qp_offset_list.is_none());
        let tiles = pps.tiles.as_ref().unwrap();
        assert_eq!(tiles.column_widths, [1, 1]);
        assert_eq!(tiles.row_heights, [2]);
        assert!(!tiles.loop_filter_across);
        let layout = TileLayout::new(tiles, [2, 2], 80).unwrap();
        assert_eq!(layout.tile_scan_to_raster, [0, 2, 1, 3]);
        assert_eq!(layout.raster_to_tile_scan, [0, 2, 1, 3]);
        assert_eq!(layout.tile_ids, [0, 1, 0, 1]);
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let headers = decoder.slice_headers(&packet).unwrap();
        assert_eq!(headers.len(), 1);
        assert!(headers[0].first && headers[0].address == 0 && headers[0].nal.layer_id == 0);
        assert_eq!(headers[0].entropy_substreams.len(), 2);
        assert_eq!(headers[0].entry_point_offsets.len(), 1);
        assert!(decoder.decode_packet(&packet).unwrap().is_some());
    }

    #[test]
    fn tiled_fixtures_match_every_hm_sample_and_reset() {
        assert_ne!(
            include_bytes!("../../tests/fixtures/playback-errors/hevc-tiles-filtered-rext8.yuv"),
            include_bytes!(
                "../../tests/fixtures/playback-errors/hevc-tiles-cross-filtered-rext8.yuv"
            ),
            "the filter-boundary policy must affect this owned oracle"
        );
        macro_rules! fixture {
            ($stem:literal, $bits:literal) => {
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
                    $bits,
                    $stem,
                )
            };
        }
        for (data, expected, bits, name) in [
            fixture!("hevc-tiles-two-columns-rext8", 8),
            fixture!("hevc-tiles-filtered-rext8", 8),
            fixture!("hevc-tiles-cross-filtered-rext8", 8),
            fixture!("hevc-tiles-asymmetric-rext8", 8),
            fixture!("hevc-tiles-high10-rext10", 10),
            fixture!("hevc-tiles-high12-rext12", 12),
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
            assert_eq!(sps.depth, [bits; 2]);
            let tiles = pps.tiles.as_ref().unwrap();
            assert_eq!(tiles.loop_filter_across, name.contains("cross-filtered"));
            let asymmetric = name.contains("asymmetric");
            assert_eq!(
                tiles.column_widths,
                if asymmetric { vec![1, 2] } else { vec![1, 1] }
            );
            assert_eq!(
                tiles.row_heights,
                if asymmetric { vec![2, 1] } else { vec![2] }
            );
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
                    assert_eq!(
                        headers[0].entropy_substreams.len(),
                        if asymmetric { 4 } else { 2 }
                    );
                    let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                    assert_eq!(decoded.poc, frame as i32);
                    pixels.extend(decoded.picture.planes.iter().flat_map(|p| {
                        p.samples().iter().flat_map(|&v| {
                            if bits == 8 {
                                vec![u8::try_from(v).unwrap()]
                            } else {
                                v.to_le_bytes().to_vec()
                            }
                        })
                    }));
                }
                assert_eq!(pixels, expected, "{name}, pass={pass}");
            }
        }
    }
    #[test]
    fn tiled_picture_checks_stream_bounds_geometry_and_combined_budget() {
        let mut input =
            crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(DATA), Default::default())
                .unwrap();
        let decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
            &input.tracks()[0].configuration,
            16 << 20,
        )
        .unwrap();
        let (sps, pps) = decoder.parameters();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let header = decoder.slice_headers(&packet).unwrap().remove(0);
        let lists = [Vec::new(), Vec::new()];
        let decode_error = |pps, header, budget| {
            super::super::hevc_picture::decode(sps, pps, header, 0, &lists, budget)
                .err()
                .expect("invalid tiled input must refuse")
                .to_string()
        };
        let mut missing = header.clone();
        missing.entropy_substreams.pop();
        assert!(decode_error(pps, &missing, 16 << 20).contains("substream count"));
        let mut outside = header.clone();
        outside.entropy_substreams[1].end = outside.rbsp.len() + 1;
        assert!(decode_error(pps, &outside, 16 << 20).contains("substream bounds"));
        let mut short = header.clone();
        short.entropy_substreams[1].end = short.entropy_substreams[1].start + 1;
        assert!(super::super::hevc_picture::decode(sps, pps, &short, 0, &lists, 16 << 20).is_err());
        let mut geometry = pps.clone();
        geometry.tiles.as_mut().unwrap().column_widths = vec![2, 1];
        assert!(decode_error(&geometry, &header, 16 << 20).contains("tile partition"));
        assert!(
            decode_error(pps, &header, 64 * 64 * 24 + 65536).contains("tile layout exceeds budget")
        );
    }
}
