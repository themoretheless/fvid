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
            .join("tests/fixtures/playback-errors")
            .join(if n.starts_with("7.1-aht-base51pulse") {
                "eac3-mp4-aht-sounding-base"
            } else if n.starts_with("7.1-spx") {
                "eac3-expanded-spx"
            } else if n.starts_with("7.1-aht-ecpl") {
                "eac3-expanded-enhanced-coupling"
            } else if n.starts_with("7.1-aht-cpl") {
                "eac3-expanded-coupling"
            } else {
                "eac3-mp4-aht"
            })
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let mut c = m["cases"].as_array().unwrap().clone();
    assert_eq!(c.len(), 18);
    let sounding: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors/eac3-mp4-aht-sounding-base/manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(sounding["cases"].as_array().unwrap().len(), 18);
    c.extend(sounding["cases"].as_array().unwrap().iter().cloned());
    let coupled: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors/eac3-expanded-coupling/manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(coupled["cases"].as_array().unwrap().len(), 48);
    c.extend(coupled["cases"].as_array().unwrap().iter().cloned());
    let enhanced: serde_json::Value =
        serde_json::from_slice(
            &std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "tests/fixtures/playback-errors/eac3-expanded-enhanced-coupling/manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(enhanced["cases"].as_array().unwrap().len(), 48);
    c.extend(enhanced["cases"].as_array().unwrap().iter().cloned());
    let spx: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors/eac3-expanded-spx/manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(spx["cases"].as_array().unwrap().len(), 30);
    c.extend(spx["cases"].as_array().unwrap().iter().cloned());
    assert_eq!(c.len(), 162);
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
        // Prove this exercises real hybrid-transform syntax, not a scalar
        // fixture that merely happens to carry an AHT-looking filename.
        let parts = c["components"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        for component in &parts[1..] {
            let header = fvid::codec::eac3_decoder::dependent_frame(&read(
                component["raw"].as_str().unwrap(),
            ))
            .unwrap();
            if c["name"].as_str().unwrap().starts_with("7.1-spx") {
                assert!(header.frame.aht.iter().all(|active| !*active));
                let attenuation = if c["name"].as_str().unwrap().contains("-notch-") {
                    [Some(5), Some(12)]
                } else if c["name"].as_str().unwrap().contains("-edge-") {
                    [Some(15), Some(31)]
                } else {
                    [None, None]
                };
                assert_eq!(&header.frame.spx_attenuation[..2], &attenuation);
            } else {
                assert!(header.frame.aht.iter().any(|active| *active));
            }
            if c["name"].as_str().unwrap().starts_with("7.1-aht-cpl")
                || c["name"].as_str().unwrap().starts_with("7.1-aht-ecpl")
            {
                assert!(header.frame.coupling.iter().all(|active| *active));
                assert!(
                    header.frame.aht[5],
                    "the coupling plane itself must use AHT"
                );
            }
        }
        let base_pcm = packets(&mut *open(parts[0]["control_video"].as_str().unwrap()));
        let sounding = !c["name"]
            .as_str()
            .unwrap()
            .starts_with("7.1-aht-base51zero");
        if sounding {
            let base_header =
                fvid::codec::eac3_decoder::frame(&read(parts[0]["raw"].as_str().unwrap())).unwrap();
            assert_eq!(base_header.blocks, 6);
            assert!(base_header.snr_offsets.is_none());
            assert_eq!(base_header.blocks_state.snr_offsets, 1);
            assert!(base_header.blocks_state.dither_flags);
        }
        let nonzero = base_pcm
            .iter()
            .flat_map(|(_, bytes)| floats(bytes))
            .any(|value| value.abs() > 1e-8);
        assert_eq!(
            nonzero, sounding,
            "base signal must distinguish silence from sounding 5.1"
        );
        if sounding {
            for channel in 0..6 {
                assert!(
                    base_pcm
                        .iter()
                        .flat_map(|(_, bytes)| floats(bytes))
                        .skip(channel)
                        .step_by(6)
                        .any(|v| v.abs() > 1e-8),
                    "base channel {channel} must carry real signal"
                );
            }
        }
        let want = expected(&c);
        assert!(
            floats(&want).iter().any(|value| value.abs() > 1e-8),
            "active dependent signal required"
        );
        let raw = read(c["file"].as_str().unwrap());
        let width = c["channels"].as_u64().unwrap() as u16;
        let mut d =
            PresentationDecoder::for_container(c["rate"].as_u64().unwrap() as u32, width).unwrap();
        let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        compare(&got, &want, &c);
        if c["name"].as_str().unwrap().starts_with("7.1-spx") {
            let pcm = floats(&got);
            let repeated = (0..1536).all(|sample| {
                [4, 5].into_iter().all(|channel| {
                    pcm[(1536 + sample) * 8 + channel] == pcm[(3072 + sample) * 8 + channel]
                })
            });
            assert_eq!(
                repeated,
                c["name"].as_str().unwrap().contains("-blend31-"),
                "{}: dependent SPX noise must advance, signal-only must repeat",
                c["name"]
            );
        }
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
#[path = "support/audio_capture.rs"]
mod capture;

#[test]
fn mp4_expanded_aht_preserves_scalar_oracle_pcm_and_worker_seek() {
    for c in cases() {
        let open = || -> Box<dyn AudioStream> {
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(read(&format!("{}.mp4", c["name"].as_str().unwrap()))),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        let mut stream = open();
        assert_eq!(stream.channels(), 8);
        assert_eq!(stream.speaker_mask().unwrap(), Some(0x63f));
        assert_eq!(stream.duration(), Some(Duration::from_millis(96)));
        let got = packets(&mut *stream);
        assert_eq!(got.len(), 3);
        let joined: Vec<u8> = got.iter().flat_map(|(_, b)| b.iter().copied()).collect();
        compare(&joined, &expected(&c), &c);
        for (index, (pts, data)) in got.iter().enumerate() {
            assert_eq!(*pts, Duration::from_millis(index as u64 * 32));
            assert_eq!(data.len(), 1536 * 8 * 4);
        }
        for millis in [5, 40, 80] {
            let seek = Duration::from_millis(millis);
            let mut wanted = Vec::new();
            for (pts, data) in &got {
                let skip = if seek > *pts {
                    ((seek - *pts).as_nanos() * 48000 / 1_000_000_000) as usize
                } else {
                    0
                };
                if skip >= data.len() / (8 * 4) {
                    continue;
                }
                wanted.push(((*pts).max(seek), data[skip * 8 * 4..].to_vec()));
            }
            capture::worker_check_with_layout_and_seek_expected(open(), &got, seek, &wanted, 0x63f);
        }
    }
}

#[test]
fn expanded_tools_reach_raw_and_split_matroska_workers() {
    for c in cases().into_iter().filter(|c| {
        c["name"].as_str().unwrap().starts_with("7.1-aht-cpl")
            || c["name"].as_str().unwrap().starts_with("7.1-aht-ecpl")
            || c["name"].as_str().unwrap().starts_with("7.1-spx")
    }) {
        for raw in [false, true] {
            let source = || -> Box<dyn AudioStream> {
                if raw {
                    Box::new(
                        fvid::playback_ac3::Ac3AudioReader::open(
                            Cursor::new(read(c["file"].as_str().unwrap())),
                            Default::default(),
                        )
                        .unwrap(),
                    )
                } else {
                    open(c["video"].as_str().unwrap())
                }
            };
            let mut stream = source();
            assert_eq!(stream.channels(), 8);
            assert_eq!(stream.speaker_mask().unwrap(), Some(0x63f));
            let got = packets(&mut *stream);
            assert_eq!(got.len(), 3);
            let joined: Vec<u8> = got
                .iter()
                .flat_map(|(_, bytes)| bytes.iter().copied())
                .collect();
            let want = expected(&c);
            let lead = if raw { 256 } else { 0 };
            compare(&joined, &want[lead * 8 * 4..], &c);
            assert_eq!(
                stream.duration(),
                Some(Duration::from_nanos(
                    (4608 - lead as u64) * 1_000_000_000 / 48000
                ))
            );
            for (index, (pts, _)) in got.iter().enumerate() {
                let sample = if index == 0 { 0 } else { index * 1536 - lead };
                assert_eq!(
                    *pts,
                    Duration::from_nanos(sample as u64 * 1_000_000_000 / 48000)
                );
            }
            let seek = Duration::from_millis(40);
            let mut wanted = got[1..].to_vec();
            wanted[0].0 = seek;
            wanted[0].1.drain(..(1920 - (1536 - lead)) * 8 * 4);
            capture::worker_check_with_layout_and_seek_expected(
                source(),
                &got,
                seek,
                &wanted,
                0x63f,
            );
        }
    }
}
