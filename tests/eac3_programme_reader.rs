#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
use fvid::{
    audio::{AudioStep, AudioStream},
    playback_ac3::{Ac3AudioReader, Limits},
};
use std::{io::Cursor, path::Path, time::Duration};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-programme-reader")
            .join(name),
    )
    .unwrap()
}
fn open(name: &str, id: u8, compatible: bool) -> Ac3AudioReader {
    if compatible {
        Ac3AudioReader::open_compatible_programme(
            Cursor::new(read(&format!("{name}.ec3"))),
            Limits::default(),
            id,
        )
        .unwrap()
    } else {
        Ac3AudioReader::open_programme(
            Cursor::new(read(&format!("{name}.ec3"))),
            Limits::default(),
            id,
        )
        .unwrap()
    }
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
fn elementary_reader_indexes_selected_geometry_pcm_time_rewind_seek_and_worker() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["accepted"], true);
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        for (id, control) in c["controls"].as_array().unwrap().iter().enumerate() {
            for compatible in [false, true] {
                let mut reference = open(control["name"].as_str().unwrap(), 0, compatible);
                let width = reference.channels() as usize;
                let rate = reference.sample_rate();
                let duration = reference.duration();
                let mask = reference.speaker_mask().unwrap();
                let expected = collect(&mut reference);
                assert_eq!(expected.len(), 3);
                let mut stream = open(name, id as u8, compatible);
                assert_eq!(
                    stream.channels() as usize,
                    width,
                    "{name}: programme{id} geometry"
                );
                assert_eq!(stream.sample_rate(), rate);
                assert_eq!(stream.duration(), duration);
                assert_eq!(stream.speaker_mask().unwrap(), mask);
                assert_eq!(stream.ac3().packets(), 3);
                assert_eq!(stream.ac3().programme(), id as u8);
                assert_eq!(stream.audio_track_index(), id);
                let tracks = stream.audio_tracks();
                assert_eq!(tracks.len(), 8);
                assert_eq!(tracks[id].channels as usize, width);
                assert_eq!(tracks[id].sample_rate, rate);
                assert_eq!(tracks[id].name, format!("E-AC-3 programme {id}"));
                for index in 0..stream.ac3().packets() {
                    let bytes = stream.ac3().packet(index);
                    let mut at = 0;
                    while at < bytes.len() {
                        let kind = bytes[at + 2] >> 6;
                        let identity = (bytes[at + 2] >> 3) & 7;
                        if at == 0 {
                            assert_ne!(kind, 1);
                            assert_eq!(identity, id as u8);
                        } else {
                            assert_eq!(kind, 1);
                        }
                        at += ((bytes[at + 2] as usize & 7) * 256 + bytes[at + 3] as usize + 1) * 2;
                    }
                    assert_eq!(at, bytes.len());
                }
                if !compatible {
                    let mut track = Ac3AudioReader::open_track(
                        Cursor::new(read(&format!("{name}.ec3"))),
                        Limits::default(),
                        id,
                    )
                    .unwrap();
                    assert_eq!(track.audio_track_index(), id);
                    assert_eq!(track.ac3().programme(), id as u8);
                    assert!(collect(&mut track) == expected, "{name}: track{id} PCM");
                }

                assert!(
                    collect(&mut stream) == expected,
                    "{name}: programme{id} PCM"
                );
                stream.rewind();
                assert_eq!(collect(&mut stream), expected);
                let seek = Duration::from_millis(40);
                let target = i64::from(rate) / 25;
                assert_eq!(stream.seek_to(target), target);
                let wanted = seek_reference(&expected, seek, rate, width);
                assert_eq!(
                    collect(&mut stream),
                    wanted,
                    "{name}: programme{id} exact seek"
                );
                if !compatible {
                    capture::worker_check_with_compression(
                        Box::new(open(name, id as u8, false)),
                        &expected,
                        seek,
                        &wanted,
                        mask,
                        fvid::codec::ac3_decoder::CompressionMode::Line,
                    );
                }
            }
        }
    }
}

#[test]
fn reader_rejects_missing_ids_and_truncated_tails_and_skips_damaged_unselected_headers() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let c = &m["cases"][0];
    let name = c["name"].as_str().unwrap();
    let raw = read(&format!("{name}.ec3"));
    for id in [8, 255] {
        assert!(Ac3AudioReader::open_programme(Cursor::new(&raw), Limits::default(), id).is_err());
    }
    assert!(Ac3AudioReader::open_track(Cursor::new(&raw), Limits::default(), 8).is_err());
    for id in [1, 7] {
        let mut reference = open(c["controls"][id]["name"].as_str().unwrap(), 0, false);
        let expected = collect(&mut reference);
        let mut stream = open(&format!("corrupt-unselected-p{id}"), id as u8, false);
        assert!(
            collect(&mut stream) == expected,
            "damaged unselected headers must not affect programme{id}"
        );
    }
    assert!(Ac3AudioReader::open_programme(
        Cursor::new(read("without-programme7.ec3")),
        Limits::default(),
        7
    )
    .is_err());
    assert!(Ac3AudioReader::open_track(
        Cursor::new(read("without-programme7.ec3")),
        Limits::default(),
        7
    )
    .is_err());
    let mut stream = open("truncated-unselected-tail", 0, false);
    let mut d = stream.make_decoder().unwrap();
    while let Some(p) = stream.next_packet().unwrap() {
        d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap();
    }
    assert!(stream
        .validate_eof()
        .unwrap_err()
        .to_string()
        .contains("truncated"));
    assert!(Ac3AudioReader::open_programme(
        Cursor::new(&raw),
        Limits {
            file_bytes: raw.len() - 1,
            ..Limits::default()
        },
        1
    )
    .is_err());
    assert!(Ac3AudioReader::open_programme(
        Cursor::new(&raw),
        Limits {
            packets: 2,
            ..Limits::default()
        },
        1
    )
    .is_err());
    assert!(Ac3AudioReader::open_programme(
        Cursor::new(&raw),
        Limits {
            packets: 3,
            ..Limits::default()
        },
        1
    )
    .is_ok());
}
