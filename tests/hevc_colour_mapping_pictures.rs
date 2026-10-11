//! Full reference-picture mapping against owned Y4M/rational oracles.
//! This does not claim inter-layer slice decoding or scalable playback.
use fvid::codec::{
    hevc_pps::{ColourMappingPicture, ColourMappingTable, LayeredPps, LayeredPpsContext},
    hevc_sps::{LayeredSps, LayeredSpsContext},
    hevc_vps::LayeredVps,
};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn table(prefix: &str) -> ColourMappingTable {
    let read = |suffix: &str| data(&format!("{prefix}.{suffix}"));
    let vps = LayeredVps::parse(&read("vps"), 1 << 20).unwrap();
    let base = LayeredSps::parse(
        &read("base.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 0,
            output_layer_set: 0,
            active_sps: &[],
        },
        1 << 20,
    )
    .unwrap();
    let current = LayeredSps::parse(
        &read("current.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 3,
            output_layer_set: 1,
            active_sps: std::slice::from_ref(&base),
        },
        1 << 20,
    )
    .unwrap();
    LayeredPps::parse(
        &read("pps"),
        &LayeredPpsContext {
            vps: &vps,
            sps: &current,
            active_sps: std::slice::from_ref(&base),
            active_pps: &[],
        },
        1 << 20,
    )
    .unwrap()
    .colour_mapping
    .unwrap()
}
#[test]
fn spatial_colour_mapping_matches_every_sample_and_keeps_reference_unchanged() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("hevc-colour-mapping-pictures.json")).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["prefix"].as_str().unwrap();
        let table = table(case["pps"].as_str().unwrap());
        let [w, h] = std::array::from_fn(|i| case["size"][i].as_u64().unwrap() as usize);
        let format = case["format"].as_u64().unwrap() as u8;
        let depth = case["depth"].as_u64().unwrap() as u8;
        let [sx, sy] = match format {
            1 => [2, 2],
            2 => [2, 1],
            _ => [1, 1],
        };
        let counts = [w * h, w * h / sx / sy, w * h / sx / sy];
        let bytes = data(&format!("{name}.y4m"));
        let mut at = bytes.iter().position(|&v| v == b'\n').unwrap() + 1;
        let header = std::str::from_utf8(&bytes[..at]).unwrap();
        assert!(header.starts_with(&format!("YUV4MPEG2 W{w} H{h} F25:1 ")));
        let suffix = format!(
            "C{}{}\n",
            match format {
                1 => "420",
                2 => "422",
                _ => "444",
            },
            if depth == 8 {
                String::new()
            } else {
                format!("p{depth}")
            }
        );
        assert!(header.ends_with(&suffix));
        for frame in case["frames"].as_array().unwrap() {
            assert_eq!(&bytes[at..at + 6], b"FRAME\n");
            at += 6;
            let planes: [Vec<u16>; 3] = std::array::from_fn(|c| {
                (0..counts[c])
                    .map(|_| {
                        let value = if depth == 8 {
                            u16::from(bytes[at])
                        } else {
                            u16::from_le_bytes([bytes[at], bytes[at + 1]])
                        };
                        at += if depth == 8 { 1 } else { 2 };
                        value
                    })
                    .collect()
            });
            assert_eq!(serde_json::to_value(&planes).unwrap(), frame["input"]);
            let snapshot = planes.clone();
            let input = || ColourMappingPicture {
                dimensions: [w, h],
                chroma_format: format,
                depth: [depth; 2],
                planes: std::array::from_fn(|i| planes[i].as_slice()),
            };
            let mapped = table
                .map_picture(input(), 1 << 20)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            for c in 0..3 {
                assert!(mapped[c].complete());
                assert_eq!(
                    mapped[c].dimensions(),
                    if c == 0 { [w, h] } else { [w / sx, h / sy] }
                );
                assert_eq!(
                    serde_json::to_value(mapped[c].samples()).unwrap(),
                    frame["output"][c],
                    "{name} component {c}"
                );
            }
            assert_eq!(planes, snapshot);
            let replay = table.map_picture(input(), 1 << 20).unwrap();
            for c in 0..3 {
                assert_eq!(replay[c].samples(), mapped[c].samples());
            }
            let error = table
                .map_picture(input(), counts.iter().sum::<usize>() * 3 - 1)
                .err()
                .unwrap();
            assert!(error.to_string().contains("budget"), "{name}: {error}");
            let mut invalid = input();
            invalid.depth[0] = if depth == 8 { 10 } else { 8 };
            assert!(table.map_picture(invalid, 1 << 20).is_err());
            let mut invalid = input();
            invalid.planes[1] = &planes[1][1..];
            assert!(table.map_picture(invalid, 1 << 20).is_err());
            let mut invalid = input();
            invalid.chroma_format = 0;
            assert!(table.map_picture(invalid, 1 << 20).is_err());
        }
        assert_eq!(at, bytes.len());
    }
}
