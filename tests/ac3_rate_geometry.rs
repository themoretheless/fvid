#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStep, AudioStream};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(ext: &str, bytes: Vec<u8>) -> Box<dyn AudioStream> {
    match ext {
        "mkv" => Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes),
                Default::default(),
            )
            .unwrap(),
        ),
        "mp4" => Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        _ => unreachable!(),
    }
}
fn first(
    source: &mut dyn AudioStream,
    engine: &mut Box<dyn AudioDecode>,
) -> fvid::audio::EncodedPacket {
    loop {
        match source.next_step().unwrap().unwrap() {
            AudioStep::Encoded(p) => return p,
            AudioStep::ReconfigureDecoder => {
                panic!("unexpected configuration change in constant-setup fixture")
            }
            AudioStep::ResetDecoder => source.reset_decoder(engine).unwrap(),
            AudioStep::Pcm(_) => panic!("unexpected source silence"),
        }
    }
}
#[test]
fn ac3_configured_rate_must_match_clean_frame_headers_without_disabling_downmix() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ac3-rate-geometry");
    let mut failures = Vec::new();
    for (file, rate, channels) in [
        ("48k.ac3", 48000, 2),
        ("32k.ac3", 32000, 1),
        ("48k.ac3", 48000, 1),
    ] {
        let bytes = std::fs::read(base.join(file)).unwrap();
        let mut engine = fvid::codec::make_audio_decoder("A_AC3", &[], rate, channels, 16).unwrap();
        let p = engine
            .decode_packet(&bytes, 0, 1536)
            .unwrap()
            .unwrap()
            .packet;
        assert_eq!(p.data.len() / 4 / usize::from(channels), 1280);
        assert!(
            p.data
                .chunks_exact(4)
                .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.001)
        );
    }
    for (file, rate) in [("48k.ac3", 32000), ("48k.ac3", 44100), ("32k.ac3", 48000)] {
        let bytes = std::fs::read(base.join(file)).unwrap();
        let mut engine = fvid::codec::make_audio_decoder("A_AC3", &[], rate, 1, 16).unwrap();
        match engine.decode_packet(&bytes, 0, 1536) {
            Ok(_) => failures.push(format!(
                "{file} configured{rate}: clean rate mismatch accepted"
            )),
            Err(e) => assert!(
                e.to_string().contains("frame sample rate") && e.to_string().contains("disagrees"),
                "{e}"
            ),
        }
    }
    let damaged = std::fs::read(base.join("damaged-rate.ac3")).unwrap();
    let mut engine = fvid::codec::make_audio_decoder("A_AC3", &[], 48000, 1, 16).unwrap();
    let concealed = engine.decode_packet(&damaged, 0, 1536).unwrap().unwrap();
    assert_eq!(concealed.packet.data.len(), 1280 * 4);
    assert!(
        concealed.packet.data.iter().all(|b| *b == 0),
        "CRC damage must retain concealment policy"
    );
    let changing = std::fs::read(base.join("rate-change.ac3")).unwrap();
    let mut engine = fvid::codec::make_audio_decoder("A_AC3", &[], 48000, 1, 16).unwrap();
    let error = engine
        .decode_packet(&changing, 0, 3072)
        .err()
        .expect("mid-packet native rate change must refuse");
    assert!(
        error.to_string().contains("frame sample rate") && error.to_string().contains("disagrees"),
        "{error}"
    );
    for ext in ["mkv", "mp4"] {
        let good = std::fs::read(base.join(format!("control.{ext}"))).unwrap();
        let bad = std::fs::read(base.join(format!("mismatch.{ext}"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bad.clone()), 32 << 20)
                .unwrap();
        let mut pictures = 0;
        while video.read_frame_raw().unwrap().is_some() {
            pictures += 1;
        }
        assert_eq!(pictures, 6);
        let mut source = open(ext, good.clone());
        let mut engine = source.make_decoder().unwrap();
        let clean = first(source.as_mut(), &mut engine);
        if ext == "mp4" {
            // MP4 validates the coded clock while opening, before constructing
            // a decoder that could label 48 kHz PCM as 32 kHz.
            let error = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(bad.clone()),
                Default::default(),
            )
            .err()
            .expect("mismatched MP4 clock accepted");
            assert!(
                error
                    .to_string()
                    .contains("AC-3 coded clock rate disagrees with description"),
                "{error}"
            );
        } else {
            let mut source = open(ext, bad.clone());
            assert_eq!(source.sample_rate(), 32000);
            let mut engine = source.make_decoder().unwrap();
            let encoded = first(source.as_mut(), &mut engine);
            assert_eq!(encoded.data, clean.data, "only container rate is corrupted");
            match engine.decode_packet(&encoded.data, encoded.pts, encoded.duration as u64) {
                Ok(Some(p)) => {
                    assert!(
                        p.packet.data.chunks_exact(4).any(|b| f32::from_le_bytes(
                            b.try_into().unwrap()
                        )
                        .abs()
                            > 0.001)
                    );
                    failures.push(format!("{ext}: native48000Hz PCM mislabeled32000Hz"));
                }
                Err(e) => {
                    assert!(
                        e.to_string().contains("frame sample rate")
                            && e.to_string().contains("disagrees"),
                        "{e}"
                    );
                    capture::worker_refusal(open(ext, bad), "frame sample rate");
                }
                _ => panic!("no output or precise refusal"),
            }
        }
        let mut source = open(ext, good.clone());
        let mut engine = source.make_decoder().unwrap();
        let mut expected = Vec::new();
        while let Some(step) = source.next_step().unwrap() {
            let packet = match step {
                AudioStep::Encoded(p) => {
                    let f = engine
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    source.present_decoded(f.packet, f.source_pts).unwrap()
                }
                AudioStep::Pcm(p) => Some(p),
                AudioStep::ReconfigureDecoder => {
                    panic!("unexpected configuration change in constant-setup fixture")
                }
                AudioStep::ResetDecoder => {
                    source.reset_decoder(&mut engine).unwrap();
                    None
                }
            };
            if let Some(p) = packet {
                expected.push((p.presentation_time(), p.data));
            }
        }
        source.validate_eof().unwrap();
        capture::worker_check_with_seek(
            open(ext, good),
            &expected,
            Some(Duration::from_millis(120)),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
