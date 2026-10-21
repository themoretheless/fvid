#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::audio::{AudioStep, AudioStream};
use std::{io::Cursor, path::Path, time::Duration};
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-header-crc-geometry")
            .join(n),
    )
    .unwrap()
}
fn open(n: &str) -> Box<dyn AudioStream> {
    if n.ends_with("mp4") {
        Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(read(n)),
                Default::default(),
            )
            .unwrap(),
        )
    } else {
        Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(read(n)),
                Default::default(),
            )
            .unwrap(),
        )
    }
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
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn crc_invalid_eac3_header_does_not_define_container_geometry() {
    let bad = read("bad-first.ec3");
    assert_eq!(fvid::codec::eac3_decoder::frame(&bad).unwrap().channels, 2);
    assert!(fvid::codec::eac3_decoder::checked_frame(&bad).is_none());
    assert_eq!(
        fvid::codec::eac3_decoder::checked_frame(&read("control.ec3"))
            .unwrap()
            .channels,
        3
    );
    let mut d = fvid::codec::eac3_decoder::Eac3Decoder::new(&[], 48000, 3).unwrap();
    use fvid::audio::AudioDecode;
    let decoded = d.decode_complete(&bad, 0, 4608).unwrap().unwrap().data;
    let mut wanted = vec![0; 256 * 3 * 4];
    wanted.extend(decoded);
    assert_eq!(wanted.len(), 4608 * 3 * 4);
    assert!(wanted[..1536 * 3 * 4].iter().all(|v| *v == 0));
    assert!(
        wanted[1536 * 3 * 4..]
            .chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 1e-8)
    );
    for n in ["bad-first.mp4", "bad-first.mkv"] {
        let mut s = open(n);
        assert_eq!(
            s.channels(),
            3,
            "{n}: damaged header must not redefine channels"
        );
        assert_eq!(s.speaker_mask().unwrap(), None);
        let got = packets(&mut *s);
        assert_eq!(
            got.iter()
                .flat_map(|(_, b)| b.iter().copied())
                .collect::<Vec<_>>(),
            wanted
        );
        s.rewind();
        assert_eq!(packets(&mut *s), got);
        let seek = Duration::from_millis(40);
        let mut partial = got[1..].to_vec();
        partial[0].0 = seek;
        partial[0].1.drain(..384 * 3 * 4);
        capture::worker_check_with_seek_expected(open(n), &got, seek, &partial);
    }
}
