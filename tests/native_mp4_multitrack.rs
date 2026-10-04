//! MP4 presentation edits feed owned filtering and atomic lossless export.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn raw_frames(path: &Path) -> Vec<Vec<u8>> {
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(path).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        frames.push(match frame {
            fvid::playback_native::RawFrame::Planar(frame) => frame.frame.data.clone(),
            fvid::playback_native::RawFrame::Planar8(frame) => {
                [frame.y.as_slice(), frame.cb.as_slice(), frame.cr.as_slice()].concat()
            }
            fvid::playback_native::RawFrame::Avc { picture, .. } => {
                let mut data = Vec::new();
                picture.write_planar(&mut data).unwrap();
                data
            }
            _ => panic!("expected planar compressed-video output"),
        });
    }
    frames
}

fn packets(
    reader: &mut fvid_media::owned_webm::WebmReader<Cursor<Vec<u8>>>,
    track: u64,
) -> Vec<(i64, Option<u64>, i64, Vec<u8>)> {
    let mut out = Vec::new();
    for i in 0..reader.packets.len() {
        let p = reader.packets[i].clone();
        if p.track == track {
            out.push((
                p.pts_ns,
                p.duration_ns,
                p.discard_padding_ns,
                reader.read_packet(i).unwrap(),
            ));
        }
    }
    out
}
#[test]
fn owned_filter_export_keeps_every_aac_track_packet_clock_and_metadata() {
    for name in ["shared-mp4-av.mp4", "shared-mp4-av-multiple.mp4"] {
        let source = fixture(&format!("playback-errors/{name}"));
        let mut input = fvid_media::owned_mp4::Mp4Reader::open(
            Cursor::new(std::fs::read(&source).unwrap()),
            Default::default(),
        )
        .unwrap();
        let video = input
            .tracks()
            .iter()
            .position(|t| t.handler == *b"vide")
            .unwrap();
        let audio: Vec<_> = input
            .tracks()
            .iter()
            .enumerate()
            .filter(|(_, t)| t.handler == *b"soun")
            .map(|(i, _)| i)
            .collect();
        let mut original_packets = Vec::new();
        for &track in &audio {
            let mut data = Vec::new();
            let mut track_packets = Vec::new();
            for i in 0..input.tracks()[track].samples.len() {
                input.read_packet(track, i, &mut data).unwrap();
                track_packets.push(data.clone());
            }
            original_packets.push(track_packets);
        }
        let mut reference = Cursor::new(Vec::new());
        fvid_media::owned_mp4_matroska::write_selected(
            &mut input,
            &mut reference,
            None,
            None,
            None,
            &audio,
        )
        .unwrap();
        let mut control = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(reference.into_inner()),
            Default::default(),
        )
        .unwrap();
        control.scan_all().unwrap();
        let originals = raw_frames(&source);
        for (operation, transform, expected) in [
            (
                "negate",
                fvid_media::LosslessTransform {
                    negate: Some("".into()),
                    ..Default::default()
                },
                originals
                    .iter()
                    .map(|p| p.iter().map(|v| 255 - v).collect())
                    .collect::<Vec<Vec<u8>>>(),
            ),
            (
                "reverse",
                fvid_media::LosslessTransform {
                    reverse: Some("".into()),
                    ..Default::default()
                },
                originals.iter().rev().cloned().collect(),
            ),
            (
                "step",
                fvid_media::LosslessTransform {
                    framestep: Some("2".into()),
                    ..Default::default()
                },
                originals.iter().step_by(2).cloned().collect(),
            ),
        ] {
            let output = std::env::temp_dir().join(format!(
                "fvid-multitrack-{}-{name}-{operation}.mkv",
                std::process::id()
            ));
            let mut options = fvid_control::CopyOptions {
                metadata_set: vec![("title".into(), "owned multitrack".into())],
                ..Default::default()
            };
            options
                .stream_metadata_set
                .push((video, "title".into(), "filtered video".into()));
            for &i in &audio {
                options.stream_metadata_set.extend([
                    (i, "title".into(), format!("AAC {i}")),
                    (i, "language".into(), "deu".into()),
                    (i, "ROLE".into(), "copied".into()),
                ]);
            }
            let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let capture = events.clone();
            let published = output.clone();
            options.progress = Some(fvid_control::ProgressHook::new(move |event| {
                if event.done {
                    assert!(published.exists());
                }
                capture.lock().unwrap().push(event);
            }));
            let stats = fvid_media::transcode_lossless(&source, &output, transform, &options)
                .unwrap_or_else(|e| panic!("{name}/{operation}: {e}"));
            assert_eq!(stats.backend, "fvid");
            assert_eq!(
                stats.copied_packets,
                original_packets.iter().map(|p| p.len() as u64).sum::<u64>()
            );
            assert_eq!(stats.video_packets, expected.len() as u64);
            assert_eq!(raw_frames(&output), expected);
            assert_eq!(events.lock().unwrap().iter().filter(|e| e.done).count(), 1);
            let mut decoded = fvid_media::owned_webm::WebmReader::open(
                Cursor::new(std::fs::read(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            decoded.scan_all().unwrap();
            assert_eq!(decoded.tracks.len(), input.tracks().len());
            assert_eq!(decoded.tags.title, "owned multitrack");
            assert_eq!(decoded.tracks[video].codec, "V_FFV1");
            assert_eq!(decoded.tracks[video].name, "filtered video");
            for (ordinal, &i) in audio.iter().enumerate() {
                let track = decoded.tracks[i].clone();
                let reference = control.tracks[ordinal].clone();
                assert_eq!(track.codec, "A_AAC");
                assert_eq!(track.codec_private, reference.codec_private);
                assert_eq!(track.codec_delay_ns, reference.codec_delay_ns);
                assert_eq!(
                    (track.sample_rate, track.channels),
                    (reference.sample_rate, reference.channels)
                );
                assert_eq!(track.name, format!("AAC {i}"));
                assert_eq!(track.language, "deu");
                let uid = decoded.track_uids[&track.number];
                assert_eq!(decoded.track_metadata[&uid]["ROLE"], "copied");
                let actual = packets(&mut decoded, track.number);
                assert_eq!(
                    actual.iter().map(|p| p.3.clone()).collect::<Vec<_>>(),
                    original_packets[ordinal]
                );
                assert_eq!(actual, packets(&mut control, reference.number));
            }
            std::fs::remove_file(output).unwrap();
        }
    }
}
#[test]
fn multitrack_source_limits_and_cancellation_never_publish_partial_output() {
    let source = fixture("playback-errors/shared-mp4-av.mp4");
    for (case, options) in [
        fvid_control::CopyOptions {
            max_packets: Some(1),
            ..Default::default()
        },
        {
            let cancel = fvid_control::CancelFlag::new();
            cancel.cancel();
            fvid_control::CopyOptions {
                cancel: Some(cancel),
                ..Default::default()
            }
        },
    ]
    .into_iter()
    .enumerate()
    {
        let output = std::env::temp_dir().join(format!(
            "fvid-multitrack-abort-{}-{case}.mkv",
            std::process::id()
        ));
        let error = fvid_media::transcode_lossless(&source, &output, Default::default(), &options)
            .unwrap_err();
        assert!(
            error.to_string().contains(if case == 0 {
                "packet count exceeds limit"
            } else {
                "cancelled"
            }),
            "{error}"
        );
        assert!(!output.exists());
    }
}

fn wave_payload(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let count = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"data" {
            return bytes[at + 8..at + 8 + count].to_vec();
        }
        at += 8 + count + (count & 1);
    }
    panic!("missing PCM data")
}
#[test]
fn copied_aac_has_identical_decoded_samples_after_video_filtering() {
    let source = fixture("playback-errors/shared-mp4-av.mp4");
    let base = std::env::temp_dir().join(format!("fvid-multitrack-pcm-{}", std::process::id()));
    let output = base.with_extension("mkv");
    fvid_media::transcode_lossless(
        &source,
        &output,
        fvid_media::LosslessTransform {
            negate: Some("".into()),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    let before = base.with_extension("before.wav");
    let after = base.with_extension("after.wav");
    let selected = fvid_control::CopyOptions {
        streams: vec![1],
        ..Default::default()
    };
    let a = fvid_media::decode_audio(&source, &before, &selected).unwrap();
    let b = fvid_media::decode_audio(&output, &after, &selected).unwrap();
    assert_eq!(
        (a.sample_frames, a.sample_rate, a.channels),
        (b.sample_frames, b.sample_rate, b.channels)
    );
    assert_eq!(wave_payload(&before), wave_payload(&after));
    for file in [output, before, after] {
        std::fs::remove_file(file).unwrap();
    }
}

#[test]
fn interval_filter_export_preserves_sample_exact_aac_preroll_and_padding() {
    for name in [
        "shared-mp4-av.mp4",
        "shared-mp4-av-multiple.mp4",
        "shared-mp4-av-priming.mp4",
    ] {
        let source = fixture(&format!("playback-errors/{name}"));
        for (case, interval) in [(30_001, 100_003), (0, 50_001)].into_iter().enumerate() {
            let base = std::env::temp_dir().join(format!(
                "fvid-mp4-interval-{}-{name}-{case}",
                std::process::id()
            ));
            let output = base.with_extension("mkv");
            let transform = fvid_media::LosslessTransform {
                interval: Some(interval),
                negate: Some("".into()),
                ..Default::default()
            };
            assert!(fvid_media::owned_lossless::supports(
                &source,
                &transform,
                &Default::default()
            ));
            let stats =
                fvid_media::transcode_lossless(&source, &output, transform, &Default::default())
                    .unwrap();
            assert_eq!(stats.backend, "fvid");
            let input = fvid_media::owned_mp4::Mp4Reader::open(
                Cursor::new(std::fs::read(&source).unwrap()),
                Default::default(),
            )
            .unwrap();
            for (index, track) in input
                .tracks()
                .iter()
                .enumerate()
                .filter(|(_, t)| t.handler == *b"soun")
            {
                let before = base.with_extension(format!("{index}.before.wav"));
                let after = base.with_extension(format!("{index}.after.wav"));
                let options = fvid_control::CopyOptions {
                    streams: vec![index],
                    ..Default::default()
                };
                let a =
                    fvid_media::decode_audio_interval(&source, &before, Some(interval), &options)
                        .unwrap();
                let b = fvid_media::decode_audio(&output, &after, &options).unwrap();
                let available = match track.edits.as_slice() {
                    [] => {
                        u128::from(track.duration) * u128::from(track.sample_rate)
                            / u128::from(track.timescale)
                    }
                    [edit] => (u128::from(edit.duration) * u128::from(track.sample_rate))
                        .div_ceil(u128::from(input.movie_timescale())),
                    _ => panic!("synthetic input must have at most one audio edit"),
                };
                let expected = ((interval.1 as u128 * u128::from(track.sample_rate))
                    .div_ceil(1_000_000)
                    .min(available)
                    - (interval.0 as u128 * u128::from(track.sample_rate)).div_ceil(1_000_000))
                    as u64;
                assert_eq!(a.sample_frames, expected);
                assert_eq!(b.sample_frames, expected);
                assert_eq!(
                    wave_payload(&before),
                    wave_payload(&after),
                    "{name}/{case}/{index}"
                );
                std::fs::remove_file(before).unwrap();
                std::fs::remove_file(after).unwrap();
            }
            let mut native = fvid::playback_native::NativeReader::software(
                Cursor::new(std::fs::read(&source).unwrap()),
                usize::MAX,
            )
            .unwrap();
            let raw = raw_frames(&source);
            let mut selected = Vec::new();
            let mut timeline = Vec::new();
            let mut i = 0;
            while native.read_frame_raw().unwrap().is_some() {
                let (from, to, scale) = native.frame_interval().unwrap();
                let from = (from * 1_000_000_000 / u128::from(scale)) as i64;
                let to = (to * 1_000_000_000 / u128::from(scale)) as i64;
                let start = from;
                let end = to;
                if from >= interval.0 * 1000 && from < interval.1 * 1000 {
                    selected.push(raw[i].iter().map(|p| 255 - p).collect::<Vec<_>>());
                    timeline.push((start - interval.0 * 1000, Some((end - start) as u64)));
                }
                i += 1;
            }
            assert!(
                raw_frames(&output) == selected,
                "video pixels differ: {name}/{case}"
            );
            let mut mux = fvid_media::owned_webm::WebmReader::open(
                Cursor::new(std::fs::read(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            mux.scan_all().unwrap();
            let video = mux.tracks.iter().find(|t| t.kind == 1).unwrap().number;
            assert_eq!(
                mux.packets
                    .iter()
                    .filter(|p| p.track == video)
                    .map(|p| (p.pts_ns, p.duration_ns))
                    .collect::<Vec<_>>(),
                timeline
            );
            std::fs::remove_file(output).unwrap();
        }
    }
}
