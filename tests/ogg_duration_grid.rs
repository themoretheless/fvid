#![cfg(feature = "player")]
use fvid::audio::{AudioStep, AudioStream};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(base: &Path, kind: &str, name: &str) -> Box<dyn AudioStream> {
    if kind == "joint" {
        Box::new(fvid::playback_ogg_catalog::Reader::open(&base.join(name), 0).unwrap())
    } else {
        Box::new(
            fvid::playback_ogg_audio::OggAudioReader::open(
                Cursor::new(std::fs::read(base.join(name)).unwrap()),
                Default::default(),
            )
            .unwrap(),
        )
    }
}
fn collect(source: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut decoder = source.make_decoder().unwrap();
    let mut packets = Vec::new();
    while let Some(step) = source.next_step().unwrap() {
        match step {
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => source.reset_decoder(&mut decoder).unwrap(),
            AudioStep::Pcm(p) => packets.push((p.presentation_time(), p.data)),
            AudioStep::Encoded(p) => {
                if let Some(f) = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                {
                    if let Some(pcm) = source.present_decoded(f.packet, p.pts).unwrap() {
                        packets.push((pcm.presentation_time(), pcm.data));
                    }
                }
            }
        }
    }
    packets
}
#[test]
fn ogg_vorbis_and_joint_public_duration_match_exact_pcm_endpoint() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-duration-grid");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut opus = fvid::playback_ogg_opus::OggOpusAudioReader::open(
        Cursor::new(std::fs::read(base.join("opus-control.ogg")).unwrap()),
        Default::default(),
    )
    .unwrap();
    let pcm = collect(&mut opus);
    assert_eq!(pcm.iter().map(|(_, p)| p.len() / 8).sum::<usize>(), 960);
    let peak = pcm
        .iter()
        .flat_map(|(_, p)| p.chunks_exact(4))
        .map(|s| f32::from_le_bytes(s.try_into().unwrap()).abs())
        .fold(0.0f32, f32::max);
    assert!(peak <= 1e-30, "Opus control peak{peak}");
    assert!(
        pcm.iter()
            .flat_map(|(_, p)| p.chunks_exact(4))
            .all(|s| f32::from_le_bytes(s.try_into().unwrap()).abs() <= 1e-30)
    );
    assert_eq!(opus.duration(), Some(Duration::from_millis(20)));
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let kind = case["kind"].as_str().unwrap();
        let samples = case["samples"].as_u64().unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let end = Duration::from_nanos(case["end_ns"].as_u64().unwrap());
        let mut source = open(&base, kind, name);
        assert_eq!(source.sample_rate() as u64, rate);
        assert_eq!(source.channels(), 2);
        let published = source.duration().unwrap();
        if published != end {
            failures.push(format!("{name}: duration{published:?} instead of{end:?}"));
        }
        assert_eq!(source.time_of(samples as i64), end);
        let expected = collect(source.as_mut());
        let mut cursor = 0u64;
        for (at, data) in &expected {
            assert_eq!(
                *at,
                Duration::from_nanos(cursor * 1_000_000_000 / rate),
                "{name}: independentPTS"
            );
            assert!(
                data.chunks_exact(4)
                    .all(|s| f32::from_le_bytes(s.try_into().unwrap()).abs() <= 1e-30),
                "{name}: literal silence"
            );
            cursor += (data.len() / 8) as u64;
        }
        assert_eq!(cursor, samples, "{name}: completeEOSsample count");
        assert_eq!(source.seek_to(samples as i64), samples as i64);
        assert!(source.next_step().unwrap().is_none());
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            capture::worker_check_with_seek(open(&base, kind, name), &expected, Some(published))
        }))
        .is_err()
        {
            failures.push(format!("{name}: actualpublicEOFclock"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
