#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::Eac3Decoder,
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
    m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["wave_mask"].is_u64())
        .cloned()
        .collect()
}
fn open(c: &serde_json::Value) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_ac3::Ac3AudioReader::open(
            Cursor::new(read(c["file"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap(),
    )
}
fn packets(s: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut d = s.make_decoder().unwrap();
    let mut out = vec![];
    while let Some(step) = s.next_step().unwrap() {
        let p = match step {
            AudioStep::Encoded(p) => d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .and_then(|p| s.present_decoded(p.packet, p.source_pts).unwrap()),
            AudioStep::Pcm(p) => Some(p),
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => {
                s.reset_decoder(&mut d).unwrap();
                None
            }
        };
        if let Some(p) = p {
            out.push((p.presentation_time(), p.data));
        }
    }
    out.extend(d.drain_presented(s).into_iter().map(|p| (p.presentation_time(), p.data)));
    s.validate_eof().unwrap();
    out
}
#[test]
fn raw_reader_advertises_expanded_native_layout_and_groups_each_programme_once() {
    let cases = cases();
    assert_eq!(cases.len(), 21);
    for c in cases {
        let mut source = open(&c);
        let width = c["channels"].as_u64().unwrap() as u16;
        let rate = c["rate"].as_u64().unwrap() as u32;
        let blocks = c["blocks"].as_u64().unwrap();
        assert_eq!(
            source.channels(),
            width,
            "{} raw reader dropped extension channels",
            c["name"]
        );
        assert_eq!(
            source.speaker_mask().unwrap(),
            c["wave_mask"].as_u64().map(|v| v as u32)
        );
        let got = packets(&mut *source);
        assert_eq!(got.len(), 3);
        let expected = Eac3Decoder::new(&[], rate, width)
            .unwrap()
            .decode_complete(&read(c["file"].as_str().unwrap()), 0, 0)
            .unwrap()
            .unwrap()
            .data;
        let joined = got
            .iter()
            .flat_map(|(_, b)| b.iter().copied())
            .collect::<Vec<_>>();
        assert!(joined == expected, "{} raw PCM", c["name"]);
        assert_eq!(
            source.duration(),
            Some(Duration::from_nanos(
                (3 * blocks * 256 - 256) * 1_000_000_000 / u64::from(rate)
            ))
        );
        for (i, (pts, _)) in got.iter().enumerate() {
            let samples = if i == 0 {
                0
            } else {
                i as u64 * blocks * 256 - 256
            };
            assert_eq!(
                *pts,
                Duration::from_nanos(samples * 1_000_000_000 / u64::from(rate))
            );
        }
        source.rewind();
        assert_eq!(packets(&mut *source), got);
    }
}
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn raw_worker_keeps_native_pcm_assignment_rewind_and_second_packet_seek() {
    for c in cases() {
        let mut s = open(&c);
        let expected = packets(&mut *s);
        let width = c["channels"].as_u64().unwrap() as usize;
        let rate = c["rate"].as_u64().unwrap();
        let seek = Duration::from_millis(20);
        let first = c["blocks"].as_u64().unwrap() * 256 - 256;
        let offset = (rate / 50 - first) as usize;
        let mut wanted = expected[1..].to_vec();
        wanted[0].0 = seek;
        wanted[0].1.drain(..offset * width * 4);
        capture::worker_check_with_layout_and_seek_expected(
            open(&c),
            &expected,
            seek,
            &wanted,
            c["wave_mask"].as_u64().unwrap() as u32,
        );
    }
}

#[test]
fn explicit_compatible_reader_keeps_the_independent_mix_and_extended_maps_are_named() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let bytes = read(c["file"].as_str().unwrap());
        let rate = c["rate"].as_u64().unwrap() as u32;
        let mut source = fvid::playback_ac3::Ac3AudioReader::open_compatible(
            Cursor::new(&bytes),
            Default::default(),
        )
        .unwrap();
        assert_eq!(source.channels(), 6);
        assert_eq!(source.speaker_mask().unwrap(), Some(0x60f));
        let got = packets(&mut source)
            .into_iter()
            .flat_map(|(_, b)| b)
            .collect::<Vec<_>>();
        let expected = Eac3Decoder::new(&[], rate, 6)
            .unwrap()
            .decode_complete(&bytes, 0, 0)
            .unwrap()
            .unwrap()
            .data;
        assert!(got == expected, "{} compatible mix", c["name"]);
        if c["wave_mask"].is_null() {
            let native=fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(bytes),Default::default()).unwrap();
            assert_eq!(native.channels(),16);
            assert_eq!(native.speaker_mask().unwrap(),None);
            let error=native.stereo_router().err().expect("unmapped native device roles must be named");
            assert!(
                error.to_string().contains("extended speaker mask"),
                "{error}"
            );
        }
    }
}

#[test]
fn truncated_extension_preserves_complete_programmes_and_is_reported() {
    let mut source = fvid::playback_ac3::Ac3AudioReader::open(
        Cursor::new(read("truncated-expanded-tail.ec3")),
        Default::default(),
    )
    .unwrap();
    assert_eq!(source.channels(), 8);
    assert_eq!(source.ac3().packets(), 2);
    assert_eq!(source.ac3().samples(), 1536);
    let mut decoder = source.make_decoder().unwrap();
    let mut frames = 0;
    while let Some(step) = source.next_step().unwrap() {
        if let AudioStep::Encoded(p) = step {
            if let Some(d) = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
            {
                frames += source
                    .present_decoded(d.packet, d.source_pts)
                    .unwrap()
                    .unwrap()
                    .data
                    .len()
                    / 32;
            }
        }
    }
    frames+=decoder.drain_presented(&mut source).iter().map(|p|p.data.len()/32).sum::<usize>();
    assert_eq!(frames, 1280);
    assert!(decoder.finish_packet().unwrap().is_none());
    assert!(source.validate_eof().is_err());
    let mut video = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(read("truncated-expanded-tail.mkv")),
        Default::default(),
    )
    .unwrap();
    for _ in 0..5 {
        assert!(video.next_step().unwrap().is_some());
    }
    let error = video
        .next_step()
        .err()
        .expect("truncated dependent frame must be refused");
    assert!(
        error
            .to_string()
            .contains("truncated skipped E-AC-3 substream"),
        "{error}"
    );
}

#[test]
fn mp4_six_block_samples_infer_expanded_channels_and_preserve_pcm() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/eac3-mp4-presentation");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 21);
    for c in manifest["cases"].as_array().unwrap() {
        let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(std::fs::read(root.join(c["file"].as_str().unwrap())).unwrap()),
            Default::default(),
        )
        .unwrap();
        let rate = c["rate"].as_u64().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        assert_eq!(stream.channels(), channels as u16, "{}", c["file"]);
        let descriptions = stream.audio_tracks();
        assert_eq!(descriptions.len(), 1);
        assert_eq!(
            descriptions[0].channels, channels as u16,
            "{} track metadata",
            c["file"]
        );
        assert_eq!(
            stream.speaker_mask().unwrap(),
            Some(c["mask"].as_u64().unwrap() as u32)
        );
        assert_eq!(
            stream.duration(),
            Some(Duration::from_nanos(4608 * 1_000_000_000 / rate))
        );
        let first = packets(&mut stream);
        assert_eq!(first.len(), 3);
        for (index, (pts, data)) in first.iter().enumerate() {
            assert_eq!(
                *pts,
                Duration::from_nanos(index as u64 * 1536 * 1_000_000_000 / rate)
            );
            assert_eq!(data.len(), 1536 * channels * 4);
        }
        let got: Vec<u8> = first
            .iter()
            .flat_map(|(_, data)| data.iter().copied())
            .collect();
        let raw = read(c["source"].as_str().unwrap()).repeat(2);
        let mut decoder = Eac3Decoder::new(&[], rate as u32, channels as u16).unwrap();
        let expected = decoder
            .decode_complete(&raw, 0, 4608)
            .unwrap()
            .unwrap()
            .data;
        assert_eq!(got.len(), 4608 * channels * 4);
        assert_eq!(&got[256 * channels * 4..], expected, "{}", c["file"]);
        stream.rewind();
        assert_eq!(packets(&mut stream), first);
    }
}

#[test]
fn mp4_expanded_worker_preserves_partial_sample_seeks_and_rewind() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/eac3-mp4-presentation");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let open = || -> Box<dyn AudioStream> {
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(std::fs::read(root.join(c["file"].as_str().unwrap())).unwrap()),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        let rate = c["rate"].as_u64().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let expected = packets(&mut *open());
        for millis in [20, 40, 80] {
            let seek = Duration::from_millis(millis);
            let mut wanted = Vec::new();
            for (pts, data) in &expected {
                let frames = data.len() / (channels * 4);
                let skip = if seek > *pts {
                    ((seek - *pts).as_nanos() * u128::from(rate) / 1_000_000_000) as usize
                } else {
                    0
                };
                if skip >= frames {
                    continue;
                }
                wanted.push(((*pts).max(seek), data[skip * channels * 4..].to_vec()));
            }
            capture::worker_check_with_layout_and_seek_expected(
                open(),
                &expected,
                seek,
                &wanted,
                c["mask"].as_u64().unwrap() as u32,
            );
        }
    }
}

#[test]
fn expanded_mp4_track_descriptions_match_both_selection_and_decoded_pcm() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/eac3-mp4-presentation");
    let bytes = std::fs::read(root.join("two-expanded-audio-tracks.mp4")).unwrap();
    let mut expected = None;
    for selected in [0, 1] {
        let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open_at(
            Cursor::new(bytes.clone()),
            Default::default(),
            selected,
        )
        .unwrap();
        assert_eq!(stream.audio_track_index(), selected);
        assert_eq!(stream.channels(), 8);
        let tracks = stream.audio_tracks();
        assert_eq!(tracks.len(), 2);
        assert!(
            tracks
                .iter()
                .all(|track| track.channels == 8 && track.sample_rate == 48000)
        );
        let got = packets(&mut stream);
        assert_eq!(
            got.iter().map(|(_, data)| data.len()).sum::<usize>(),
            4608 * 8 * 4
        );
        if let Some(want) = &expected {
            assert_eq!(&got, want);
        } else {
            expected = Some(got.clone());
        }
        stream.rewind();
        assert_eq!(stream.audio_tracks(), tracks);
        assert_eq!(packets(&mut stream), got);
    }
}
