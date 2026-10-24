#[cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{
    audio::AudioDecode,
    codec::mace_decoder::{MaceCoding, MaceDecoder},
    container::mp4::Mp4Reader,
};
use std::{io::Cursor, path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-mace-transitions")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(format!("{name}.mp4"))).unwrap()
}
fn pcm(name: &str) -> (u32, u16, Vec<u8>) {
    let mut reader = Mp4Reader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
    let index = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let track = &reader.tracks()[index];
    let rate = track.sample_rate;
    let channels = track.channels;
    let count = track.samples.len();
    let coding = if track.codec == *b"MAC3" {
        MaceCoding::ThreeToOne
    } else {
        MaceCoding::SixToOne
    };
    let mut decoder = MaceDecoder::new(coding, rate, channels).unwrap();
    let mut packet = Vec::new();
    let mut out = Vec::new();
    for i in 0..count {
        reader.read_packet(index, i, &mut packet).unwrap();
        out.extend(decoder.decode_encoded(&packet, 0, 0).unwrap().unwrap().data);
    }
    (rate, channels, out)
}
fn edited(rate: u32, ch: u16, source: &[u8]) -> Vec<u8> {
    let stride = usize::from(ch) * 4;
    let first = (u64::from(rate) * 4 / 1000) as usize;
    let last = (u64::from(rate) * 12 / 1000) as usize;
    let mut out = vec![0; (rate as usize * 3 / 1000) * stride];
    out.extend_from_slice(&source[first * stride..last * stride]);
    out.extend(vec![0; (rate as usize * 2 / 1000) * stride]);
    out.extend_from_slice(&source[first * stride..last * stride]);
    out
}
#[test]
fn owned_mace_export_admission_intervals_and_edits_preserve_predictor_history() {
    for name in ["control-0", "control-1", "control-2", "control-6mono"] {
        let (rate, ch, source) = pcm(name);
        for is_edited in [false, true] {
            let stem = if is_edited {
                format!("{name}-edits")
            } else {
                name.to_owned()
            };
            let path = root().join(format!("{stem}.mp4"));
            let wanted = if is_edited {
                edited(rate, ch, &source)
            } else {
                source.clone()
            };
            let stride = usize::from(ch) * 4;
            let dest = std::env::temp_dir().join(format!(
                "fvid-mace-export-{}-{stem}.f32le",
                std::process::id()
            ));
            for interval in [
                None,
                Some((Duration::from_millis(5), Duration::from_millis(14))),
            ] {
                let stats = fvid::native_export::export_audio_pcm_selected(
                    &path, &dest, interval, 1.0, None, None, None, None, None,
                )
                .unwrap();
                let actual = std::fs::read(&dest).unwrap();
                std::fs::remove_file(&dest).unwrap();
                let expected = if interval.is_some() {
                    &wanted
                        [(rate as usize * 5 / 1000) * stride..(rate as usize * 14 / 1000) * stride]
                } else {
                    &wanted
                };
                assert_eq!(
                    actual, expected,
                    "{stem}: PCM history or selected window mismatch"
                );
                assert_eq!(
                    (stats.sample_rate, stats.channels, stats.sample_frames),
                    (rate, ch, expected.len() as u64 / (u64::from(ch) * 4))
                );
            }
            assert!(fvid::native_media::is_owned_audio_source(&path).unwrap());
            let info = fvid::native_media::audio_source_info_selected(&path, None).unwrap();
            assert_eq!(
                info.codec,
                if name == "control-1" || name == "control-6mono" {
                    "mace6"
                } else {
                    "mace3"
                }
            );
        }
    }
}
#[test]
#[cfg(feature = "player")]
fn constant_mace_player_seeks_and_edits_recover_predictors_before_exposing_pcm() {
    use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
    for name in ["control-0", "control-1", "control-2", "control-6mono"] {
        let (rate, ch, source) = pcm(name);
        let stride = usize::from(ch) * 4;
        for is_edited in [false, true] {
            let stem = if is_edited {
                format!("{name}-edits")
            } else {
                name.to_owned()
            };
            let wanted = if is_edited {
                edited(rate, ch, &source)
            } else {
                source.clone()
            };
            let mut reader =
                Mp4AudioReader::open(Cursor::new(bytes(&stem)), Default::default()).unwrap();
            for ms in [0, 1, 4, 5, 7, 10, 13, 14, 20, 200] {
                reader.seek_to(ms * 48);
                let mut decoder = reader.make_decoder().unwrap();
                reader.reset_decoder(&mut decoder).unwrap();
                let actual: Vec<u8> = transitions::collect(&mut reader)
                    .into_iter()
                    .flat_map(|p| p.3)
                    .collect();
                let first =
                    (ms as usize * rate as usize / 1000).min(wanted.len() / stride) * stride;
                assert_eq!(
                    actual,
                    &wanted[first..],
                    "{stem}/seek{ms}: exact PCM suffix"
                );
            }
        }
    }
}

#[test]
#[cfg(feature = "player")]
fn constant_mace_edits_apply_silence_and_repeat_source_with_fresh_predictors() {
    use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
    for name in ["control-0", "control-1", "control-2", "control-6mono"] {
        let (rate, ch, source) = pcm(name);
        let mut reader = Mp4AudioReader::open(
            Cursor::new(bytes(&format!("{name}-edits"))),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            reader.duration(),
            Some(Duration::from_millis(21)),
            "{name}: edit duration"
        );
        let actual: Vec<u8> = transitions::collect(&mut reader)
            .into_iter()
            .flat_map(|f| f.3)
            .collect();
        assert_eq!(
            actual,
            edited(rate, ch, &source),
            "{name}: exact edited PCM"
        );
    }
}

#[cfg(feature = "player")]
fn frames(rate: u32, ch: u16, source: &[u8], is_edited: bool, ms: u64) -> transitions::Frames {
    let stride = usize::from(ch) * 4;
    let target = ms as usize * rate as usize / 1000;
    let mut out = Vec::new();
    let schedules = if is_edited {
        vec![
            (0, 0, rate as usize * 3 / 1000, true),
            (
                rate as usize * 3 / 1000,
                rate as usize * 4 / 1000,
                rate as usize * 12 / 1000,
                false,
            ),
            (rate as usize * 11 / 1000, 0, rate as usize * 2 / 1000, true),
            (
                rate as usize * 13 / 1000,
                rate as usize * 4 / 1000,
                rate as usize * 12 / 1000,
                false,
            ),
        ]
    } else {
        vec![(0, 0, source.len() / stride, false)]
    };
    for (at, first, last, silence) in schedules {
        if silence {
            let skip = target.saturating_sub(at).min(last - first);
            if skip < last - first {
                out.push((
                    rate,
                    ch,
                    Duration::from_nanos((at + skip) as u64 * 1_000_000_000 / u64::from(rate)),
                    vec![0; (last - first - skip) * stride],
                ));
            }
        } else {
            for chunk in (0..source.len() / stride).step_by(6) {
                let begin = chunk.max(first).max(first + target.saturating_sub(at));
                let end = (chunk + 6).min(last);
                if begin < end {
                    out.push((
                        rate,
                        ch,
                        Duration::from_nanos(
                            (at + begin - first) as u64 * 1_000_000_000 / u64::from(rate),
                        ),
                        source[begin * stride..end * stride].to_vec(),
                    ));
                }
            }
        }
    }
    out
}
#[test]
#[cfg(feature = "player")]
fn mace_workers_keep_exact_packet_times_and_history_after_seek_and_rewind() {
    use fvid::{audio_thread::AudioPlayback, playback_mp4_audio::Mp4AudioReader};
    use std::sync::{Arc, Mutex};
    use transitions::{Capture, Trace, compare, ended};
    for name in ["control-0", "control-1", "control-2", "control-6mono"] {
        let (rate, ch, source) = pcm(name);
        for is_edited in [false, true] {
            let stem = if is_edited {
                format!("{name}-edits")
            } else {
                name.to_owned()
            };
            let trace = Arc::new(Mutex::new(Trace::default()));
            let shared = trace.clone();
            let mut audio = AudioPlayback::start(
                Box::new(
                    Mp4AudioReader::open(Cursor::new(bytes(&stem)), Default::default()).unwrap(),
                ),
                move || Box::new(Capture::new(shared)),
            );
            audio.play();
            ended(&audio, 0);
            compare(
                &trace.lock().unwrap().packets,
                &frames(rate, ch, &source, is_edited, 0),
            );
            trace.lock().unwrap().packets.clear();
            audio.rewind();
            audio.play();
            ended(&audio, audio.generation());
            compare(
                &trace.lock().unwrap().packets,
                &frames(rate, ch, &source, is_edited, 0),
            );
            for ms in [1, 5, 10, 13, 14, 20, 200] {
                trace.lock().unwrap().packets.clear();
                audio.seek(Duration::from_millis(ms));
                ended(&audio, audio.generation());
                let actual = trace.lock().unwrap().packets.clone();
                let expected = frames(rate, ch, &source, is_edited, ms);
                assert_eq!(actual.len(), expected.len(), "{stem}/worker seek{ms}");
                compare(&actual, &expected);
            }
        }
    }
}

#[test]
#[cfg(feature = "player")]
fn mace_export_witnesses_keep_exact_owned_video_planes_and_clocks() {
    use fvid::playback_native::{NativeReader, RawFrame};
    for name in ["control-0", "control-1", "control-2", "control-6mono"] {
        for suffix in ["", "-edits"] {
            let mut video =
                NativeReader::software(Cursor::new(bytes(&format!("{name}{suffix}"))), 32 << 20)
                    .unwrap();
            for i in 0..6 {
                match video.read_frame_raw().unwrap().unwrap() {
                    RawFrame::Planar(p) => {
                        assert_eq!((p.depth, p.frame.width, p.frame.height), (8, 16, 16));
                        assert_eq!(p.frame.data, vec![128; 384]);
                    }
                    _ => panic!("owned VP9 must retain exact planes"),
                }
                assert_eq!(video.frame_interval(), Some((i, i + 1, 50)));
            }
            assert!(video.read_frame_raw().unwrap().is_none());
        }
    }
}
