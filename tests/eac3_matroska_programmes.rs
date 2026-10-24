#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
use fvid::{
    audio::{AudioStep, AudioStream},
    container::webm::Limits,
    playback_webm_audio::WebmAudioReader,
};
use std::{io::Cursor, path::Path, time::Duration};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-matroska-programmes")
            .join(name),
    )
    .unwrap()
}
fn open(name: &str, programme: u8) -> WebmAudioReader<Cursor<Vec<u8>>> {
    WebmAudioReader::open_programme(
        Cursor::new(read(&format!("{name}.mkv"))),
        Limits::default(),
        0,
        programme,
    )
    .unwrap()
}
fn reference(name: &str) -> WebmAudioReader<Cursor<Vec<u8>>> {
    WebmAudioReader::open_at(
        Cursor::new(read(&format!("{name}.mkv"))),
        Limits::default(),
        0,
    )
    .unwrap()
}
fn collect(s: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut d = s.make_decoder().unwrap();
    let mut out = vec![];
    while let Some(step) = s.next_step().unwrap() {
        if let AudioStep::Encoded(p) = step {
            if let Some(p) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
                    out.push((p.presentation_time(), p.data));
                }
            }
        }
    }
    while let Some(p) = d.finish_packet().unwrap() {
        if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
            out.push((p.presentation_time(), p.data));
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
fn selected_matroska_programme_supplies_pcm_geometry_clock_navigation_and_worker() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["accepted"], true);
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 11);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        for (id, control) in c["controls"].as_array().unwrap().iter().enumerate() {
            let mut wanted = reference(control["name"].as_str().unwrap());
            let rate = wanted.sample_rate();
            let width = wanted.channels() as usize;
            let mask = wanted.speaker_mask().unwrap();
            let expected = collect(&mut wanted);
            assert_eq!(expected.len(), 3);
            let mut s = open(name, id as u8);
            assert_eq!(s.sample_rate(), rate, "{name}: programme{id} rate");
            assert_eq!(s.channels() as usize, width, "{name}: programme{id} width");
            assert_eq!(s.speaker_mask().unwrap(), mask);
            assert_eq!(s.audio_track_index(), 0);
            let tracks = s.audio_tracks();
            assert_eq!(tracks.len(), 1);
            assert_eq!(
                (tracks[0].sample_rate, tracks[0].channels),
                (rate, width as u16)
            );

            assert!(collect(&mut s) == expected, "{name}: programme{id} PCM/PTS");
            assert_eq!(s.duration(), wanted.duration());
            s.rewind();
            assert!(collect(&mut s) == expected);
            let seek = Duration::from_millis(40);
            assert_eq!(s.seek_to(40_000_000), 40_000_000);
            let sought = seek_reference(&expected, seek, rate, width);
            assert!(
                collect(&mut s) == sought,
                "{name}: programme{id} exact seek"
            );
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
fn selected_programme_setup_rejects_missing_corrupt_or_non_eac3_and_ignores_unselected_bsi() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let first = &m["cases"][0];
    for id in [1, 7] {
        let mut correct = reference(first["controls"][id]["name"].as_str().unwrap());
        let expected = collect(&mut correct);
        let mut source = open(&format!("corrupt-unselected-p{id}"), id as u8);
        assert!(
            collect(&mut source) == expected,
            "unselected BSI must not affect programme{id}"
        );
    }
    for (name, nth, id) in [
        ("without-programme7", 0, 7),
        ("corrupt-unselected-p1", 0, 7),
        ("pcm", 0, 0),
        ("pcm", 0, 1),
        ("heterogeneous", 1, 0),
        ("heterogeneous", 0, 8),
    ] {
        assert!(
            WebmAudioReader::open_programme(
                Cursor::new(read(&format!("{name}.mkv"))),
                Limits::default(),
                nth,
                id
            )
            .is_err(),
            "{name}: invalid selection must not fall back"
        );
    }
}
