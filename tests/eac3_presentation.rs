#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{Eac3Decoder, presentation::PresentationDecoder},
};
use std::{io::Cursor, path::Path, time::Duration};
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-presentation")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let c = m["cases"].as_array().unwrap().clone();
    assert_eq!(c.len(), 27);
    c
}
fn open(n: &str) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(read(n)), Default::default())
            .unwrap(),
    )
}
fn packets(s: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut d = s.make_decoder().unwrap();
    let mut out = vec![];
    while let Some(step) = s.next_step().unwrap() {
        let packet = match step {
            AudioStep::Encoded(p) => d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .and_then(|d| s.present_decoded(d.packet, d.source_pts).unwrap()),
            AudioStep::Pcm(p) => Some(p),
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => {
                s.reset_decoder(&mut d).unwrap();
                None
            }
        };
        if let Some(p) = packet {
            out.push((p.presentation_time(), p.data));
        }
    }
    out.extend(d.drain_presented(s).into_iter().map(|p| (p.presentation_time(), p.data)));
    s.validate_eof().unwrap();
    out
}
fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
fn expected(c: &serde_json::Value) -> Vec<u8> {
    let width = c["channels"].as_u64().unwrap() as usize;
    let blocks = c["blocks"].as_u64().unwrap() as usize;
    let samples = 3 * blocks * 256;
    let mut out = vec![0.0f32; samples * width];
    for component in c["components"].as_array().unwrap() {
        let pcm = floats(
            &packets(&mut *open(component["control_video"].as_str().unwrap()))
                .into_iter()
                .flat_map(|(_, b)| b)
                .collect::<Vec<_>>(),
        );
        let roles = component["roles"].as_array().unwrap();
        assert_eq!(pcm.len(), samples * roles.len());
        for (coded, role) in roles.iter().enumerate() {
            let slot = c["speakers"]
                .as_array()
                .unwrap()
                .iter()
                .position(|s| s == role)
                .unwrap();
            let native = component["coded_to_native"][coded].as_u64().unwrap() as usize;
            for n in 0..samples {
                out[n * width + slot] =
                    pcm[n * roles.len() + native] * c["scale"].as_f64().unwrap() as f32;
            }
        }
    }
    for n in 0..samples {
        let gain = c["gains"][(n / 256) % blocks].as_f64().unwrap() as f32;
        for ch in 0..width {
            out[n * width + ch] = (out[n * width + ch] * gain).clamp(-1.0, 1.0);
        }
    }
    if c["name"].as_str().unwrap().ends_with("-clip") {
        assert!(
            out.iter().any(|v| v.abs() > 0.125),
            "clipping witness must distinguish pre-gain clamping"
        );
    }
    out.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn compare(got: &[u8], want: &[u8], c: &serde_json::Value) {
    assert_eq!(got.len(), want.len(), "{} size", c["name"]);
    let mut error = 0.0f32;
    for (a, b) in floats(got).into_iter().zip(floats(want)) {
        assert!(a.is_finite());
        error = error.max((a - b).abs());
    }
    let bound = c["bound"].as_f64().unwrap();
    assert!(
        f64::from(error) <= if bound == 0.0 { 0.0 } else { bound + 1e-7 },
        "{} error {error} bound {bound}",
        c["name"]
    );
}
#[test]
fn presentation_replaces_extends_and_applies_last_dependent_gain_once() {
    for c in cases() {
        let want = expected(&c);
        let raw = read(c["file"].as_str().unwrap());
        let width = c["channels"].as_u64().unwrap() as u16;
        let mut d =
            PresentationDecoder::for_container(c["rate"].as_u64().unwrap() as u32, width).unwrap();
        let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        compare(&got, &want, &c);
        assert!(d.finish_packet().unwrap().is_none());
        assert_eq!(
            d.speakers().unwrap().wave_mask(),
            c["wave_mask"].as_u64().map(|m| m as u32)
        );
        d.reset();
        compare(
            &d.decode_complete(&raw, 0, 0).unwrap().unwrap().data,
            &want,
            &c,
        );
        let mut dispatched =
            Eac3Decoder::new(&[], c["rate"].as_u64().unwrap() as u32, width).unwrap();
        compare(
            &dispatched.decode_complete(&raw, 0, 0).unwrap().unwrap().data,
            &want[256 * usize::from(width) * 4..],
            &c,
        );
    }
}
#[test]
fn split_substreams_retain_original_signed_window_and_report_incomplete_eof() {
    let c = cases()
        .into_iter()
        .find(|c| c["name"] == "7.1-m0-None-vary")
        .unwrap();
    let parts = c["components"].as_array().unwrap();
    let mut d = PresentationDecoder::for_container(48000, 8).unwrap();
    assert!(
        d.decode_packet(&read(parts[0]["raw"].as_str().unwrap()), -19, 16000)
            .unwrap()
            .is_none()
    );
    assert!(d.finish_packet().is_err());
    assert!(d.decode_packet(&read(parts[1]["raw"].as_str().unwrap()),999,0).unwrap().is_none());
    let got=d.finish_packet().unwrap().unwrap();
    assert_eq!((got.source_pts, got.source_duration), (-19, 16000));
    compare(&got.packet.data, &expected(&c)[..768 * 8 * 4], &c);
    assert!(d.finish_packet().unwrap().is_none());
    // A packet can complete the previous window and contain a second complete
    // programme. They must not be merged under the newer source timestamp.
    d.reset();
    let base = read(parts[0]["raw"].as_str().unwrap());
    let dependent = read(parts[1]["raw"].as_str().unwrap());
    assert!(d.decode_packet(&base, -19, 16000).unwrap().is_none());
    let incoming = [dependent.as_slice(), base.as_slice(), dependent.as_slice()].concat();
    let first = d.decode_packet(&incoming, 1000, 16000).unwrap().unwrap();
    let second = d.finish_packet().unwrap().unwrap();
    assert_eq!((first.source_pts, first.source_duration), (-19, 16000));
    assert_eq!((second.source_pts, second.source_duration), (1000, 16000));
    let all = expected(&c);
    compare(&first.packet.data, &all[..768 * 8 * 4], &c);
    compare(&second.packet.data, &all[768 * 8 * 4..2 * 768 * 8 * 4], &c);
    assert!(d.finish_packet().unwrap().is_none());
    d.reset();
    assert!(
        d.decode_packet(&read(parts[1]["raw"].as_str().unwrap()), 0, 0)
            .is_err()
    );
}
#[test]
fn matroska_dispatch_and_worker_preserve_expanded_pcm_masks_rewind_and_seek() {
    for c in cases().into_iter().filter(|c| c["wave_mask"].is_u64()) {
        let mut s = open(c["video"].as_str().unwrap());
        let width = c["channels"].as_u64().unwrap() as usize;
        assert_eq!(s.channels() as usize, width);
        assert_eq!(
            s.speaker_mask().unwrap(),
            c["wave_mask"].as_u64().map(|m| m as u32)
        );
        let got = packets(&mut *s);
        assert_eq!(got.len(), 3);
        assert_eq!(
            s.duration(),
            Some(Duration::from_nanos(
                3 * c["blocks"].as_u64().unwrap() * 256 * 1_000_000_000
                    / c["rate"].as_u64().unwrap()
            )),
            "{} dependent packets must not reset sample-clock duration",
            c["name"]
        );
        let joined = got
            .iter()
            .flat_map(|(_, b)| b.iter().copied())
            .collect::<Vec<_>>();
        compare(&joined, &expected(&c), &c);
        s.rewind();
        assert_eq!(packets(&mut *s), got);
        for (i, (pts, _)) in got.iter().enumerate() {
            assert_eq!(
                *pts,
                Duration::from_nanos(
                    i as u64 * c["blocks"].as_u64().unwrap() * 256 * 1_000_000_000
                        / c["rate"].as_u64().unwrap()
                ),
                "{} selected packet {i} must keep exact sample time",
                c["name"]
            );
        }
        let seek = Duration::from_millis(if c["rate"] == 44100 { 10 } else { 5 });
        let mut wanted = got.clone();
        wanted[0].0 = seek;
        wanted[0].1.drain(
            ..(seek.as_millis() as usize * c["rate"].as_u64().unwrap() as usize / 1000) * width * 4,
        );
        capture::worker_check_with_layout_and_seek_expected(
            open(c["video"].as_str().unwrap()),
            &got,
            seek,
            &wanted,
            c["wave_mask"].as_u64().unwrap() as u32,
        );
        if ["7.1-m0-mixed-vary", "7.1-m0-None-neutral-rate44100"]
            .contains(&c["name"].as_str().unwrap())
        {
            let seek = Duration::from_millis(20);
            let offset =
                (c["rate"].as_u64().unwrap() / 50 - c["blocks"].as_u64().unwrap() * 256) as usize;
            let mut wanted = got[1..].to_vec();
            wanted[0].0 = seek;
            wanted[0].1.drain(..offset * width * 4);
            capture::worker_check_with_layout_and_seek_expected(
                open(c["video"].as_str().unwrap()),
                &got,
                seek,
                &wanted,
                c["wave_mask"].as_u64().unwrap() as u32,
            );
        }
    }
}
#[test]
fn sixteen_channel_video_keeps_semantic_roles_and_refuses_unknown_device_mapping() {
    for c in cases().into_iter().filter(|c| c["channels"] == 16) {
        let source = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(read(c["video"].as_str().unwrap())),
            Default::default(),
        ).unwrap();
        assert_eq!(source.channels(),16);
        assert_eq!(source.speaker_mask().unwrap(),None);
        let error=source.stereo_router().err().expect("native device route must remain explicit");
        assert!(
            error.to_string().contains("extended speaker mask"),
            "{error}"
        );
        let mut reader = fvid::container::webm::WebmReader::open(
            Cursor::new(read(c["video"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        let indices: Vec<_> = reader
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == 2)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(indices.len(), 3 * c["components"].as_array().unwrap().len());
        let mut decoder = PresentationDecoder::for_container(48000, 16).unwrap();
        let mut pcm = vec![];
        for index in indices {
            if let Some(packet) = decoder
                .decode_encoded(&reader.read_packet(index).unwrap(), 0, 0)
                .unwrap()
            {
                pcm.extend(packet.data);
            }
        }

        while let Some(p)=decoder.finish_packet().unwrap() {pcm.extend(p.packet.data);}
        compare(&pcm, &expected(&c), &c);
        assert_eq!(decoder.speakers().unwrap().0.len(), 16);
    }
}
#[path = "support/audio_capture.rs"]
mod capture;
