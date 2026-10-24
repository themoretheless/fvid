#![cfg(feature = "player")]
use fvid::audio::{AudioStep, AudioStream};
use std::{io::Cursor, path::Path, time::Duration};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/ac3-mp4-channels")
            .join(name),
    )
    .unwrap()
}
fn open(name: &str) -> Box<dyn AudioStream> {
    if name.ends_with(".mkv") {
        Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(read(name)),
                Default::default(),
            )
            .unwrap(),
        )
    } else {
        Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(read(name)),
                Default::default(),
            )
            .unwrap(),
        )
    }
}
fn packets(s: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut decoder = s.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(step) = s.next_step().unwrap() {
        let packet = match step {
            AudioStep::Encoded(p) => decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .and_then(|d| s.present_decoded(d.packet, d.source_pts).unwrap()),
            AudioStep::Pcm(p) => Some(p),
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => {
                s.reset_decoder(&mut decoder).unwrap();
                None
            }
        };
        if let Some(p) = packet {
            out.push((p.presentation_time(), p.data));
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    s.validate_eof().unwrap();
    out
}
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn ac3_container_metadata_preserves_native_channels_pcm_track_list_and_seek() {
    let mut raw = fvid::playback_ac3::Ac3AudioReader::open(
        Cursor::new(read("control.ac3")),
        Default::default(),
    )
    .unwrap();
    assert_eq!(raw.channels(), 6);
    let raw_pcm: Vec<u8> = packets(&mut raw).into_iter().flat_map(|(_, b)| b).collect();
    let mut control = None;
    for name in [
        "placeholder.mp4",
        "native-six.mp4",
        "metadata-two.mkv",
        "native-six.mkv",
    ] {
        let mut stream = open(name);
        assert_eq!(
            stream.channels(),
            6,
            "{name}: derive output from AC3 syncframe, not inconsistent container channel metadata"
        );
        assert_eq!(stream.speaker_mask().unwrap(), Some(0x60f));
        assert_eq!(stream.audio_tracks()[0].channels, 6);
        assert_eq!(stream.duration(), Some(Duration::from_millis(96)));
        let got = packets(&mut *stream);
        assert_eq!(got.len(), 3);
        let joined: Vec<u8> = got.iter().flat_map(|(_, b)| b.iter().copied()).collect();
        assert_eq!(joined.len(), 4608 * 6 * 4);
        assert_eq!(
            &joined[256 * 6 * 4..],
            raw_pcm,
            "{name}: native PCM assignment"
        );
        for (index, (pts, bytes)) in got.iter().enumerate() {
            assert_eq!(*pts, Duration::from_millis(index as u64 * 32));
            assert_eq!(bytes.len(), 1536 * 6 * 4);
        }
        if let Some(want) = &control {
            assert_eq!(&got, want);
        } else {
            control = Some(got.clone());
        }
        stream.rewind();
        assert_eq!(packets(&mut *stream), got);
        let seek = Duration::from_millis(40);
        let mut wanted = got[1..].to_vec();
        wanted[0].0 = seek;
        wanted[0].1.drain(..384 * 6 * 4);
        capture::worker_check_with_layout_and_seek_expected(open(name), &got, seek, &wanted, 0x60f);
    }
}

#[test]
fn damaged_ac3_header_cannot_override_container_geometry_before_concealment() {
    let raw = read("bad-first-header.ac3");
    let parsed = fvid::codec::ac3_decoder::syncframe(&raw).unwrap();
    assert!(fvid::codec::ac3_decoder::checked_syncframe(&raw).is_none());
    assert_eq!(
        fvid::codec::ac3_decoder::checked_syncframe(&read("control.ac3"))
            .unwrap()
            .channels,
        6
    );
    assert_ne!(
        parsed.channels, 6,
        "fixture must corrupt native channel geometry"
    );
    let mut decoder = fvid::codec::make_audio_decoder("A_AC3", &[], 48000, 6, 16).unwrap();
    let decoded = decoder
        .decode_packet(&raw, 0, 4608)
        .unwrap()
        .unwrap()
        .packet
        .data;
    let mut expected = vec![0; 256 * 6 * 4];
    expected.extend(decoded);
    assert_eq!(expected.len(), 4608 * 6 * 4);
    assert!(expected[..1536 * 6 * 4].iter().all(|byte| *byte == 0));
    assert!(
        expected[1536 * 6 * 4..]
            .chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 1e-4)
    );
    for name in ["bad-first-header.mp4", "bad-first-header.mkv"] {
        let mut stream = open(name);
        assert_eq!(
            stream.channels(),
            6,
            "{name}: CRC-invalid header must not define output geometry"
        );
        assert_eq!(stream.audio_tracks()[0].channels, 6);
        assert_eq!(
            stream.speaker_mask().unwrap(),
            None,
            "do not infer roles from untrusted header"
        );
        let got = packets(&mut *stream);
        let joined: Vec<u8> = got.iter().flat_map(|(_, b)| b.iter().copied()).collect();
        assert_eq!(
            joined, expected,
            "{name}: conceal first frame, preserve native following PCM"
        );
        stream.rewind();
        assert_eq!(packets(&mut *stream), got);
        let seek = Duration::from_millis(40);
        let mut wanted = got[1..].to_vec();
        wanted[0].0 = seek;
        wanted[0].1.drain(..384 * 6 * 4);
        capture::worker_check_with_seek_expected(open(name), &got, seek, &wanted);
    }
}
