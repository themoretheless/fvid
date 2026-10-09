use fvid::container::mp4::Mp4Reader;
use fvid_media::owned_aac::{
    aac_native::NativeAacDecoder,
    aac_ssr_alignment::{LaneInput, OutputGain, SsrPcmAlignment},
};
use serde_json::Value;
use std::io::Cursor;
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ssr-alignment.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn packets(video: &Value) -> Vec<Vec<u8>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(video["file"].as_str().unwrap());
    let mut reader = Mp4Reader::open(
        Cursor::new(std::fs::read(path).unwrap()),
        Default::default(),
    )
    .unwrap();
    let track = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    (0..reader.tracks()[track].samples.len())
        .map(|i| {
            let mut out = vec![];
            reader.read_packet(track, i, &mut out).unwrap();
            out
        })
        .collect()
}
#[test]
fn native_ssr_sources_align_with_original_target_stamps_and_independent_pcm_oracle() {
    let gold = include_bytes!("fixtures/playback-errors/aac-ssr-alignment-pcm.f32le");
    let blob = include_bytes!("fixtures/playback-errors/aac-ssr-alignment-packets.bin");
    for case in manifest()["cases"].as_array().unwrap() {
        let sources = packets(&case["source_video"]);
        let mut decoder =
            NativeAacDecoder::new(&hex(case["source_asc"].as_str().unwrap())).unwrap();
        let mut alignment = SsrPcmAlignment::new(1, 2).unwrap();
        let gains = [OutputGain {
            channel: 0,
            gain: 1.0,
        }];
        let mut emitted = vec![];
        let mut position = 0u64;
        let mut previous = 0u64;
        for (i, packet) in sources.iter().enumerate() {
            let descriptor = &case["source_frames"][i];
            let off = descriptor["offset"].as_u64().unwrap() as usize;
            assert_eq!(
                packet,
                &blob[off..off + descriptor["bytes"].as_u64().unwrap() as usize]
            );
            let source = decoder.decode(packet).unwrap();
            assert_eq!(
                source.len(),
                descriptor["samples"].as_u64().unwrap() as usize
            );
            let rows = case["frames"][i]["samples"].as_u64().unwrap() as usize;
            let target = vec![0.0; rows];
            let inputs = [
                LaneInput {
                    samples: &target,
                    outputs: &gains,
                },
                LaneInput {
                    samples: &source,
                    outputs: &gains,
                },
            ];
            let checkpoint = alignment.clone();
            let rendered = alignment.submit(position, rows, &inputs).unwrap();
            alignment = checkpoint;
            assert_eq!(alignment.submit(position, rows, &inputs).unwrap(), rendered);
            if let Some(frame) = rendered {
                assert_eq!(frame.stamp, previous);
                emitted.extend(frame.samples);
            } else {
                assert_eq!(i, 0);
            }
            previous = position;
            position += rows as u64;
        }
        let last = alignment.finish().unwrap().unwrap();
        assert_eq!(last.stamp, previous);
        emitted.extend(last.samples);
        assert_eq!(emitted.len(), 6144);
        let off = case["pcm_offset"].as_u64().unwrap() as usize;
        for (i, (a, b)) in emitted
            .iter()
            .zip(gold[off..off + emitted.len() * 4].chunks_exact(4))
            .enumerate()
        {
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!(
                (a - b).abs() < 2e-7,
                "{} sample {i}: {a} vs {b}",
                case["name"]
            );
        }
    }
}
#[test]
fn coupled_videos_still_refuse_the_exact_unintegrated_alignment_path_and_roll_back() {
    for case in manifest()["cases"].as_array().unwrap() {
        let encoded = packets(&case["video"]);
        let mut decoder = NativeAacDecoder::new(&hex(case["asc"].as_str().unwrap())).unwrap();
        decoder.decode(&encoded[0]).unwrap();
        let saved = decoder.checkpoint();
        assert_eq!(
            decoder.decode(&encoded[1]).unwrap_err().to_string(),
            case["error"].as_str().unwrap()
        );
        let actual = decoder.decode(&encoded[0]).unwrap();
        decoder.restore(&saved).unwrap();
        assert_eq!(decoder.decode(&encoded[0]).unwrap(), actual);
    }
}
#[test]
#[ignore = "SSR alignment primitive is not yet integrated with native delayed frames and EOF"]
fn independently_switched_ssr_coupled_videos_have_native_playback_acceptance() {
    let gold = include_bytes!("fixtures/playback-errors/aac-ssr-alignment-pcm.f32le");
    for case in manifest()["cases"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap());
        let bytes = std::fs::read(path).unwrap();
        let mut actual = vec![];
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut actual,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 6144);
        assert_eq!(actual.len(), 6144 * 4);
        let offset = case["pcm_offset"].as_u64().unwrap() as usize;
        for (a, e) in actual
            .chunks_exact(4)
            .zip(gold[offset..offset + actual.len()].chunks_exact(4))
        {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(e.try_into().unwrap()))
                .abs()
                    < 2e-7
            );
        }
        let mut root = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut root).unwrap();
        assert_eq!(root, actual);
    }
}
