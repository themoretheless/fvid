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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp1-native")
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    m["cases"].as_array().unwrap().clone()
}
fn read(stream: &mut dyn AudioStream, decoder: &mut dyn AudioDecode, start: u64) -> Vec<u8> {
    let rate = u64::from(stream.sample_rate());
    let width = usize::from(stream.channels()) * 4;
    let mut position = start;
    let mut pcm = Vec::new();
    while let Some(p) = stream.next_packet().unwrap() {
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        if let Some(p) = stream
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            assert_eq!(
                p.presentation_time(),
                Duration::new(
                    position / rate,
                    ((position % rate) * 1_000_000_000 / rate) as u32
                ),
                "exact Layer I presentation clock"
            );
            position += (p.data.len() / width) as u64;
            pcm.extend(p.data);
        }
    }
    pcm
}
fn same(a: &[u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    for (i, (a, b)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() < 2e-6, "sample {i}: {a} versus {b}");
    }
}
#[test]
fn layer1_factory_raw_matroska_video_pcm_seek_and_rewind_are_owned() {
    for case in cases() {
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let channels = case["channels"].as_u64().unwrap();
        let golden = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
        let raw = std::fs::read(root().join(format!("{name}.mp1"))).unwrap();
        let video = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
        let mut image =
            fvid::playback_native::NativeReader::software(Cursor::new(&video), 32 << 20).unwrap();
        for _ in 0..7 {
            assert!(image.read_frame_raw().unwrap().is_some(), "{name}");
        }
        assert!(image.read_frame_raw().unwrap().is_none());
        for container in ["raw", "matroska"] {
            let mut stream: Box<dyn AudioStream> = if container == "raw" {
                Box::new(
                    fvid::playback_mp3::Mp3AudioReader::open(raw.as_slice(), Default::default())
                        .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(video.clone()),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            assert_eq!(stream.codec(), "A_MPEG/L1");
            let mut decoder = stream.make_decoder().unwrap();
            same(&read(stream.as_mut(), decoder.as_mut(), 0), &golden);
            assert_eq!(
                stream.duration(),
                Some(Duration::from_nanos(2688 * 1_000_000_000 / rate)),
                "{name}/{container} duration"
            );
            stream.rewind();
            decoder.reset();
            same(&read(stream.as_mut(), decoder.as_mut(), 0), &golden);
            let target = if container == "raw" {
                777
            } else {
                (777 * 1_000_000_000 / rate) as i64
            };
            assert_eq!(stream.seek_to(target), target);
            decoder.reset();
            same(
                &read(stream.as_mut(), decoder.as_mut(), 777),
                &golden[777 * channels as usize * 4..],
            );
        }
    }
}
#[test]
fn layer1_indexed_pcm_matches_reference_and_reference_free_refusal_is_explicit() {
    for case in cases() {
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap() as u32;
        let channels = case["channels"].as_u64().unwrap() as u16;
        let raw = std::fs::read(root().join(format!("{name}.mp1"))).unwrap();
        let mut stream =
            fvid::playback_mp3::Mp3AudioReader::open(raw.as_slice(), Default::default()).unwrap();
        let mut owned = stream.make_decoder().unwrap();
        let mut reference = fvid::codec::symphonia_decoder::SymphoniaDecoder::new(
            "Layer I reference",
            symphonia::core::codecs::audio::well_known::CODEC_ID_MP1,
            &[],
            rate,
            channels,
        )
        .unwrap();
        while let Some(packet) = stream.next_packet().unwrap() {
            let a = owned
                .decode_packet(&packet.data, packet.pts, 384)
                .unwrap()
                .unwrap()
                .packet;
            if case["free"].as_bool() == Some(true) {
                let error = reference
                    .decode_packet(&packet.data, packet.pts, 384)
                    .err()
                    .unwrap()
                    .to_string();
                assert!(error.contains("free bit-rate is not supported"), "{error}");
                break;
            }
            let b = reference
                .decode_packet(&packet.data, packet.pts, 384)
                .unwrap()
                .unwrap()
                .packet;
            assert_eq!(a.data.len(), b.data.len());
            for (a, b) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
                let a = f32::from_le_bytes(a.try_into().unwrap());
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!((a - b).abs() < 5e-5, "{name}: {a} versus {b}");
            }
        }
    }
}
#[test]
fn layer1_bad_audio_video_companions_reproduce_specific_owned_refusals() {
    for (name, reason) in [
        ("bad-crc", "CRC mismatch"),
        ("reserved-allocation", "reserved MP1 bit allocation"),
        ("truncated", "truncated MP1 payload"),
        ("free-spacing", "length changes beyond padding"),
    ] {
        let video = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
        let mut image =
            fvid::playback_native::NativeReader::software(Cursor::new(video.clone()), 32 << 20)
                .unwrap();
        let mut frames = 0;
        while image.read_frame_raw().unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, if name == "truncated" { 1 } else { 7 });
        let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(video),
            Default::default(),
        )
        .unwrap();
        let mut decoder = stream.make_decoder().unwrap();
        let mut error = None;
        while let Some(p) = stream.next_packet().unwrap() {
            match decoder.decode_packet(&p.data, p.pts, p.duration as u64) {
                Ok(_) => {}
                Err(e) => {
                    error = Some(e.to_string());
                    break;
                }
            }
        }
        assert!(
            error.as_ref().is_some_and(|e| e.contains(reason)),
            "{name}: {error:?}"
        );
    }
}

#[test]
fn layer1_captured_workers_preserve_every_rate_and_partial_seek_packet() {
    let mut checked = 0;
    for case in cases() {
        if case["dense"].as_bool() == Some(true) {
            continue;
        }
        let mode = case["mode"].as_u64().unwrap();
        let extension = case["extension"].as_u64().unwrap();
        let protected = case["protected"].as_bool().unwrap();
        if !((mode == 3 && protected) || (mode == 1 && extension == 2 && !protected)) {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let channels = case["channels"].as_u64().unwrap() as usize;
        for container in ["raw", "matroska"] {
            let raw = std::fs::read(root().join(format!("{name}.mp1"))).unwrap();
            let video = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
            let make = || -> Box<dyn AudioStream> {
                if container == "raw" {
                    Box::new(
                        fvid::playback_mp3::Mp3AudioReader::open(
                            raw.as_slice(),
                            Default::default(),
                        )
                        .unwrap(),
                    )
                } else {
                    Box::new(
                        fvid::playback_webm_audio::WebmAudioReader::open(
                            Cursor::new(video.clone()),
                            Default::default(),
                        )
                        .unwrap(),
                    )
                }
            };
            let mut direct = make();
            let mut decoder = direct.make_decoder().unwrap();
            let mut full = Vec::new();
            while let Some(packet) = direct.next_packet().unwrap() {
                let frame = decoder
                    .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                    .unwrap()
                    .unwrap();
                if let Some(pcm) = direct
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    full.push((pcm.presentation_time(), pcm.data));
                }
            }
            assert_eq!(full.len(), 7);
            let target = rate * 40 / 1000;
            let suffix: Vec<_> = full
                .iter()
                .enumerate()
                .filter_map(|(i, (_, data))| {
                    let begin = i as u64 * 384;
                    let end = begin + 384;
                    if end <= target {
                        return None;
                    }
                    let first = begin.max(target);
                    Some((
                        Duration::from_nanos(first * 1_000_000_000 / rate),
                        data[(first - begin) as usize * channels * 4..].to_vec(),
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
        checked += 1;
    }
    assert_eq!(checked, 36);
}

#[test]
fn layer1_single_eof_free_frame_and_small_spacing_do_not_hide_damaged_packets() {
    for (name, frames) in [("single-free", 1), ("free-small", 7)] {
        let raw = std::fs::read(root().join(format!("{name}.mp1"))).unwrap();
        let expected = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
        let mut stream =
            fvid::playback_mp3::Mp3AudioReader::open(raw.as_slice(), Default::default()).unwrap();
        assert_eq!(stream.mp3().packets(), frames);
        let mut decoder = stream.make_decoder().unwrap();
        same(&read(&mut stream, decoder.as_mut(), 0), &expected);
        let video = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
        let mut image =
            fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
        let mut count = 0;
        while image.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, frames);
    }
    let bad = std::fs::read(root().join("bad-second-crc-free.mp1")).unwrap();
    let error = fvid::playback_mp3::Mp3AudioReader::open(bad.as_slice(), Default::default())
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("cannot infer free-format"), "{error}");
    let video = std::fs::read(root().join("bad-second-crc-free.mkv")).unwrap();
    let mut audio =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(video), Default::default())
            .unwrap();
    let mut decoder = audio.make_decoder().unwrap();
    let first = audio.next_packet().unwrap().unwrap();
    decoder.decode_packet(&first.data, first.pts, 384).unwrap();
    let second = audio.next_packet().unwrap().unwrap();
    let error = decoder
        .decode_packet(&second.data, second.pts, 384)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("CRC mismatch"), "{error}");
}

#[test]
fn layer1_quantized_codec_delay_and_tail_padding_preserve_exact_sample_clocks() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["clock_cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let base = case["base"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let first = case["start"].as_u64().unwrap() as usize;
        let last = case["end"].as_u64().unwrap() as usize;
        let golden = std::fs::read(root().join(format!("{base}.f32le"))).unwrap();
        let expected = &golden[first * 8..last * 8];
        let movie = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
        let mut image =
            fvid::playback_native::NativeReader::software(Cursor::new(movie.clone()), 32 << 20)
                .unwrap();
        for _ in 0..7 {
            assert!(image.read_frame_raw().unwrap().is_some());
        }
        assert!(image.read_frame_raw().unwrap().is_none());
        let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(movie),
            Default::default(),
        )
        .unwrap();
        let mut decoder = stream.make_decoder().unwrap();
        same(&read(&mut stream, decoder.as_mut(), 0), expected);
        let duration = Duration::from_nanos((last - first) as u64 * 1_000_000_000 / rate);
        assert_eq!(stream.duration(), Some(duration), "{name}");
        stream.rewind();
        decoder.reset();
        same(&read(&mut stream, decoder.as_mut(), 0), expected);
        assert_eq!(stream.duration(), Some(duration));
        let target = (777 * 1_000_000_000 / rate) as i64;
        assert_eq!(stream.seek_to(target), target);
        decoder.reset();
        same(
            &read(&mut stream, decoder.as_mut(), 777),
            &expected[777 * 8..],
        );
        assert_eq!(stream.duration(), Some(duration));
        if name.ends_with("delay-tail-ceil") {
            let movie = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
            let make = || -> Box<dyn AudioStream> {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(movie.clone()),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            let mut direct = make();
            let mut decoder = direct.make_decoder().unwrap();
            let mut full = Vec::new();
            while let Some(p) = direct.next_packet().unwrap() {
                let frame = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap();
                if let Some(pcm) = direct
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    full.push((pcm.presentation_time(), pcm.data));
                }
            }
            let target = rate * 40 / 1000;
            let mut position = 0u64;
            let suffix: Vec<_> = full
                .iter()
                .filter_map(|(_, data)| {
                    let begin = position;
                    let end = begin + data.len() as u64 / 8;
                    position = end;
                    if end <= target {
                        return None;
                    }
                    let first = begin.max(target);
                    Some((
                        Duration::from_nanos(first * 1_000_000_000 / rate),
                        data[(first - begin) as usize * 8..].to_vec(),
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
