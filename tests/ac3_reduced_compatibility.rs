#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ac3-reduced-compatibility")
}
fn decode(name: &str, rate: u32) -> Vec<f32> {
    let data = std::fs::read(root().join(name)).unwrap();
    let mut decoder = fvid::codec::ac3_decoder::Ac3Decoder::new(&[], rate, 1).unwrap();
    let pcm = decoder.decode_complete(&data, 0, 0).unwrap().unwrap();
    assert_eq!(pcm.timebase_den, rate);
    pcm.data
        .chunks_exact(4)
        .map(|x| f32::from_le_bytes(x.try_into().unwrap()))
        .collect()
}
#[test]
fn reduced_rate_quiet_allocation_matches_independent_high_precision_pcm_control() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        let expected = decode(
            &format!("{name}-control.ac3"),
            c["base_rate"].as_u64().unwrap() as u32,
        );
        let actual = decode(&format!("{name}.ac3"), rate);
        assert_eq!(actual.len(), c["frames"].as_u64().unwrap() as usize);
        assert_eq!(actual.len(), expected.len());
        let energy: f64 = expected.iter().map(|x| f64::from(*x).powi(2)).sum();
        assert!(energy > 1e-18, "{name}: silent control");
        let peak = actual
            .iter()
            .zip(&expected)
            .map(|(a, b)| f64::from((a - b).abs()))
            .fold(0., f64::max);
        let bound = c["error_bound"].as_f64().unwrap() + 1e-11;
        assert!(
            peak <= bound,
            "{name}: PCM error {peak} exceeds independently authored quantization bound {bound}"
        );
    }
}
fn open(name: &str) -> Box<dyn AudioStream> {
    let data = Cursor::new(std::fs::read(root().join(name)).unwrap());
    if name.ends_with(".mkv") {
        Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(data, Default::default()).unwrap(),
        )
    } else if name.ends_with(".mp4") {
        Box::new(fvid::playback_mp4_audio::Mp4AudioReader::open(data, Default::default()).unwrap())
    } else {
        Box::new(fvid::playback_ac3::Ac3AudioReader::open(data, Default::default()).unwrap())
    }
}
fn pcm(frames: &transitions::Frames) -> Vec<u8> {
    frames.iter().flat_map(|f| f.3.iter().copied()).collect()
}
#[test]
fn reduced_ac3_native_readers_preserve_pcm_clock_aliases_seek_and_rewind() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        let expected = decode(&format!("{name}.ac3"), rate);
        for suffix in [".ac3", "-le.ac3", ".mkv", "-alias.mkv", ".mp4"] {
            let file = format!("{name}{suffix}");
            eprintln!("{file}");
            let mut source = open(&file);
            assert_eq!((source.sample_rate(), source.channels()), (rate, 1));
            let full = transitions::collect(source.as_mut());
            let bytes = pcm(&full);
            let samples: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let start = if suffix.ends_with(".ac3") { 0 } else { 256 };
            assert_eq!(samples.len(), 4352 + start, "{file}");
            assert_eq!(&samples[start..], expected.as_slice(), "{file}");
            for ns in [1, 33_111_111, 99_999_999] {
                let target = std::time::Duration::from_nanos(ns);
                // Raw and native-rate MP4 expose a sample-coordinate seek; their
                // Duration adapter rounds down; WebM navigation uses nanoseconds.
                let raw_tick = (u128::from(ns) * u128::from(rate) / 1_000_000_000) as usize;
                if suffix.ends_with(".ac3") || suffix == ".mp4" {
                    assert_eq!(source.timescale(), rate);
                    source.seek_to(raw_tick as i64);
                } else {
                    source.seek_time(target);
                }
                let actual = transitions::collect(source.as_mut());
                let wanted: Vec<u8> = full
                    .iter()
                    .flat_map(|(r, ch, at, b)| {
                        let skip = (target.saturating_sub(*at).as_nanos() * u128::from(*r))
                            .div_ceil(1_000_000_000) as usize;
                        b[(skip * usize::from(*ch) * 4).min(b.len())..].to_vec()
                    })
                    .collect();
                let wanted = if suffix.ends_with(".ac3") || suffix == ".mp4" {
                    bytes[(raw_tick * 4).min(bytes.len())..].to_vec()
                } else {
                    wanted
                };
                let actual = pcm(&actual);
                assert!(
                    actual == wanted,
                    "{file}: seek {ns}, bytes {} vs {} first mismatch {:?}",
                    actual.len(),
                    wanted.len(),
                    actual.iter().zip(&wanted).position(|(a, b)| a != b)
                );
            }
            source.rewind();
            transitions::compare(&transitions::collect(source.as_mut()), &full);
            if suffix.ends_with(".mkv") || suffix == ".mp4" {
                let mut video = fvid::playback_native::NativeReader::software(
                    Cursor::new(std::fs::read(root().join(&file)).unwrap()),
                    16 << 20,
                )
                .unwrap();
                let mut count = 0;
                while video.read_frame_raw().unwrap().is_some() {
                    count += 1;
                }
                assert!(count > 0);
            }
        }
    }
}
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn reduced_ac3_scheduler_reaches_eof_and_rewinds_at_native_device_geometry() {
    for name in ["bsid9-24000", "bsid10-11025", "bsid10-8000"] {
        for ext in ["-le.ac3", "-alias.mkv", ".mp4"] {
            let file = format!("{name}{ext}");
            let expected: Vec<_> = transitions::collect(open(&file).as_mut())
                .into_iter()
                .map(|f| (f.2, f.3))
                .collect();
            capture::worker_check(open(&file), &expected);
        }
    }
}
#[test]
fn reduced_headers_keep_crc_validation_and_configured_rate_refusal() {
    use fvid::audio::AudioDecode;
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        let data = std::fs::read(root().join(format!("{name}.ac3"))).unwrap();
        let h = fvid::codec::ac3_decoder::checked_syncframe(&data).unwrap();
        assert_eq!((h.sample_rate, h.channels), (rate, 1));
        let mut bad = data[..h.frame_bytes].to_vec();
        *bad.last_mut().unwrap() ^= 1;
        assert!(fvid::codec::ac3_decoder::checked_syncframe(&bad).is_none());
        let mut d = fvid::codec::ac3_decoder::Ac3Decoder::new(&[], rate, 1).unwrap();
        let output = d.decode_complete(&bad, 0, 0).unwrap().unwrap();
        assert_eq!(output.data.len(), 1280 * 4);
        assert!(output.data.iter().all(|&b| b == 0));
        let mut wrong = fvid::codec::ac3_decoder::Ac3Decoder::new(
            &[],
            if rate == 48_000 { 32_000 } else { 48_000 },
            1,
        )
        .unwrap();
        let e = wrong
            .decode_packet(&data, 0, 4608)
            .err()
            .expect("clean wrong-rate AC3 accepted");
        assert!(
            e.to_string().contains("frame sample rate") && e.to_string().contains("disagrees"),
            "{e}"
        );
        let mut demux = fvid::container::webm::WebmReader::open(
            Cursor::new(std::fs::read(root().join(format!("{name}-alias.mkv"))).unwrap()),
            Default::default(),
        )
        .unwrap();
        let bsid = c["bsid"].as_u64().unwrap();
        let tag = if bsid >= 9 {
            format!("A_AC3/BSID{bsid}")
        } else {
            "A_AC3".into()
        };
        assert!(demux.tracks.iter().any(|t| t.kind == 2 && t.codec == tag));
        demux.scan_all().unwrap();
        let i = demux.packets.iter().position(|p| p.track == 2).unwrap();
        assert!(demux.read_packet(i).unwrap() == data[..h.frame_bytes]);
    }
}
#[test]
fn public_decoder_dispatch_accepts_both_legacy_matroska_aliases() {
    for (name, rate, alias) in [
        ("bsid9-22050", 22050, "A_AC3/BSID9"),
        ("bsid10-8000", 8000, "A_AC3/BSID10"),
    ] {
        let bytes = std::fs::read(root().join(format!("{name}.ac3"))).unwrap();
        let expected = decode(&format!("{name}.ac3"), rate);
        for tag in [alias, "A_AC3", "ac-3"] {
            let mut d = fvid::codec::make_audio_decoder(tag, &[], rate, 1, 0).unwrap();
            let p = d.decode_complete(&bytes, 0, 4608).unwrap().unwrap();
            let samples: Vec<f32> = p
                .data
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            assert_eq!(samples, expected);
        }
    }
}
