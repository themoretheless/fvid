#![cfg(feature = "player")]
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-v1-coded-config")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    manifest()["cases"].as_array().unwrap().clone()
}
fn decode(n: &str) -> Vec<u8> {
    let mut out = Vec::new();
    if n.starts_with("ec3-") {
        let mut reader = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let t = &reader.tracks()[1];
        let mut decoder: Box<dyn fvid::audio::AudioDecode> = Box::new(
            fvid::codec::eac3_decoder::Eac3Decoder::for_container_with_compression(
                &t.configuration,
                t.sample_rate,
                t.channels,
                fvid::codec::ac3_decoder::CompressionMode::Line,
            )
            .unwrap(),
        );
        let count = t.samples.len();
        let mut packet = Vec::new();
        for i in 0..count {
            let sample = reader.tracks()[1].samples.get(i).unwrap();
            reader.read_packet(1, i, &mut packet).unwrap();
            if let Some(frame) = decoder
                .decode_packet(
                    &packet,
                    sample.pts * i64::from(reader.tracks()[1].sample_rate)
                        / i64::from(reader.tracks()[1].timescale),
                    u64::from(sample.duration) * u64::from(reader.tracks()[1].sample_rate)
                        / u64::from(reader.tracks()[1].timescale),
                )
                .unwrap()
            {
                out.extend(frame.packet.data);
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            out.extend(frame.packet.data);
        }
        return out;
    }
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes(n)),
        &mut out,
        None,
        &Default::default(),
    )
    .unwrap();
    out
}
#[test]
fn version_zero_controls_keep_valid_nonzero_pcm_and_video() {
    for c in manifest()["controls"].as_array().unwrap() {
        let n = c["file"].as_str().unwrap();
        let pcm = decode(n);
        assert_eq!(
            pcm.len(),
            c["frames"].as_u64().unwrap() as usize * c["channels"].as_u64().unwrap() as usize * 4
        );
        assert!(
            pcm.chunks_exact(4)
                .any(|v| f32::from_le_bytes(v.try_into().unwrap()) != 0.),
            "{n}"
        );
        if c["codec"] == "alac" {
            assert_eq!(pcm, bytes("alac.f32le"));
        }
        let video = fvid_media::decode_video(&root().join(n)).unwrap();
        assert_eq!(video.decode_errors, 0);
        assert_eq!(video.video_frames, if c["codec"] == "alac" { 3 } else { 5 });
    }
}
#[test]
fn version_one_configuration_and_every_packet_match_the_valid_control() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let control = c["control"].as_str().unwrap();
        let mut target = match fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(n)),
            Default::default(),
        ) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{n}:{e}"));
                continue;
            }
        };
        let mut baseline = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(control)),
            Default::default(),
        )
        .unwrap();
        let cfg = &target.tracks()[1].configuration;
        let original = &baseline.tracks()[1].configuration;
        if cfg != original {
            failures.push(format!(
                "{n}:config {} != {} bytes",
                cfg.len(),
                original.len()
            ));
            continue;
        }
        assert_eq!(
            (target.tracks()[1].sample_rate, target.tracks()[1].channels),
            (
                baseline.tracks()[1].sample_rate,
                baseline.tracks()[1].channels
            )
        );
        let mut a = Vec::new();
        let mut b = Vec::new();
        for track in 0..2 {
            assert_eq!(
                target.tracks()[track].samples.len(),
                baseline.tracks()[track].samples.len()
            );
            for i in 0..target.tracks()[track].samples.len() {
                let a0 = target.tracks()[track].samples.get(i).unwrap();
                let b0 = baseline.tracks()[track].samples.get(i).unwrap();
                assert_eq!(
                    (a0.pts, a0.dts, a0.duration, a0.size, a0.description_index),
                    (b0.pts, b0.dts, b0.duration, b0.size, b0.description_index)
                );
                target.read_packet(track, i, &mut a).unwrap();
                baseline.read_packet(track, i, &mut b).unwrap();
                assert_eq!(a, b);
            }
        }
        assert_eq!(decode(n), decode(control), "{n}");
    }
    assert!(
        failures.is_empty(),
        "{} version1 failures: {failures:?}",
        failures.len()
    );
}
#[test]
fn incomplete_version_one_headers_have_specific_invalid_refusals() {
    let mut failures = Vec::new();
    for n in manifest()["bad"].as_array().unwrap() {
        let n = n.as_str().unwrap();
        match fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(n)),
            Default::default(),
        ) {
            Err(e) => {
                assert!(matches!(e, fvid::Error::Invalid(_)));
                if !e
                    .to_string()
                    .contains("truncated version-1 MP4 audio sample description")
                {
                    failures.push(format!("{n}:wrong refusal:{e}"));
                }
            }
            Ok(_) => failures.push(format!("{n}:admitted")),
        }
        match fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(n)),
            Default::default(),
        ) {
            Err(e) => {
                assert!(!e.is_unsupported());
                if !e
                    .to_string()
                    .contains("truncated version-1 MP4 audio sample description")
                {
                    failures.push(format!("owned:{n}:wrong refusal:{e}"));
                }
            }
            Ok(_) => failures.push(format!("owned:{n}:admitted")),
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
