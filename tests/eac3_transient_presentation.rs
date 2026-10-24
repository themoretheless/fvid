#![cfg(feature = "player")]
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{
        Eac3Decoder, presentation::PresentationDecoder, transient::apply_window,
    },
};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
#[path = "support/audio_capture.rs"]
mod capture;
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-transient-presentation")
            .join(n),
    )
    .unwrap()
}
fn open(n: &str) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(read(n)), Default::default())
            .unwrap(),
    )
}
fn packets(s: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut decoder = s.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(step) = s.next_step().unwrap() {
        match step {
            AudioStep::Encoded(p) => {
                if let Some(p) = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                {
                    if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
                        out.push((p.presentation_time(), p.data));
                    }
                }
            }
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => s.reset_decoder(&mut decoder).unwrap(),
            AudioStep::Pcm(_) => panic!("expected coded E-AC-3"),
        }
    }
    out.extend(
        decoder
            .drain_presented(s)
            .into_iter()
            .map(|p| (p.presentation_time(), p.data)),
    );
    s.validate_eof().unwrap();
    out
}
fn expected(c: &serde_json::Value, correct: bool) -> Vec<u8> {
    expected_cycles(c, correct, 1)
}
fn expected_cycles(c: &serde_json::Value, correct: bool, cycles: usize) -> Vec<u8> {
    let width = c["channels"].as_u64().unwrap() as usize;
    let frame_samples = c["blocks"].as_u64().unwrap() as usize * 256;
    let count = frame_samples * 3 * cycles;
    let mut mixed = vec![0.0f32; count * width];
    for part in c["components"].as_array().unwrap() {
        let key = if cycles == 1 {
            "control_video"
        } else {
            "mp4_control_video"
        };
        let control = packets(&mut *open(part[key].as_str().unwrap()));
        let control: Vec<f32> = control
            .into_iter()
            .flat_map(|(_, b)| {
                b.chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect::<Vec<_>>()
            })
            .collect();
        let roles = part["roles"].as_array().unwrap();
        let base_active: Vec<usize> = part["active_frames"]
            .as_array()
            .map(|v| v.iter().map(|n| n.as_u64().unwrap() as usize).collect())
            .unwrap_or_else(|| vec![0, 1, 2]);
        let active: Vec<usize> = (0..cycles)
            .flat_map(|cycle| base_active.iter().map(move |f| cycle * 3 + f))
            .collect();
        assert_eq!(control.len(), frame_samples * active.len() * roles.len());
        assert!(
            control.iter().all(|v| v.abs() < 0.9),
            "control must not lose headroom before scaling"
        );
        for (coded, role) in roles.iter().enumerate() {
            let native = part["coded_to_native"][coded].as_u64().unwrap() as usize;
            let mut pcm = vec![0.0; 1536 + count];
            for (at, &frame) in active.iter().enumerate() {
                for n in 0..frame_samples {
                    pcm[1536 + frame * frame_samples + n] = control
                        [(at * frame_samples + n) * roles.len() + native]
                        * c["scale"].as_f64().unwrap() as f32;
                }
            }
            if correct && !part["windows"][coded].is_null() {
                let loc = part["windows"][coded]["location"].as_i64().unwrap();
                let len = part["windows"][coded]["length"].as_u64().unwrap() as u8;
                pcm.resize(
                    1536 + count.max(frame_samples * (3 * cycles - 1) + loc as usize),
                    0.0,
                );
                for &f in &active {
                    apply_window(&mut pcm, -1536, f as i64 * frame_samples as i64 + loc, len)
                        .unwrap();
                }
            }
            let output = c["speakers"]
                .as_array()
                .unwrap()
                .iter()
                .position(|s| s == role)
                .unwrap();
            for n in 0..count {
                if active.contains(&(n / frame_samples)) {
                    mixed[n * width + output] = pcm[1536 + n];
                }
            }
        }
    }
    for n in 0..count {
        for ch in 0..width {
            mixed[n * width + ch] = (mixed[n * width + ch]
                * c["gains"][(n / 256) % (frame_samples / 256)]
                    .as_f64()
                    .unwrap() as f32)
                .clamp(-1.0, 1.0);
        }
    }
    mixed.into_iter().flat_map(f32::to_le_bytes).collect()
}
#[test]
fn expanded_transient_precedes_replacement_common_gain_and_clipping() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["expanded_transient_accepted"], true);
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 25);
    for c in cases {
        let rate = c["rate"].as_u64().unwrap() as u32;
        let width = c["channels"].as_u64().unwrap() as u16;
        let want = expected(c, true);
        let uncorrected = expected(c, false);
        assert_ne!(
            want, uncorrected,
            "{} must actually exercise correction",
            c["name"]
        );
        if c["mode"] == "clip" {
            assert!(
                want.chunks_exact(4)
                    .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() == 1.0)
            );
        }
        let raw = read(c["file"].as_str().unwrap());
        let mut decoder = PresentationDecoder::for_container(rate, width).unwrap();
        let got = decoder.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        assert!(
            got == want,
            "{} correction before final assembly (first mismatch {:?})",
            c["name"],
            got.iter().zip(&want).position(|(a, b)| a != b)
        );
        decoder.reset();
        assert_eq!(
            decoder.decode_complete(&raw, 0, 0).unwrap().unwrap().data,
            want
        );
        let mut dispatched = Eac3Decoder::new(&[], rate, width).unwrap();
        assert_eq!(
            dispatched
                .decode_complete(&raw, 0, 0)
                .unwrap()
                .unwrap()
                .data,
            want[256 * usize::from(width) * 4..]
        );
    }
}

#[test]
fn expanded_transient_split_matroska_worker_preserves_clocks_rewind_seek() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["wave_mask"].is_u64())
    {
        let rate = c["rate"].as_u64().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize;
        let count = c["blocks"].as_u64().unwrap() as usize * 256;
        let bytes = expected(c, true);
        let stride = width * 4;
        let wanted: Vec<_> = (0..3)
            .map(|i| {
                (
                    Duration::from_nanos(i as u64 * count as u64 * 1_000_000_000 / rate),
                    bytes[i * count * stride..(i + 1) * count * stride].to_vec(),
                )
            })
            .collect();
        let mut s = open(c["video"].as_str().unwrap());
        assert_eq!(s.channels() as usize, width);
        assert_eq!(
            s.speaker_mask().unwrap(),
            c["wave_mask"].as_u64().map(|v| v as u32)
        );
        let got = packets(&mut *s);
        assert!(got == wanted, "{} original clocks/PCM", c["name"]);
        s.rewind();
        assert!(packets(&mut *s) == wanted, "{} rewind", c["name"]);
        let seek = Duration::from_millis(20);
        let first = (rate / 50) as usize;
        let start = first / count;
        let after: Vec<_> = (start..3)
            .map(|i| {
                (
                    if i == start { seek } else { wanted[i].0 },
                    bytes[(i * count).max(first) * stride..(i + 1) * count * stride].to_vec(),
                )
            })
            .collect();
        capture::worker_check_with_layout_and_seek_expected(
            open(c["video"].as_str().unwrap()),
            &wanted,
            seek,
            &after,
            c["wave_mask"].as_u64().unwrap() as u32,
        );
    }
}

#[test]
fn expanded_transient_packet_groups_are_atomic_and_reset_discards_pending_work() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "7.1-48000-unity")
        .unwrap();
    let want = expected(c, true);
    let raw = read(c["file"].as_str().unwrap());
    let parts = c["components"].as_array().unwrap();
    let base = read(parts[0]["raw"].as_str().unwrap());
    let dep = read(parts[1]["raw"].as_str().unwrap());
    let mut d = PresentationDecoder::for_container(48_000, 8).unwrap();
    // Several programmes in one input must remain one source window even
    // when all but the final programme's blocks have become ready.
    assert!(d.decode_packet(&raw, -17, 48000).unwrap().is_none());
    let output = d.finish_packet().unwrap().unwrap();
    assert_eq!((output.source_pts, output.source_duration), (-17, 48000));
    assert!(output.packet.data == want);
    assert!(d.finish_packet().unwrap().is_none());
    assert!(d.decode_packet(&raw, 1, 1).is_err());
    d.reset();
    let incomplete = [base.as_slice(), dep.as_slice(), base.as_slice()].concat();
    assert!(d.decode_packet(&incomplete, -19, 32000).unwrap().is_none());
    assert!(
        d.finish_packet().is_err(),
        "missing final dependent must remain an error"
    );
    assert!(
        d.decode_packet(&dep, 999, 0).unwrap().is_none(),
        "do not emit a partial earlier source packet"
    );
    let output = d.finish_packet().unwrap().unwrap();
    assert_eq!((output.source_pts, output.source_duration), (-19, 32000));
    assert!(output.packet.data == want[..2 * 768 * 8 * 4]);
    assert!(d.finish_packet().unwrap().is_none());
    d.reset();
    assert!(
        d.decode_packet(&[base.as_slice(), dep.as_slice()].concat(), 44, 768)
            .unwrap()
            .is_none()
    );
    d.reset();
    assert!(d.decode_packet(&raw, 123, 999).unwrap().is_none());
    let output = d.finish_packet().unwrap().unwrap();
    assert_eq!((output.source_pts, output.source_duration), (123, 999));
    assert!(
        output.packet.data == want,
        "reset must discard captured PCM and identities"
    );
    assert!(d.finish_packet().unwrap().is_none());
}

#[test]
fn active_transient_mp4_samples_preserve_both_programmes_eof_and_worker_seek() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let cases: Vec<_> = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mp4"].is_string())
        .collect();
    assert_eq!(cases.len(), 18);
    for c in cases {
        let rate = c["rate"].as_u64().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize;
        let stride = width * 4;
        let bytes = expected_cycles(c, true, 2);
        assert_eq!(bytes.len(), 4608 * stride);
        let wanted: Vec<_> = (0..3)
            .map(|i| {
                (
                    Duration::from_nanos(i as u64 * 1536 * 1_000_000_000 / rate),
                    bytes[i * 1536 * stride..(i + 1) * 1536 * stride].to_vec(),
                )
            })
            .collect();
        let open_mp4 = || -> Box<dyn AudioStream> {
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(read(c["mp4"].as_str().unwrap())),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        let mut stream = open_mp4();
        assert_eq!(
            stream.channels() as usize,
            width,
            "placeholder channelcount must not truncate the mix"
        );
        assert_eq!(
            stream.speaker_mask().unwrap(),
            c["wave_mask"].as_u64().map(|v| v as u32)
        );
        let got = packets(&mut *stream);
        assert!(got == wanted, "{} MP4 PCM/source windows", c["name"]);
        stream.rewind();
        assert!(packets(&mut *stream) == wanted, "{} MP4 rewind", c["name"]);
        let seek = Duration::from_millis(40);
        let first = (rate / 25) as usize;
        let after = vec![
            (seek, bytes[first * stride..3072 * stride].to_vec()),
            (wanted[2].0, bytes[3072 * stride..].to_vec()),
        ];
        capture::worker_check_with_layout_and_seek_expected(
            open_mp4(),
            &wanted,
            seek,
            &after,
            c["wave_mask"].as_u64().unwrap() as u32,
        );
    }
}
