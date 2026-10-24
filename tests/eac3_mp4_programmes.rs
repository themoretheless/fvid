#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
use fvid::{
    audio::{AudioStep, AudioStream},
    container::mp4::Limits,
    playback_mp4_audio::Mp4AudioReader,
};
use std::{io::Cursor, path::Path, time::Duration};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-mp4-programmes")
            .join(name),
    )
    .unwrap()
}
fn open(name: &str, programme: u8) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open_programme(
        Cursor::new(read(&format!("{name}.mp4"))),
        Limits::default(),
        0,
        programme,
    )
    .unwrap()
}
fn reference(name: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open_at(
        Cursor::new(read(&format!("{name}.mp4"))),
        Limits::default(),
        0,
    )
    .unwrap()
}
fn collect(s: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    collect_policy(s, None)
}
fn collect_policy(
    s: &mut dyn AudioStream,
    policy: Option<(fvid::codec::ac3_decoder::CompressionMode, bool)>,
) -> Vec<(Duration, Vec<u8>)> {
    let mut d: Box<dyn fvid::audio::AudioDecode> = if let Some((mode, normalize)) = policy {
        Box::new(
            fvid::codec::eac3_decoder::Eac3Decoder::for_container_with_compression(
                s.extra_data(),
                s.sample_rate(),
                s.channels(),
                mode,
            )
            .unwrap()
            .with_dialogue_normalization(normalize),
        )
    } else {
        s.make_decoder().unwrap()
    };
    let mut out = vec![];
    while let Some(step) = s.next_step().unwrap() {
        match step {
            AudioStep::Pcm(p) => out.push((p.presentation_time(), p.data)),
            AudioStep::ResetDecoder => s.reset_decoder(&mut d).unwrap(),
            AudioStep::Encoded(p) => {
                if let Some(p) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                    if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
                        out.push((p.presentation_time(), p.data));
                    }
                }
            }
        }
    }
    if s.drain_decoded_at_eof() {
        while let Some(p) = d.finish_packet().unwrap() {
            if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
                out.push((p.presentation_time(), p.data));
            }
        }
    }
    s.validate_eof().unwrap();
    out
}
fn seek_reference(
    expected: &[(Duration, Vec<u8>)],
    seek: Duration,
    rate: u32,
    width: usize,
) -> Vec<(Duration, Vec<u8>)> {
    let mut skip = (seek.as_nanos() * u128::from(rate) / 1_000_000_000) as usize;
    let mut out = Vec::new();
    for (pts, data) in expected {
        let frames = data.len() / (width * 4);
        if skip >= frames {
            skip -= frames;
            continue;
        }
        let mut b = data.clone();
        let at = if out.is_empty() {
            b.drain(..skip * width * 4);
            skip = 0;
            seek
        } else {
            *pts
        };
        out.push((at, b));
    }
    out
}
#[test]
fn selected_mp4_programme_supplies_geometry_pcm_edits_navigation_and_worker() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["accepted"], true);
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 20);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        for (id, control) in c["controls"].as_array().unwrap().iter().enumerate() {
            let mut baseline = reference(control["name"].as_str().unwrap());
            let rate = baseline.sample_rate();
            let width = baseline.channels() as usize;
            let mask = baseline.speaker_mask().unwrap();
            let expected = collect(&mut baseline);
            assert!(
                expected
                    .iter()
                    .flat_map(|(_, b)| b.chunks_exact(4))
                    .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 1e-7),
                "{name}: nonzero waveform control"
            );
            let mut cursor = 0u64;
            for (pts, b) in &expected {
                assert_eq!(
                    *pts,
                    Duration::from_nanos(cursor * 1_000_000_000 / u64::from(rate))
                );
                cursor += (b.len() / (width * 4)) as u64;
            }
            if c["edits"] == true {
                let mut plain = reference(control["plain"].as_str().unwrap());
                let plain: Vec<u8> = collect(&mut plain)
                    .into_iter()
                    .flat_map(|(_, b)| b)
                    .collect();
                let mut wanted = vec![0; 960 * width * 4];
                wanted.extend_from_slice(&plain[256 * width * 4..1792 * width * 4]);
                wanted.extend_from_slice(&plain[256 * width * 4..1792 * width * 4]);
                assert!(
                    expected
                        .iter()
                        .flat_map(|(_, b)| b.iter().copied())
                        .eq(wanted),
                    "{name}: independent edit-window oracle"
                );
                assert_eq!(cursor, 4032);
            } else {
                assert_eq!(cursor, 4608);
                assert_eq!(expected.len(), 3);
            }
            let mut s = open(name, id as u8);
            assert_eq!(
                (s.sample_rate(), s.channels()),
                (rate, width as u16),
                "{name}: programme{id} geometry"
            );
            assert_eq!(s.speaker_mask().unwrap(), mask);
            assert_eq!(s.audio_track_index(), 0);
            let tracks = s.audio_tracks();
            assert_eq!(
                (tracks[0].sample_rate, tracks[0].channels),
                (rate, width as u16)
            );
            assert!(collect(&mut s) == expected, "{name}: programme{id} PCM/PTS");
            assert_eq!(s.duration(), baseline.duration());
            s.rewind();
            assert!(collect(&mut s) == expected);
            let seek = Duration::from_millis(40);
            let ticks = i64::from(s.timescale()) / 25;
            assert_eq!(s.seek_to(ticks), ticks);
            let sought = seek_reference(&expected, seek, rate, width);
            assert!(collect(&mut s) == sought, "{name}: programme{id} seek");
            capture::worker_check_with_compression(
                Box::new(open(name, id as u8)),
                &expected,
                seek,
                &sought,
                mask,
                fvid::codec::ac3_decoder::CompressionMode::Line,
            );
        }
    }
}
#[test]
fn multiple_used_sample_descriptions_have_an_explicit_refusal_witness() {
    let error = Mp4AudioReader::open_at(
        Cursor::new(read("multiple-descriptions.mp4")),
        Limits::default(),
        0,
    )
    .err()
    .expect("current parser must explicitly refuse this unsupported case");
    assert!(
        error.to_string().contains("multiple sample descriptions"),
        "{error}"
    );
}
#[test]
#[ignore = "MP4 multiple sample descriptions are not implemented; passing refusal is not playback acceptance"]
fn multiple_used_sample_descriptions_play_without_changing_pcm() {
    let mut expected = reference("stereo-48000");
    let wanted = collect(&mut expected);
    let mut stream = open("multiple-descriptions", 0);
    assert!(collect(&mut stream) == wanted);
}

#[test]
fn physical_track_choice_and_unselected_payloads_do_not_override_programme_choice() {
    for id in [1, 7] {
        let mut control = reference(&format!("dual-48000-p{id}-control"));
        let expected = collect(&mut control);
        let mut stream = open(&format!("corrupt-unselected-p{id}"), id);
        assert!(
            collect(&mut stream) == expected,
            "unselected corrupt payloads must not affect programme{id}"
        );
    }
    let mut expected = reference("heterogeneous-48000-p3-control");
    let wanted = collect(&mut expected);
    let mut second = Mp4AudioReader::open_programme(
        Cursor::new(read("two-physical-tracks.mp4")),
        Limits::default(),
        1,
        3,
    )
    .unwrap();
    assert_eq!(second.audio_track_index(), 1);
    assert_eq!(second.track().id, 3);
    let tracks = second.audio_tracks();
    assert_eq!(tracks.len(), 2);
    assert_eq!((tracks[0].channels, tracks[1].channels), (2, 4));
    assert_eq!(second.channels(), 4);
    assert!(
        collect(&mut second) == wanted,
        "physical and encoded programme choices are independent"
    );
    for (name, nth, id) in [
        ("without-programme7", 0, 7),
        ("corrupt-unselected-p1", 0, 7),
        ("dual-48000", 1, 0),
        ("dual-48000", 0, 8),
    ] {
        assert!(
            Mp4AudioReader::open_programme(
                Cursor::new(read(&format!("{name}.mp4"))),
                Limits::default(),
                nth,
                id
            )
            .is_err(),
            "{name}: invalid selection must not fall back"
        );
    }
}

#[test]
fn selected_expanded_worker_keeps_rf_and_dialogue_policy_through_seek() {
    use fvid::{audio_thread::DolbyPlaybackPolicy, codec::ac3_decoder::CompressionMode};
    let name = "expanded-policy-44100-b6";
    let control = format!("{name}-p7-control");
    let mut reference = reference(&control);
    let rate = reference.sample_rate();
    let width = reference.channels() as usize;
    let mask = reference.speaker_mask().unwrap();
    let expected = collect_policy(&mut reference, Some((CompressionMode::Rf, true)));
    let mut plain = Mp4AudioReader::open_at(
        Cursor::new(read(&format!("{control}.mp4"))),
        Limits::default(),
        0,
    )
    .unwrap();
    assert_ne!(
        expected,
        collect_policy(&mut plain, Some((CompressionMode::Rf, false))),
        "fixture must distinguish normalization"
    );
    let seek = Duration::from_millis(40);
    let wanted = seek_reference(&expected, seek, rate, width);
    capture::worker_check_with_dolby_policy(
        Box::new(open(name, 7)),
        &expected,
        seek,
        &wanted,
        mask,
        DolbyPlaybackPolicy {
            compression: CompressionMode::Rf,
            normalize_dialogue: true,
        },
    );
}
