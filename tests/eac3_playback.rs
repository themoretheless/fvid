#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;

use fvid::audio::{AudioDecode, AudioStream};
use fvid::codec::eac3_decoder::{Eac3Decoder, frame as eac3_frame};
use fvid::codec::make_audio_decoder;
use std::io::Cursor;
use std::path::Path;

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/eac3-playback")
}

fn f32_samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|w| f32::from_le_bytes(w.try_into().unwrap()))
        .collect()
}

fn peak_channels(samples: &[f32], channels: usize) -> Vec<f32> {
    let mut peaks = vec![0.0f32; channels];
    for frame in samples.chunks_exact(channels) {
        for (ch, &val) in frame.iter().enumerate() {
            peaks[ch] = peaks[ch].max(val.abs());
        }
    }
    peaks
}

#[test]
fn make_audio_decoder_dispatches_eac3_tags_and_validates_frame_rate() {
    let dir = fixture_dir();
    let bytes = std::fs::read(dir.join("blocks-6.ec3")).unwrap();
    for tag in ["A_EAC3", "ec-3"] {
        let mut decoder = make_audio_decoder(tag, &[], 48_000, 2, 0)
            .unwrap_or_else(|e| panic!("make_audio_decoder({tag}) failed: {e}"));
        let pkt = decoder
            .decode_complete(&bytes, 0, 0)
            .expect("decodes 6-block E-AC-3")
            .expect("audio packet");
        let samples = f32_samples(&pkt.data);
        assert_eq!(samples.len(), (2 * 1536 - 256) * 2);
        let peaks = peak_channels(&samples, 2);
        assert!(peaks[0] > 0.2 && peaks[1] > 0.15, "peaks={peaks:?}");
    }

    // Clean syncframe rate mismatch against configured decoder rate must be refused.
    let mut bad_rate = make_audio_decoder("A_EAC3", &[], 32_000, 2, 0).unwrap();
    let err = bad_rate
        .decode_complete(&bytes, 0, 0)
        .err()
        .expect("48 kHz E-AC-3 frame on 32 kHz decoder must fail");
    assert!(
        err.to_string().contains("frame sample rate") && err.to_string().contains("disagrees"),
        "unexpected error: {err}"
    );
}

#[test]
fn eac3_conversion_layer_1_2_3_and_6_block_syncframes_decode_matching_reference() {
    let dir = fixture_dir();
    for (blocks, expected_syncframes) in [(6, 2), (3, 4), (2, 6), (1, 12)] {
        let raw = std::fs::read(dir.join(format!("blocks-{blocks}.ec3"))).unwrap();
        let ref_bytes = std::fs::read(dir.join(format!("blocks-{blocks}.f32le"))).unwrap();
        let ref_samples = f32_samples(&ref_bytes);

        // Verify syncframe walker sees the exact block count on every frame.
        let mut rest = raw.as_slice();
        let mut count = 0usize;
        while !rest.is_empty() {
            let hdr = eac3_frame(rest)
                .unwrap_or_else(|| panic!("blocks-{blocks}: header failed at frame {count}"));
            assert_eq!(hdr.blocks, blocks);
            assert_eq!(hdr.sample_rate, 48_000);
            assert_eq!(hdr.channels, 2);
            rest = &rest[hdr.frame_bytes..];
            count += 1;
        }
        assert_eq!(count, expected_syncframes);

        // Verify full decode matches reference amplitude and length (after 256-sample encoder delay).
        let mut dec = Eac3Decoder::new(&[], 48_000, 2).unwrap();
        let pkt = dec.decode_complete(&raw, 0, 0).unwrap().unwrap();
        let got = f32_samples(&pkt.data);
        let ref_trimmed = &ref_samples[256 * 2..];
        assert_eq!(got.len(), ref_trimmed.len(), "blocks-{blocks} sample count");
        let got_peaks = peak_channels(&got, 2);
        let ref_peaks = peak_channels(ref_trimmed, 2);
        for ch in 0..2 {
            assert!(
                (got_peaks[ch] - ref_peaks[ch]).abs() < 0.02,
                "blocks-{blocks} ch {ch}: got peak {} vs ref {}",
                got_peaks[ch],
                ref_peaks[ch]
            );
        }

        // Verify bare .ec3 stream opens through Ac3AudioReader too.
        let mut reader = fvid::playback_ac3::Ac3AudioReader::open(
            Cursor::new(raw),
            fvid::playback_ac3::Limits::default(),
        )
        .unwrap_or_else(|e| panic!("Ac3AudioReader failed on blocks-{blocks}.ec3: {e}"));
        assert_eq!(reader.codec(), "A_EAC3");
        assert_eq!((reader.sample_rate(), reader.channels()), (48_000, 2));
        let mut stream_dec = reader.make_decoder().unwrap();
        let mut total_frames = 0usize;
        while let Some(enc) = reader.next_packet().unwrap() {
            assert_eq!(enc.duration, (blocks * 256) as i64);
            if let Some(decoded) = stream_dec.decode_packet(&enc.data,enc.pts,enc.duration as u64).unwrap() {
                if let Some(presented)=reader.present_decoded(decoded.packet,decoded.source_pts).unwrap() {
                    total_frames+=presented.data.len()/8;
                }
            }
        }
        total_frames+=stream_dec.drain_presented(&mut reader).iter().map(|p|p.data.len()/8).sum::<usize>();
        assert_eq!(total_frames, expected_syncframes * blocks * 256 - 256);
    }
}

#[test]
fn eac3_reduced_sample_rates_24k_22k05_and_16k_decode_cleanly() {
    let dir = fixture_dir();
    for rate in [24_000u32, 22_050, 16_000] {
        let raw = std::fs::read(dir.join(format!("reduced-{rate}.ec3"))).unwrap();
        let hdr = eac3_frame(&raw)
            .unwrap_or_else(|| panic!("reduced-{rate}.ec3 header was refused"));
        assert_eq!(hdr.sample_rate, rate);
        assert_eq!(hdr.blocks, 6);
        assert_eq!(hdr.channels, 2);

        let mut dec = make_audio_decoder("A_EAC3", &[], rate, 2, 0)
            .unwrap_or_else(|e| panic!("make_audio_decoder at {rate} Hz failed: {e}"));
        let pkt = dec
            .decode_complete(&raw, 0, 0)
            .unwrap_or_else(|e| panic!("decode at {rate} Hz failed: {e}"))
            .unwrap();
        assert_eq!((pkt.timebase_num, pkt.timebase_den), (1, rate));
        let samples = f32_samples(&pkt.data);
        assert_eq!(samples.len(), (2 * 1536 - 256) * 2);
        let peaks = peak_channels(&samples, 2);
        assert!(
            peaks[0] > 0.2 && peaks[1] > 0.15,
            "reduced-{rate} peaks={peaks:?}"
        );
    }
}

#[test]
fn mp4_and_matroska_containers_decode_eac3_stereo_51_and_conversion_layer_tracks() {
    let dir = fixture_dir();

    // 1. Stereo 48 kHz in MP4 (ec-3) and Matroska (A_EAC3)
    for (file, want_codec) in [("stereo-48k.mp4", "ec-3"), ("stereo-48k.mkv", "A_EAC3")] {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        let mut stream: Box<dyn AudioStream> = if file.ends_with(".mp4") {
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap_or_else(|e| panic!("{file} open failed: {e}")),
            )
        } else {
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap_or_else(|e| panic!("{file} open failed: {e}")),
            )
        };
        assert_eq!(stream.codec(), want_codec);
        assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 2));
        let mut dec = stream.make_decoder().unwrap();
        let mut total_frames = 0usize;
        let mut all_samples = Vec::new();
        while let Some(step) = stream.next_step().unwrap() {
            match step {
                fvid::audio::AudioStep::Encoded(p) => {
                    let d = dec
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(pres) = stream.present_decoded(d.packet, d.source_pts).unwrap() {
                        total_frames += pres.data.len() / 8;
                        all_samples.extend(f32_samples(&pres.data));
                    }
                }
                fvid::audio::AudioStep::Pcm(pres) => {
                    total_frames += pres.data.len() / 8;
                    all_samples.extend(f32_samples(&pres.data));
                }
                fvid::audio::AudioStep::ResetDecoder => stream.reset_decoder(&mut dec).unwrap(),
            }
        }
        assert!(total_frames >= 2816, "{file}: total_frames={total_frames}");
        let peaks = peak_channels(&all_samples, 2);
        assert!(
            peaks[0] > 0.2 && peaks[1] > 0.15,
            "{file} peaks={peaks:?}"
        );
    }

    // 2. 5.1 surround in MP4 and Matroska
    for (file, want_codec) in [("surround-51.mp4", "ec-3"), ("surround-51.mkv", "A_EAC3")] {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        let mut stream: Box<dyn AudioStream> = if file.ends_with(".mp4") {
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            )
        } else {
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        assert_eq!(stream.codec(), want_codec);
        assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 6));
        let mut dec = stream.make_decoder().unwrap();
        let mut all_samples = Vec::new();
        while let Some(step) = stream.next_step().unwrap() {
            match step {
                fvid::audio::AudioStep::Encoded(p) => {
                    let d = dec
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(pres) = stream.present_decoded(d.packet, d.source_pts).unwrap() {
                        all_samples.extend(f32_samples(&pres.data));
                    }
                }
                fvid::audio::AudioStep::Pcm(pres) => {
                    all_samples.extend(f32_samples(&pres.data));
                }
                fvid::audio::AudioStep::ResetDecoder => stream.reset_decoder(&mut dec).unwrap(),
            }
        }
        let peaks = peak_channels(&all_samples, 6);
        let expected = [0.3, 0.25, 0.2, 0.3, 0.15, 0.1];
        for (ch, (&got, &want)) in peaks.iter().zip(&expected).enumerate() {
            assert!(
                (got - want).abs() < 0.03,
                "{file} ch {ch}: got {got}, want {want}"
            );
        }
    }

    // 3. 2-block syncframes in MP4 and Matroska
    for file in ["blocks-2.mp4", "blocks-2.mkv"] {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        let mut stream: Box<dyn AudioStream> = if file.ends_with(".mp4") {
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            )
        } else {
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        let mut dec = stream.make_decoder().unwrap();
        let mut total_frames = 0usize;
        while let Some(step) = stream.next_step().unwrap() {
            match step {
                fvid::audio::AudioStep::Encoded(p) => {
                    let d = dec
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(pres) = stream.present_decoded(d.packet, d.source_pts).unwrap() {
                        total_frames += pres.data.len() / 4;
                    }
                }
                fvid::audio::AudioStep::Pcm(pres) => {
                    total_frames += pres.data.len() / 4;
                }
                fvid::audio::AudioStep::ResetDecoder => stream.reset_decoder(&mut dec).unwrap(),
            }
        }
        assert!(total_frames >= 1280, "{file}: total_frames={total_frames}");
    }

    // 4. Container rate mismatch is refused cleanly
    for file in ["mismatch-rate.mp4", "mismatch-rate.mkv"] {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        let mut stream: Box<dyn AudioStream> = if file.ends_with(".mp4") {
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            )
        } else {
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        assert_eq!(stream.sample_rate(), 32_000);
        let mut dec = stream.make_decoder().unwrap();
        let pkt = match stream.next_step().unwrap().unwrap() {
            fvid::audio::AudioStep::Encoded(p) => p,
            fvid::audio::AudioStep::ResetDecoder => {
                stream.reset_decoder(&mut dec).unwrap();
                match stream.next_step().unwrap().unwrap() {
                    fvid::audio::AudioStep::Encoded(p) => p,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        };
        let err = dec
            .decode_packet(&pkt.data, pkt.pts, pkt.duration as u64)
            .err()
            .unwrap_or_else(|| panic!("{file}: rate mismatch was not refused"));
        assert!(
            err.to_string().contains("frame sample rate") && err.to_string().contains("disagrees"),
            "{file}: {err}"
        );
    }
}

#[test]
fn eac3_block_switching_and_spx_attenuation_decode_matching_reference() {
    let dir = fixture_dir();

    // 1. Block switching (`blkswe = 1` with per-block `blksw[ch]` flags)
    let blksw_raw = std::fs::read(dir.join("blksw.ec3")).unwrap();
    let blksw_ref = f32_samples(&std::fs::read(dir.join("blksw.f32le")).unwrap());
    let hdr = eac3_frame(&blksw_raw).expect("blksw.ec3 header parses");
    assert!(hdr.blocks_state.block_switching);
    let mut dec = Eac3Decoder::new(&[], 48_000, 2).unwrap();
    let pkt = dec.decode_complete(&blksw_raw, 0, 0).unwrap().unwrap();
    let got = f32_samples(&pkt.data);
    let ref_trimmed = &blksw_ref[256 * 2..];
    assert_eq!(got.len(), ref_trimmed.len());
    let max_err = got
        .iter()
        .zip(ref_trimmed)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_err < 1e-5,
        "blksw.ec3 sample mismatch against reference: max_err={max_err}"
    );

    // 2. Spectral Extension boundary attenuation (`spxattene = 1`) vs unattenuated control
    let no_atten_raw = std::fs::read(dir.join("spx-no-atten.ec3")).unwrap();
    let no_atten_ref = f32_samples(&std::fs::read(dir.join("spx-no-atten.f32le")).unwrap());
    let atten_raw = std::fs::read(dir.join("spx-atten.ec3")).unwrap();
    let atten_ref = f32_samples(&std::fs::read(dir.join("spx-atten.f32le")).unwrap());

    let hdr_no = eac3_frame(&no_atten_raw).expect("spx-no-atten.ec3 header parses");
    assert_eq!(hdr_no.spx_attenuation[..2], [None, None]);
    let hdr_at = eac3_frame(&atten_raw).expect("spx-atten.ec3 header parses");
    assert_eq!(hdr_at.spx_attenuation[..2], [Some(15), Some(31)]);

    let mut dec_no = Eac3Decoder::new(&[], 48_000, 2).unwrap();
    let got_no = f32_samples(
        &dec_no
            .decode_complete(&no_atten_raw, 0, 0)
            .unwrap()
            .unwrap()
            .data,
    );
    let mut dec_at = Eac3Decoder::new(&[], 48_000, 2).unwrap();
    let got_at = f32_samples(
        &dec_at
            .decode_complete(&atten_raw, 0, 0)
            .unwrap()
            .unwrap()
            .data,
    );

    let ref_no_trimmed = &no_atten_ref[256 * 2..];
    let ref_at_trimmed = &atten_ref[256 * 2..];
    assert_eq!(got_no.len(), ref_no_trimmed.len());
    assert_eq!(got_at.len(), ref_at_trimmed.len());

    let max_err_no = got_no
        .iter()
        .zip(ref_no_trimmed)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let max_err_at = got_at
        .iter()
        .zip(ref_at_trimmed)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let atten_effect = got_no
        .iter()
        .zip(&got_at)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_err_no < 1e-5,
        "spx-no-atten.ec3 mismatch against reference: max_err={max_err_no}"
    );
    assert!(
        max_err_at < 1e-5,
        "spx-atten.ec3 mismatch against reference: max_err={max_err_at}"
    );
    assert!(
        atten_effect > 0.015,
        "spxattene should measurably attenuate SPX wrap boundaries: atten_effect={atten_effect}"
    );
}
