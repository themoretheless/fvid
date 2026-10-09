use fvid_media::owned_aac::NativeAacDecoder;
use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn manifest() -> Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/aac-main-tools.json")).unwrap()
}
fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn asc(case: &Value) -> Vec<u8> {
    case["asc"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect()
}
fn bytes(video: &Value) -> Vec<u8> {
    std::fs::read(path(video["file"].as_str().unwrap())).unwrap()
}
fn decode(video: &Value) -> Vec<u8> {
    let mut output = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes(video)),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap();
    output
}
fn compare(actual: &[u8], expected: &[u8], name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name}");
    assert!(
        expected
            .chunks_exact(4)
            .any(|s| f32::from_le_bytes(s.try_into().unwrap()) != 0.0)
    );
    for (i, (a, b)) in actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .enumerate()
    {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() < 2e-8, "{name} sample {i}: {a} vs {b}");
    }
}
#[test]
fn main_stereo_pns_and_order20_tns_videos_match_independent_pcm_oracles() {
    let blob = include_bytes!("fixtures/playback-errors/aac-main-tools-packets.bin");
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let config = asc(case);
        let mut owned = NativeAacDecoder::new(&config).unwrap();
        let mut root = fvid::codec::aac_native::NativeAacDecoder::new(&config).unwrap();
        let mut pcm = Vec::new();
        for (index, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let start = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let packet = &blob[start..start + len];
            let saved = owned.checkpoint();
            let output = owned.decode(packet).unwrap();
            owned.restore(&saved).unwrap();
            assert_eq!(
                owned.decode(packet).unwrap(),
                output,
                "{name} checkpoint {index}"
            );
            let saved_root = root.checkpoint();
            assert_eq!(root.decode(packet).unwrap(), output, "{name} root {index}");
            root.restore(&saved_root).unwrap();
            assert_eq!(root.decode(packet).unwrap(), output);
            assert_eq!(
                output.len(),
                row["samples"].as_u64().unwrap() as usize
                    * case["channels"].as_u64().unwrap() as usize
            );
            pcm.extend(output.iter().flat_map(|v| v.to_le_bytes()));
            let after = owned.checkpoint();
            assert!(
                owned.decode(&packet[..packet.len() / 2]).is_err(),
                "{name} truncation {index}"
            );
            let replay = owned.decode(packet).unwrap();
            owned.restore(&after).unwrap();
            assert_eq!(
                owned.decode(packet).unwrap(),
                replay,
                "{name} failed packet rollback {index}"
            );
            owned.restore(&after).unwrap();
        }
        let expected = std::fs::read(path(case["pcm_file"].as_str().unwrap())).unwrap();
        compare(&pcm, &expected, name);
        assert_eq!(decode(&case["video"]), pcm, "{name} owned MP4");
        let mut exported = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes(&case["video"]), &mut exported).unwrap();
        assert_eq!(exported, pcm, "{name} root MP4");
        if let Some(control) = case.get("control_video") {
            let baseline = decode(control);
            compare(
                &baseline,
                &std::fs::read(path(case["control_pcm_file"].as_str().unwrap())).unwrap(),
                name,
            );
            assert!(
                pcm.chunks_exact(4).zip(baseline.chunks_exact(4)).any(
                    |(a, b)| (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap()))
                    .abs()
                        > 1e-5
                ),
                "{name} must exercise a nonzero twentieth-order effect"
            );
            let row = &case["frames"][0];
            let start = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let mut lc = config.clone();
            lc[0] = (lc[0] & 7) | (2 << 3);
            let mut decoder = NativeAacDecoder::new(&lc).unwrap();
            assert_eq!(
                decoder
                    .decode(&blob[start..start + len])
                    .unwrap_err()
                    .to_string(),
                "AAC-LC TNS order exceeds limit"
            );
        }
        owned.reset();
        let row = &case["frames"][0];
        let start = row["offset"].as_u64().unwrap() as usize;
        let len = row["bytes"].as_u64().unwrap() as usize;
        let mut fresh = NativeAacDecoder::new(&config).unwrap();
        assert_eq!(
            owned.decode(&blob[start..start + len]).unwrap(),
            fresh.decode(&blob[start..start + len]).unwrap(),
            "{name} reset"
        );
    }
}
#[test]
fn main_tns_order21_video_refuses_with_main_profile_diagnostic_and_preserves_history() {
    let m = manifest();
    let blob = include_bytes!("fixtures/playback-errors/aac-main-tools-packets.bin");
    for invalid in m["invalid"].as_array().unwrap() {
        let valid = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| {
                invalid["name"]
                    .as_str()
                    .unwrap()
                    .starts_with(v["name"].as_str().unwrap())
            })
            .unwrap();
        let mut decoder = NativeAacDecoder::new(&asc(valid)).unwrap();
        let mut root = fvid::codec::aac_native::NativeAacDecoder::new(&asc(valid)).unwrap();
        for row in valid["frames"].as_array().unwrap().iter().take(4) {
            let start = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            decoder.decode(&blob[start..start + len]).unwrap();
            root.decode(&blob[start..start + len]).unwrap();
        }
        let checkpoint = decoder.checkpoint();
        let root_checkpoint = root.checkpoint();
        let row = &valid["frames"][4];
        let start = row["offset"].as_u64().unwrap() as usize;
        let len = row["bytes"].as_u64().unwrap() as usize;
        let expected = decoder.decode(&blob[start..start + len]).unwrap();
        decoder.restore(&checkpoint).unwrap();
        assert_eq!(root.decode(&blob[start..start + len]).unwrap(), expected);
        root.restore(&root_checkpoint).unwrap();
        let row = &invalid["frames"][0];
        let at = row["offset"].as_u64().unwrap() as usize;
        let size = row["bytes"].as_u64().unwrap() as usize;
        assert_eq!(
            decoder
                .decode(&blob[at..at + size])
                .unwrap_err()
                .to_string(),
            "AAC Main TNS order exceeds limit"
        );
        assert_eq!(
            root.decode(&blob[at..at + size]).unwrap_err().to_string(),
            "AAC Main TNS order exceeds limit"
        );
        assert_eq!(root.decode(&blob[start..start + len]).unwrap(), expected);
        assert_eq!(decoder.decode(&blob[start..start + len]).unwrap(), expected);
        assert!(
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(&invalid["video"])),
                &mut Vec::new(),
                None,
                &Default::default()
            )
            .unwrap_err()
            .to_string()
            .contains("AAC Main TNS order exceeds limit")
        );
    }
}
#[cfg(feature = "player")]
#[test]
fn main_tool_videos_playback_rewind_seek_and_checkpoint_pcm_match_full_export() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut fvid::playback_mp4_audio::Mp4AudioReader<Cursor<&Vec<u8>>>) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut output = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            let saved = decoder.checkpoint().unwrap();
            let frame = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            decoder.restore(&saved).unwrap();
            let replay = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            assert_eq!(
                frame
                    .as_ref()
                    .map(|f| (&f.packet.data, f.source_pts, f.source_duration)),
                replay
                    .as_ref()
                    .map(|f| (&f.packet.data, f.source_pts, f.source_duration))
            );
            if let Some(frame) = frame {
                if let Some(pcm) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    output.extend(pcm.data);
                }
            }
        }
        assert!(decoder.finish_packet().unwrap().is_none());
        output
    }
    for case in manifest()["cases"].as_array().unwrap() {
        let data = bytes(&case["video"]);
        let expected = decode(&case["video"]);
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for at in [1100, 3000, 6100, case["samples"].as_u64().unwrap()] {
            let landed = reader.seek_to(at as i64);
            assert_eq!(
                play(&mut reader),
                expected[landed as usize * case["channels"].as_u64().unwrap() as usize * 4..],
                "{} seek {at}",
                case["name"]
            );
        }
    }
}
