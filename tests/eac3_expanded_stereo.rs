#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::AudioStream,
    codec::eac3_decoder::{
        CompressionMode as Compression, Eac3Decoder, StereoDownmixMode as Mode,
        StereoDownmixPolicy as Policy,
    },
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/eac3-expanded-stereo")
}
fn reader(name: &str, kind: &str, programme: u8) -> Box<dyn AudioStream> {
    let raw = std::fs::read(root().join(format!("{name}.{kind}"))).unwrap();
    match kind {
        "ec3" => Box::new(
            fvid::playback_ac3::Ac3AudioReader::open_programme(
                Cursor::new(raw),
                Default::default(),
                programme,
            )
            .unwrap(),
        ),
        "mkv" => Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open_programme(
                Cursor::new(raw),
                Default::default(),
                0,
                programme,
            )
            .unwrap(),
        ),
        "mp4" => Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open_programme(
                Cursor::new(raw),
                Default::default(),
                0,
                programme,
            )
            .unwrap(),
        ),
        _ => unreachable!(),
    }
}
fn expected(
    name: &str,
    kind: &str,
    programme: u8,
    mode: Mode,
    compression: Compression,
    dialogue: bool,
) -> Vec<(Duration, Vec<u8>)> {
    let bytes = std::fs::read(root().join(format!("{name}-p{programme}-independent.ec3"))).unwrap();
    let mut decoder = if kind == "ec3" {
        Eac3Decoder::new_stereo(&[], 48000)
    } else {
        Eac3Decoder::for_container_stereo(&[], 48000)
    }
    .unwrap()
    .with_stereo_downmix(Policy {
        mode,
        headroom: true,
    })
    .with_compression(compression)
    .with_dialogue_normalization(dialogue);
    let pcm = decoder
        .decode_complete(&bytes, 0, 2304)
        .unwrap()
        .unwrap()
        .data;
    let mut offset = 0;
    let mut result = Vec::new();
    for frames in if kind == "ec3" {
        [512, 768, 768]
    } else {
        [768, 768, 768]
    } {
        result.push((
            Duration::from_nanos(offset as u64 * 1_000_000_000 / 48000),
            pcm[offset * 8..(offset + frames) * 8].to_vec(),
        ));
        offset += frames;
    }
    assert_eq!(pcm.len(), offset * 8);
    assert!(
        pcm.chunks_exact(4)
            .any(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() > 1e-10)
    );
    result
}
#[test]
fn expanded_sources_negotiate_stereo_before_device_mapping_and_keep_native_roles() {
    for (name, width) in [("7.1", 8), ("10", 10), ("16", 16)] {
        for kind in ["ec3", "mkv", "mp4"] {
            for programme in [0, 1] {
                let mut s = reader(name, kind, programme);
                assert_eq!(s.channels(), width);
                let mask = s.speaker_mask().unwrap();
                if width == 16 {
                    assert_eq!(mask, None);
                    assert!(s.stereo_router().is_err());
                } else {
                    assert!(mask.is_some());
                }
                let duration = s.duration();
                s.set_eac3_stereo_output(true).unwrap();
                assert_eq!(s.channels(), 2);
                assert_eq!(s.speaker_mask().unwrap(), Some(3));
                assert_eq!(s.duration(), duration);
                assert_eq!(
                    s.stereo_router().unwrap().unwrap().coefficients(),
                    &[[1., 0.], [0., 1.]]
                );
                assert!(s.make_decoder().is_ok());
                s.set_eac3_stereo_output(false).unwrap();
                assert_eq!(s.channels(), width);
                assert_eq!(s.speaker_mask().unwrap(), mask);
                assert!(s.make_decoder().is_ok());
            }
        }
    }
}
#[test]
fn expanded_worker_uses_independent_downmix_and_drc_for_eof_rewind_and_seek() {
    for name in ["7.1", "10", "16"] {
        for kind in ["ec3", "mkv", "mp4"] {
            for programme in [0, 1] {
                for mode in [Mode::LoRo, Mode::LtRt, Mode::Automatic] {
                    for compression in [Compression::Line, Compression::Rf] {
                        for dialogue in [false, true] {
                            let full = expected(name, kind, programme, mode, compression, dialogue);
                            let mut offset = 0;
                            let mut sought = Vec::new();
                            for (_, p) in &full {
                                let end = offset + p.len() / 8;
                                let first = offset.max(960);
                                if first < end {
                                    sought.push((
                                        Duration::from_nanos(first as u64 * 1_000_000_000 / 48000),
                                        p[(first - offset) * 8..].to_vec(),
                                    ));
                                }
                                offset = end;
                            }
                            capture::worker_check_with_dolby_policy(
                                reader(name, kind, programme),
                                &full,
                                Duration::from_millis(20),
                                &sought,
                                Some(3),
                                fvid::audio_thread::DolbyPlaybackPolicy {
                                    compression,
                                    normalize_dialogue: dialogue,
                                    eac3_stereo_output: true,
                                    ac3_stereo_output: true,
                                    eac3_stereo_downmix: Policy {
                                        mode,
                                        headroom: true,
                                    },
                                    ..Default::default()
                                },
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn witnesses_distinguish_independent_and_full_programme_metadata() {
    use fvid::codec::eac3_decoder::{checked_frame, dependent_frame};
    for name in ["7.1", "10", "16"] {
        for programme in [0, 1] {
            let raw =
                std::fs::read(root().join(format!("{name}-p{programme}-independent.ec3"))).unwrap();
            let h = checked_frame(&raw).unwrap();
            assert_eq!(h.compression[0], Some(128 + programme * 16));
            assert_eq!(h.dialogue[0], 25 + programme);
            assert_eq!(h.downmix.unwrap().preferred, Some(1));
            let line = expected(name, "mp4", programme, Mode::LtRt, Compression::Line, false);
            assert!(line != expected(name, "mp4", programme, Mode::LtRt, Compression::Rf, false));
            assert!(line != expected(name, "mp4", programme, Mode::LtRt, Compression::Line, true));
        }
        let raw = std::fs::read(root().join(format!("{name}.ec3"))).unwrap();
        let mut cursor = 0;
        let mut found = false;
        while cursor < raw.len() {
            let bytes = &raw[cursor..];
            let size = (((bytes[2] as usize & 7) * 256) + bytes[3] as usize + 1) * 2;
            if bytes[2] >> 6 == 1 {
                let dep = dependent_frame(&bytes[..size]).unwrap();
                if dep.frame.compression[0].is_some() {
                    assert_eq!(dep.frame.compression[0], Some(32));
                    found = true;
                }
                assert_eq!(dep.frame.dialogue[0], 15);
            }
            cursor += size;
        }
        assert!(
            found,
            "fixture must carry separate full-programme compression"
        );
    }
}
