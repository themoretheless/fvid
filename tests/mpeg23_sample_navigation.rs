#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod audio_capture;
use fvid::audio::{AudioDecode, AudioStream};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(root().join("mpeg-clock-metadata/manifest.json")).unwrap(),
    )
    .unwrap()
}
fn time(sample: u64, rate: u64) -> Duration {
    Duration::new(
        sample / rate,
        ((sample % rate) * 1_000_000_000 / rate) as u32,
    )
}
fn same(a: &[u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    for (i, (a, b)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() < 2e-6, "sample {i}: {a} versus {b}");
    }
}
fn read(
    stream: &mut dyn AudioStream,
    decoder: &mut dyn AudioDecode,
    start: u64,
) -> Vec<(Duration, Vec<u8>)> {
    let rate = u64::from(stream.sample_rate());
    let width = usize::from(stream.channels()) * 4;
    let mut position = start;
    let mut out = Vec::new();
    while let Some(p) = stream.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        if let Some(p) = stream.present_decoded(f.packet, f.source_pts).unwrap() {
            assert_eq!(
                p.presentation_time(),
                time(position, rate),
                "exact MPEG sample projection"
            );
            position += p.data.len() as u64 / width as u64;
            out.push((p.presentation_time(), p.data));
        }
    }
    out
}
fn flat(p: &[(Duration, Vec<u8>)]) -> Vec<u8> {
    p.iter()
        .flat_map(|(_, data)| data.iter().copied())
        .collect()
}
#[test]
fn raw_layer2_and_layer3_seeks_keep_synthesis_history_and_exact_samples() {
    for base in manifest()["bases"].as_array().unwrap() {
        let name = base["name"].as_str().unwrap();
        let rate = base["rate"].as_u64().unwrap();
        let width = base["channels"].as_u64().unwrap() as usize * 4;
        let samples = base["samples"].as_u64().unwrap();
        let source = std::fs::read(
            root()
                .join("mpeg-clock-metadata")
                .join(base["raw"].as_str().unwrap()),
        )
        .unwrap();
        let expected = std::fs::read(root().join(base["source_pcm"].as_str().unwrap())).unwrap();
        let mut stream =
            fvid::playback_mp3::Mp3AudioReader::open(source.as_slice(), Default::default())
                .unwrap();
        let mut decoder = stream.make_decoder().unwrap();
        same(&flat(&read(&mut stream, decoder.as_mut(), 0)), &expected);
        for target in [0, 777, 1929, samples - 1, samples, samples + 777] {
            let wanted = target.min(samples);
            assert_eq!(
                stream.seek_to(target as i64),
                wanted as i64,
                "{name} exact seek target"
            );
            decoder.reset();
            same(
                &flat(&read(&mut stream, decoder.as_mut(), wanted)),
                &expected[wanted as usize * width..],
            );
        }
        assert_eq!(stream.duration(), Some(time(samples, rate)));
        stream.rewind();
        decoder.reset();
        same(&flat(&read(&mut stream, decoder.as_mut(), 0)), &expected);
    }
}
#[test]
fn matroska_layer2_and_layer3_delay_padding_duration_and_seek_use_exact_sample_clocks() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let width = case["channels"].as_u64().unwrap() as usize * 4;
        let start = case["start"].as_u64().unwrap() as usize;
        let end = case["end"].as_u64().unwrap() as usize;
        let golden = std::fs::read(root().join(case["source_pcm"].as_str().unwrap())).unwrap();
        let expected = &golden[start * width..end * width];
        let source = std::fs::read(
            root()
                .join("mpeg-clock-metadata")
                .join(format!("{name}.mkv")),
        )
        .unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(source.clone()), 32 << 20)
                .unwrap();
        for _ in 0..case["packet_count"].as_u64().unwrap() {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(source),
            Default::default(),
        )
        .unwrap();
        let mut decoder = stream.make_decoder().unwrap();
        same(&flat(&read(&mut stream, decoder.as_mut(), 0)), expected);
        let duration = time((end - start) as u64, rate);
        assert_eq!(stream.duration(), Some(duration), "{name} duration");
        stream.rewind();
        decoder.reset();
        same(&flat(&read(&mut stream, decoder.as_mut(), 0)), expected);
        for sample in [777, (end - start) as u64 - 1] {
            let target = time(sample, rate).as_nanos() as i64;
            assert_eq!(stream.seek_to(target), target);
            decoder.reset();
            same(
                &flat(&read(&mut stream, decoder.as_mut(), sample)),
                &expected[sample as usize * width..],
            );
            assert_eq!(stream.duration(), Some(duration));
        }
    }
}

#[test]
fn cold_layer2_packet_decode_reproduces_the_history_loss_that_full_seek_replay_prevents() {
    let source = std::fs::read(root().join("mpeg-clock-metadata/mp2-48000.mp2")).unwrap();
    let stream =
        fvid::playback_mp3::Mp3AudioReader::open(source.as_slice(), Default::default()).unwrap();
    let expected = std::fs::read(root().join("mp2-native/free-0-0-1-48000.f32le")).unwrap();
    let mut cold = stream.make_decoder().unwrap();
    let packet = stream.mp3().packet(1);
    let pcm = cold.decode_encoded(packet, 1152, 1152).unwrap().unwrap();
    let wanted = &expected[1152 * 8..2304 * 8];
    let peak = pcm
        .data
        .chunks_exact(4)
        .zip(wanted.chunks_exact(4))
        .map(|(a, b)| {
            (f32::from_le_bytes(a.try_into().unwrap()) - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
        })
        .fold(0f32, f32::max);
    assert!(
        peak > 1e-5,
        "cold synthesis must reproduce a real PCM error: {peak}"
    );
}
#[test]
fn captured_mpeg_workers_preserve_partial_seek_packets_and_rounded_delay_at_all_rates() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if !name.ends_with("delay-tail-ceil") {
            continue;
        }
        let rate = case["rate"].as_u64().unwrap();
        let width = case["channels"].as_u64().unwrap() as usize * 4;
        for kind in ["raw", "matroska"] {
            let source = std::fs::read(root().join("mpeg-clock-metadata").join(if kind == "raw" {
                case["raw"].as_str().unwrap().to_owned()
            } else {
                format!("{name}.mkv")
            }))
            .unwrap();
            let make = || -> Box<dyn AudioStream> {
                if kind == "raw" {
                    Box::new(
                        fvid::playback_mp3::Mp3AudioReader::open(
                            source.as_slice(),
                            Default::default(),
                        )
                        .unwrap(),
                    )
                } else {
                    Box::new(
                        fvid::playback_webm_audio::WebmAudioReader::open(
                            Cursor::new(source.clone()),
                            Default::default(),
                        )
                        .unwrap(),
                    )
                }
            };
            let mut direct = make();
            let mut decoder = direct.make_decoder().unwrap();
            let full = read(direct.as_mut(), decoder.as_mut(), 0);
            let target = rate * 40 / 1000;
            let mut position = 0;
            let suffix: Vec<_> = full
                .iter()
                .filter_map(|(_, data)| {
                    let start = position;
                    let end = start + data.len() as u64 / width as u64;
                    position = end;
                    if end <= target {
                        return None;
                    }
                    let first = start.max(target);
                    Some((
                        time(first, rate),
                        data[(first - start) as usize * width..].to_vec(),
                    ))
                })
                .collect();
            audio_capture::worker_check_with_seek_expected(
                make(),
                &full,
                Duration::from_millis(40),
                &suffix,
            );
        }
    }
}

#[test]
fn matroska_mpeg_preserves_a_real_one_tick_gap() {
    for layer in [2, 3] {
        let bytes =
            std::fs::read(root().join(format!("mpeg-clock-metadata/mp{layer}-48000-tick-gap.mkv")))
                .unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        let mut count = 0;
        while video.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert!(count >= 2);
        let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        let mut decoder = stream.make_decoder().unwrap();
        let mut index = 0;
        while let Some(p) = stream.next_packet().unwrap() {
            let expected = index * 24_000_000 + if index > 0 { 1_000_000 } else { 0 };
            assert_eq!(
                p.pts, expected,
                "Layer {layer} packet {index}: real gap swallowed"
            );
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            let out = stream
                .present_decoded(f.packet, f.source_pts)
                .unwrap()
                .unwrap();
            assert_eq!(out.pts, expected as u64);
            index += 1;
        }
        assert_eq!(index, count);
    }
}

#[test]
fn scaled_mpeg_matroska_recovers_exact_packet_and_seek_clock() {
    let info = manifest();
    for layer in [2, 3] {
        let base = info["bases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["layer"] == layer && c["rate"] == 44100)
            .unwrap();
        let golden = std::fs::read(root().join(base["source_pcm"].as_str().unwrap())).unwrap();
        let stride = base["channels"].as_u64().unwrap() as usize * 4;
        for scale in ["double", "third", "shift-third"] {
            let bytes = std::fs::read(root().join(format!(
                "mpeg-clock-metadata/mp{layer}-44100-scaled-{scale}.mkv"
            )))
            .unwrap();
            let mut video =
                fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                    .unwrap();
            let mut count = 0;
            while video.read_frame_raw().unwrap().is_some() {
                count += 1;
            }
            for target in [0, 30_000_000, 30_000_001] {
                let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(bytes.clone()),
                    Default::default(),
                )
                .unwrap();
                assert_eq!(stream.seek_to(target), target);
                let mut decoder = stream.make_decoder().unwrap();
                let mut actual = Vec::new();
                let mut index = 0;
                while let Some(p) = stream.next_packet().unwrap() {
                    let f = decoder
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(out) = stream.present_decoded(f.packet, f.source_pts).unwrap() {
                        actual.extend_from_slice(&out.data);
                        let start = (index * 1152)
                            .max((target as u128 * 44100).div_ceil(1_000_000_000) as usize);
                        assert_eq!(
                            out.pts,
                            (start as u64 * 1_000_000_000) / 44100,
                            "L{layer}/{scale}/{target}/{index}"
                        );
                    }
                    index += 1;
                }
                let first = (target as u128 * 44100).div_ceil(1_000_000_000) as usize;
                same(&actual, &golden[first * stride..]);
                assert_eq!(index, count);
                assert_eq!(stream.duration(), Some(time(count as u64 * 1152, 44100)));
            }
        }
    }
}
