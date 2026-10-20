//! RFC 9043 compatibility: v1 zero depth is interpreted as eight bits.
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
#[test]
fn zero_depth_v1_decodes_color_alpha_history_rewind_and_seek() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("ffv1-zero-depth.json")).unwrap())
            .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 24);
    for case in m["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_native::NativeReader::software(Cursor::new(data), 32 << 20).unwrap();
        for pass in 0..2 {
            for frame in case["frames"].as_array().unwrap() {
                let raw = reader
                    .read_frame_raw()
                    .unwrap_or_else(|e| panic!("{}: {e}", case["file"]))
                    .unwrap();
                let fvid::playback_native::RawFrame::Planar(p) = raw else {
                    panic!("packed FFV1")
                };
                assert_eq!(p.depth, 8);
                assert_eq!(
                    p.frame.data,
                    std::fs::read(root().join(frame["color"].as_str().unwrap())).unwrap()
                );
                assert_eq!(
                    p.alpha,
                    frame["alpha"]
                        .as_str()
                        .map(|f| std::fs::read(root().join(f)).unwrap())
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            if pass == 0 {
                reader.rewind().unwrap();
            }
        }
        for index in [1, 0] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(index * 20))
                .unwrap()
                .unwrap();
            let fvid::playback_native::RawFrame::Planar(p) = raw else {
                panic!("packed FFV1")
            };
            let f = &case["frames"][index as usize];
            assert_eq!(
                p.frame.data,
                std::fs::read(root().join(f["color"].as_str().unwrap())).unwrap()
            );
            assert_eq!(
                p.alpha,
                f["alpha"]
                    .as_str()
                    .map(|f| std::fs::read(root().join(f)).unwrap())
            );
        }
    }
}

#[test]
fn zero_depth_v1_lossless_export_writes_explicit_eight_bit_depth() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("ffv1-zero-depth.json")).unwrap())
            .unwrap();
    for (index, case) in m["cases"].as_array().unwrap().iter().enumerate() {
        let output = std::env::temp_dir().join(format!(
            "fvid-zero-depth-{}-{index}.mkv",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        fvid_media::transcode_lossless(
            &root().join(case["file"].as_str().unwrap()),
            &output,
            fvid::media_info::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let mut container = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(&output).unwrap()),
            Default::default(),
        )
        .unwrap();
        container.scan_all().unwrap();
        let track = &container.tracks[0];
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
            track.width as usize,
            track.height as usize,
            &track.codec_private,
            32 << 20,
        )
        .unwrap();
        for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            let packet = container.read_packet(i).unwrap();
            if i == 0 {
                // Independently parse the range-coded header to qualify explicit depth.
                assert_eq!(legacy_header_depth(&packet), 8);
            }
            assert_eq!(container.packets[i].pts_ns, i as i64 * 20_000_000);
            assert_eq!(container.packets[i].duration_ns, Some(20_000_000));
            let decoded = decoder.decode(&packet).unwrap();
            let source = std::fs::read(root().join(frame["color"].as_str().unwrap())).unwrap();
            let flip = |data: &[u8], width: usize, channels: usize| -> Vec<u8> {
                data.chunks_exact(width * channels)
                    .flat_map(|row| row.chunks_exact(channels).rev().flatten().copied())
                    .collect()
            };
            let expected = match case["layout"].as_str().unwrap() {
                "rgb" => flip(&source, 4, 3),
                "420" => [
                    flip(&source[..8], 4, 1),
                    flip(&source[8..10], 2, 1),
                    flip(&source[10..], 2, 1),
                ]
                .concat(),
                _ => flip(&source, 4, 1),
            };
            assert_eq!(decoded.frame.data, expected, "{}", case["file"]);
            assert_eq!(decoded.depth, 8);
            assert_eq!(
                decoded.alpha,
                frame["alpha"].as_str().map(|f| flip(
                    &std::fs::read(root().join(f)).unwrap(),
                    4,
                    1
                ))
            );
        }
        std::fs::remove_file(output).unwrap();
    }
}

// Minimal independent FFV1 default range reader, solely for the encoder header.
fn legacy_header_depth(data: &[u8]) -> u32 {
    struct Reader<'a> {
        data: &'a [u8],
        at: usize,
        range: u32,
        low: u32,
    }
    impl Reader<'_> {
        fn bit(&mut self, state: &mut u8) -> bool {
            // Default transition table is the published FFV1 table.
            let table = TRANSITIONS;
            let split = (self.range * u32::from(*state)) >> 8;
            let bit = self.low >= self.range - split;
            if bit {
                self.low -= self.range - split;
                self.range = split;
                *state = table[*state as usize];
            } else {
                self.range -= split;
                *state = 256u16.wrapping_sub(u16::from(table[256 - *state as usize])) as u8;
            }
            while self.range < 256 {
                self.range <<= 8;
                self.low = (self.low << 8) | u32::from(*self.data.get(self.at).unwrap_or(&0));
                self.at += 1;
            }
            bit
        }
        fn integer(&mut self, s: &mut [u8; 32]) -> u32 {
            if self.bit(&mut s[0]) {
                return 0;
            }
            let mut e = 0;
            while self.bit(&mut s[1 + e]) {
                e += 1;
            }
            let mut value = 1;
            for i in (0..e).rev() {
                value = 2 * value + u32::from(self.bit(&mut s[22 + i]));
            }
            value
        }
    }
    let mut r = Reader {
        data,
        at: 2,
        range: 65280,
        low: u32::from(data[0]) * 256 + u32::from(data[1]),
    };
    assert!(r.bit(&mut 128));
    let mut s = [128; 32];
    assert_eq!(r.integer(&mut s), 1);
    assert_eq!(r.integer(&mut s), 1);
    let _colorspace = r.integer(&mut s);
    r.integer(&mut s)
}

const TRANSITIONS: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37,
    37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 56, 57, 58, 59,
    60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 75, 76, 77, 78, 79, 80, 81, 82,
    83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103,
    104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 114, 115, 116, 117, 118, 119, 120, 121,
    122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 133, 134, 135, 136, 137, 138, 139,
    140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 152, 153, 154, 155, 156, 157,
    158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 171, 172, 173, 174, 175,
    176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 190, 191, 192, 194,
    194, 195, 196, 197, 198, 199, 200, 201, 202, 202, 204, 205, 206, 207, 208, 209, 209, 210, 211,
    212, 213, 215, 215, 216, 217, 218, 219, 220, 220, 222, 223, 224, 225, 226, 227, 227, 229, 229,
    230, 231, 232, 234, 234, 235, 236, 237, 238, 239, 240, 241, 242, 243, 244, 245, 246, 247, 248,
    248, 0, 0, 0, 0, 0, 0, 0,
];
