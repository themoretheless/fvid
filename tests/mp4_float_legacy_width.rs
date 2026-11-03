#![cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-float-legacy-width")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn open(n: &str) -> fvid::playback_mp4_audio::Mp4AudioReader<Cursor<Vec<u8>>> {
    fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default())
        .unwrap()
}
#[test]
fn native_width_pcm_and_video_controls_are_independent_and_valid() {
    for c in cases() {
        let mut reader = open(c["control"].as_str().unwrap());
        let p = rational::collect(&mut reader);
        assert_eq!(
            p.iter()
                .flat_map(|p| p.data.iter().copied())
                .collect::<Vec<_>>(),
            bytes(c["expected"].as_str().unwrap())
        );
    }
    let name = cases()[0]["file"].as_str().unwrap().to_owned();
    let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes(&name)),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let f = reader.read_frame().unwrap().unwrap();
        let p = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(p, bytes(&format!("frame-{i}.yuv16")));
    }
    assert!(reader.read_frame().unwrap().is_none());
}
#[test]
fn legacy_sample_size_preserves_native_clock_pcm_seek_and_eof() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut reader = open(name);
        assert_eq!(
            reader.bits_per_sample(),
            16,
            "preserve raw declared metadata"
        );
        let mut control = open(c["control"].as_str().unwrap());
        let full = rational::collect(&mut control);
        assert_eq!(
            reader.duration(),
            Some(rational::duration(
                c["frames"].as_u64().unwrap() as u128,
                48000
            ))
        );
        rational::exact(&rational::collect(&mut reader), &full, name);
        for at in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_micros(125),
            Duration::from_nanos(19999999),
            Duration::from_millis(20),
            Duration::from_millis(60),
            Duration::MAX,
        ] {
            reader.seek_time(at);
            rational::exact(
                &rational::collect(&mut reader),
                &rational::suffix(&full, at, (c["frames"].as_u64().unwrap() as u128, 48000)),
                name,
            );
        }
        reader.rewind();
        rational::exact(&rational::collect(&mut reader), &full, name);
        reader.validate_eof().unwrap();
    }
}
#[test]
fn owned_float_exports_preserve_exact_scalar_data_and_interval_endpoints() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let pcm = bytes(c["expected"].as_str().unwrap());
        let precise = bytes(c["expected64"].as_str().unwrap());
        let channels = c["channels"].as_u64().unwrap() as usize;
        for cut in [false, true] {
            let interval = cut.then(|| {
                (
                    Duration::from_millis(20),
                    rational::duration(c["frames"].as_u64().unwrap() as u128, 48000),
                )
            });
            let first = if cut { 960 * channels } else { 0 };
            let mut out = Vec::new();
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(name)),
                &mut out,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(out, pcm[first * 4..], "{name} f32");
            assert_eq!(
                stats.sample_frames,
                if cut {
                    c["frames"].as_u64().unwrap() - 960
                } else {
                    c["frames"].as_u64().unwrap()
                }
            );
            out.clear();
            fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
                Cursor::new(bytes(name)),
                &mut out,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(out, precise[first * 8..], "{name} f64");
        }
    }
}
#[test]
fn actual_workers_use_float_width_for_legacy_v1_navigation() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut control = open(c["control"].as_str().unwrap());
        let full = rational::collect(&mut control);
        for at in [Duration::ZERO, Duration::from_millis(20), Duration::MAX] {
            let wanted =
                rational::suffix(&full, at, (c["frames"].as_u64().unwrap() as u128, 48000));
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker =
                fvid::audio_thread::AudioPlayback::start(Box::new(open(name)), move || {
                    Box::new(rational::RawCapture {
                        inner: transitions::Capture::new(trace),
                        raw: copy,
                    })
                });
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            let got = raw.lock().unwrap();
            rational::exact(&got.packets, &wanted, name);
            assert_eq!(
                got.anchors,
                vec![rational::anchor(
                    &wanted,
                    (c["frames"].as_u64().unwrap() as u128, 48000)
                )]
            );
        }
    }
}

#[test]
fn coded_frame_count_agrees_with_float_storage_not_legacy_metadata() {
    for c in cases() {
        let codec = if c["bits"] == 32 { *b"fl32" } else { *b"fl64" };
        let channels = c["channels"].as_u64().unwrap() as u16;
        let stride = usize::from(channels) * c["bits"].as_u64().unwrap() as usize / 8;
        assert_eq!(
            fvid_media::owned_mp4_audio_packet_frames::frames(
                &codec,
                &[],
                48000,
                channels,
                16,
                960 * stride,
                None
            )
            .unwrap(),
            960
        );
        let error = fvid_media::owned_mp4_audio_packet_frames::frames(
            &codec,
            &[],
            48000,
            channels,
            16,
            stride / 2,
            None,
        )
        .unwrap_err();
        assert_eq!(error, "incomplete MP4 coded audio block");
    }
}

#[test]
fn incomplete_physical_float_frames_refuse_instead_of_becoming_legacy_words() {
    let manifest: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    for c in manifest["refusals"].as_array().unwrap() {
        for key in ["file", "control"] {
            let name = c[key].as_str().unwrap();
            let error = consume(name)
                .expect_err("past-source edit is an explicit refusal, not successful playback");
            assert!(
                error.to_string().contains("extends beyond source packets"),
                "{name}: {error}"
            );
            let mut out = Vec::new();
            let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(name)),
                &mut out,
                None,
                &Default::default(),
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("extends outside available samples"),
                "{name}: {error}"
            );
        }
    }
    for name in manifest["bad"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        let result = consume(name);
        let error = result.expect_err("partial physical float frame must refuse");
        assert!(error.to_string().contains("incomplete"), "{name}: {error}");
        let mut out = Vec::new();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(name)),
            &mut out,
            None,
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("incomplete"), "{name}: {error}");
    }
}

fn consume(name: &str) -> fvid::Result<()> {
    use fvid::audio::{AudioStep, DecodedAudio};
    fn present(reader: &dyn AudioStream, mut frame: DecodedAudio) -> fvid::Result<()> {
        if let Some(n) =
            reader.decoded_packet_sample_limit(frame.source_pts, frame.source_duration)?
        {
            let bytes = n.checked_mul(usize::from(reader.channels()) * 4).unwrap();
            assert!(bytes <= frame.packet.data.len());
            frame.packet.data.truncate(bytes);
        }
        reader.present_decoded(frame.packet, frame.source_pts)?;
        Ok(())
    }
    let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
        Cursor::new(bytes(name)),
        Default::default(),
    )?;
    let mut decoder = reader.make_decoder()?;
    loop {
        match reader.next_step()? {
            Some(AudioStep::Encoded(p)) => {
                if let Some(f) = decoder
                    .decode_packet(&p.data, reader.decoder_stamp(&p), p.duration as u64)
                    .map_err(|e| fvid::Error::Invalid(e.to_string()))?
                {
                    present(&reader, f)?;
                }
            }
            Some(AudioStep::Pcm(_)) => (),
            Some(AudioStep::ResetDecoder) => reader.reset_decoder(&mut decoder)?,
            Some(AudioStep::ReconfigureDecoder) => {
                while let Some(f) = decoder
                    .finish_packet()
                    .map_err(|e| fvid::Error::Invalid(e.to_string()))?
                {
                    present(&reader, f)?;
                }
                reader.advance_configuration()?;
                decoder = reader.make_decoder()?;
            }
            None => {
                if !reader.drain_decoded_at_eof() {
                    decoder.reset();
                    break;
                }
                if let Some(f) = decoder
                    .finish_packet()
                    .map_err(|e| fvid::Error::Invalid(e.to_string()))?
                {
                    present(&reader, f)?;
                    continue;
                }
                break;
            }
        }
    }
    reader.validate_eof()
}
