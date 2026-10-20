//! Authored GBR PCM and independent integer range/display oracles.
use fvid::playback_native::{NativeReader, RawFrame};
use std::{io::Cursor, path::PathBuf};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors").join(name)).unwrap()
}
#[test]
fn hevc_identity_rgb_preserves_codes_range_geometry_rewind_and_seek() {
    let manifest: serde_json::Value = serde_json::from_slice(&bytes("hevc-identity-rgb.json")).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 80);
    for case in cases {
        for kind in ["mp4", "file"] {
            let file = case[kind].as_str().unwrap();
            let mut reader = NativeReader::software(Cursor::new(bytes(file)), 32 << 20).unwrap();
            let rotation = reader.rotation();
            let insets = reader.insets();
            let frames = case["frames"].as_array().unwrap();
            for _ in 0..2 {
                for frame in frames {
                    let raw = reader.read_frame_raw().unwrap().unwrap();
                    let RawFrame::Planar(p) = raw else { panic!("{file}: RGB packed payload required") };
                    assert_eq!(p.depth, case["depth"].as_u64().unwrap() as u8);
                    assert_eq!(p.frame.subsampling, None);
                    assert_eq!(p.frame.data, bytes(frame["rgb"].as_str().unwrap()), "{file}: source RGB codes");
                    let mut rgb = Vec::new();
                    p.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
                    assert_eq!(rgb, bytes(frame["rgb8"].as_str().unwrap()), "{file}: range normalization");
                    if kind == "file" && case["crop"][0].as_u64().unwrap() == 0 {
                        let turned = p.rotated(rotation).unwrap();
                        turned.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
                        assert_eq!(rgb, bytes(frame["display8"].as_str().unwrap()), "{file}: rotated range");
                    }
                    if kind == "file" {
                        let display = fvid::native_geometry::VideoGeometry::default().apply_packed_display_media(&p, rotation, insets).unwrap();
                        assert_eq!(display.frame.data, bytes(frame["display"].as_str().unwrap()), "{file}: display codes");
                        display.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
                        assert_eq!(rgb, bytes(frame["display8"].as_str().unwrap()), "{file}: display range");
                    }
                }
                assert!(reader.read_frame_raw().unwrap().is_none());
                reader.rewind().unwrap();
            }
            for index in [frames.len() - 1, 0] {
                let raw = reader.seek_raw(std::time::Duration::from_nanos(
                    index as u64 * 1_000_000_000 / case["rate"].as_u64().unwrap(),
                )).unwrap().unwrap();
                let RawFrame::Planar(p) = raw else { panic!("RGB seek payload required") };
                assert_eq!(p.frame.data, bytes(frames[index]["rgb"].as_str().unwrap()), "{file}: seek codes");
                let mut rgb = Vec::new();
                p.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
                assert_eq!(rgb, bytes(frames[index]["rgb8"].as_str().unwrap()), "{file}: seek range");
            }
        }
    }
}

#[test]
fn hevc_rgb_lossless_export_keeps_channels_depth_range_and_timing() {
    let manifest: serde_json::Value = serde_json::from_slice(&bytes("hevc-identity-rgb.json")).unwrap();
    for case in manifest["cases"].as_array().unwrap().iter().filter(|c| c["rotation"] == 0 && c["crop"][0] == 0) {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors").join(case["mp4"].as_str().unwrap());
        let output = std::env::temp_dir().join(format!("fvid-hevc-rgb-{}-{}.mkv", std::process::id(), case["mp4"].as_str().unwrap()));
        fvid_media::transcode_lossless(&source, &output,
            fvid::media_info::LosslessTransform { horizontal_flip: true, ..Default::default() }, &Default::default()).unwrap();
        let data = std::fs::read(&output).unwrap();
        let mut container = fvid_media::owned_webm::WebmReader::open(Cursor::new(data.clone()), Default::default()).unwrap();
        container.scan_all().unwrap();
        let track = &container.tracks[0];
        assert_eq!(track.colour.matrix, 0);
        assert_eq!(track.full_range, Some(case["full_range"].as_bool().unwrap()));
        assert_eq!(track.colour.full_range, case["full_range"].as_bool().unwrap());
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(track.width as usize, track.height as usize, &track.codec_private, 32 << 20).unwrap();
        let depth = case["depth"].as_u64().unwrap() as u8;
        let width = case["size"][0].as_u64().unwrap() as usize;
        let unit = if depth == 8 { 1 } else { 2 };
        let mut player = NativeReader::software(Cursor::new(data), 32 << 20).unwrap();
        for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            let decoded = decoder.decode(&container.read_packet(i).unwrap()).unwrap();
            assert_eq!(decoded.frame.subsampling, None, "RGB export must retain identity storage");
            assert_eq!(decoded.depth, depth);
            let flip = |data: Vec<u8>, bytes: usize| -> Vec<u8> { data.chunks_exact(width * bytes * 3).flat_map(|row| row.chunks_exact(bytes * 3).rev().flatten().copied()).collect() };
            assert_eq!(decoded.frame.data, flip(bytes(frame["rgb"].as_str().unwrap()), unit));
            let RawFrame::Planar(p) = player.read_frame_raw().unwrap().unwrap() else { panic!("RGB playback payload") };
            let mut rgb = Vec::new();
            p.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
            assert_eq!(rgb, flip(bytes(frame["rgb8"].as_str().unwrap()), 1), "exported range presentation");
            assert_eq!(container.packets[i].pts_ns, i as i64 * 1_000_000_000 / case["rate"].as_i64().unwrap());
        }
        assert_eq!(container.packets.len(), case["frames"].as_array().unwrap().len());
        assert!(player.read_frame_raw().unwrap().is_none());
        for index in [case["frames"].as_array().unwrap().len() - 1, 0] {
            let raw = player.seek_raw(std::time::Duration::from_nanos(index as u64 * 1_000_000_000 / case["rate"].as_u64().unwrap())).unwrap().unwrap();
            let RawFrame::Planar(p) = raw else { panic!("exported RGB seek payload") };
            let original = bytes(case["frames"][index]["rgb8"].as_str().unwrap());
            let expected: Vec<u8> = original.chunks_exact(width * 3).flat_map(|row| row.chunks_exact(3).rev().flatten().copied()).collect();
            let mut rgb = Vec::new();
            p.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
            assert_eq!(rgb, expected, "exported RGB seek range");
        }
        player.rewind().unwrap();
        assert!(player.read_frame_raw().unwrap().is_some());
        std::fs::remove_file(output).unwrap();
    }
}
