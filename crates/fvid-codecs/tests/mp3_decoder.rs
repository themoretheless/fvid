use fvid_codecs::{
    mp3::{Decoder, region_ends},
    mp3_frame::{Granule, Header, Version},
    mp3_hybrid::Window,
};
fn packets() -> Vec<Vec<u8>> {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mp3/tone.mp3"),
    )
    .unwrap();
    let mut at = if bytes.starts_with(b"ID3") {
        10 + bytes[6..10]
            .iter()
            .fold(0usize, |n, &v| (n << 7) | usize::from(v & 127))
    } else {
        0
    };
    let mut result = Vec::new();
    while at < bytes.len() {
        let h = Header::parse(&bytes[at..]).unwrap();
        result.push(bytes[at..at + h.frame_bytes].to_vec());
        at += h.frame_bytes;
    }
    result
}
#[test]
fn full_decoder_rewinds_and_all_truncated_frames_preserve_live_history() {
    let frames = packets();
    let mut decoder = Decoder::default();
    let mut expected = Vec::new();
    for frame in &frames {
        let pcm = decoder.decode(frame).unwrap();
        assert_eq!((pcm.sample_rate, pcm.channels), (48000, 1));
        assert_eq!(pcm.samples.len(), 1152);
        expected.push(pcm.samples);
    }
    decoder.reset();
    for (i, frame) in frames.iter().enumerate() {
        for length in 0..frame.len() {
            assert!(
                decoder.decode(&frame[..length]).is_err(),
                "frame {i} prefix {length}"
            );
        }
        assert_eq!(
            decoder.decode(frame).unwrap().samples,
            expected[i],
            "failed input changed history"
        );
    }
    decoder.reset();
    for (frame, expected) in frames.iter().zip(expected) {
        assert_eq!(decoder.decode(frame).unwrap().samples, expected);
    }
}
#[test]
fn every_normal_region_count_and_switched_boundary_is_in_frequency_lines() {
    use fvid_codecs::mp3_spectral::Bands;
    for rate in [44100, 48000, 32000, 22050, 24000, 16000, 11025, 12000, 8000] {
        let version = if rate >= 32000 {
            Version::Mpeg1
        } else if rate >= 16000 {
            Version::Mpeg2
        } else {
            Version::Mpeg25
        };
        let bands = Bands::for_rate(rate).unwrap();
        for a in 0..16 {
            for b in 0..8 {
                let g = Granule {
                    regions: Some((a, b)),
                    ..Granule::default()
                };
                if a + b <= 20 {
                    assert_eq!(
                        region_ends(version, rate, &g).unwrap(),
                        [
                            bands.long[usize::from(a) + 1],
                            bands.long[usize::from(a + b) + 2]
                        ]
                    );
                } else {
                    assert!(region_ends(version, rate, &g).is_err());
                }
            }
        }
        for window in [Window::Start, Window::Short, Window::Stop] {
            for mixed in [false, true] {
                let g = Granule {
                    window,
                    mixed,
                    ..Granule::default()
                };
                let first = match (window, rate, version) {
                    (Window::Short, 8000, _) => 72,
                    (Window::Short, _, _) => 36,
                    (_, _, Version::Mpeg1) => 36,
                    (_, 8000, _) => 108,
                    _ => 54,
                };
                assert_eq!(region_ends(version, rate, &g).unwrap(), [first, 576]);
            }
        }
    }
    assert!(region_ends(Version::Mpeg1, 44100, &Granule::default()).is_err());
}

#[test]
fn authored_compressed_chains_match_independent_pcm_including_mixed_start_stop() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp3-decoder");
    for rate in [48000, 24000, 8000, 44100, 22050, 11025, 32000, 16000, 12000] {
        for channels in [1, 2] {
            let name = format!("{rate}-{channels}ch");
            let bytes = std::fs::read(root.join(format!("{name}.mp3"))).unwrap();
            let expected = std::fs::read(root.join(format!("{name}.f32le"))).unwrap();
            let expected: Vec<f32> = expected
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            let mut decoder = Decoder::default();
            let mut at = 0;
            let mut actual = Vec::new();
            while at < bytes.len() {
                let header = Header::parse(&bytes[at..]).unwrap();
                let pcm = decoder.decode(&bytes[at..at + header.frame_bytes]).unwrap();
                assert_eq!((pcm.sample_rate, pcm.channels), (rate, channels));
                actual.extend(pcm.samples);
                at += header.frame_bytes;
            }
            assert_eq!(actual.len(), expected.len());
            let mut peak = 0f64;
            let mut squared = 0f64;
            for (a, b) in actual.iter().zip(&expected) {
                let error = f64::from(a - b);
                peak = peak.max(error.abs());
                squared += error * error;
            }
            let rms = (squared / actual.len() as f64).sqrt();
            eprintln!("{name}: peak {peak}, RMS {rms}");
            assert!(peak < 3e-7 && rms < 3e-8, "{name}: peak {peak}, RMS {rms}");
            assert!(
                actual.iter().any(|v| v.abs() > 1e-5),
                "non-silent acceptance required"
            );
        }
    }
}

#[test]
fn free_spacing_errors_preserve_history_and_reset_allows_a_new_stream() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors");
    let raw = std::fs::read(root.join("mp3-free/48000-1ch-control.mp3")).unwrap();
    let expected = std::fs::read(root.join("mp3-free/48000-1ch-control.f32le")).unwrap();
    let first = &raw[..960];
    let second = &raw[960..1921];
    let mut decoder = Decoder::default();
    decoder.decode(first).unwrap();
    let mut wrong = second.to_vec();
    wrong.push(0);
    let error = decoder.decode(&wrong).err().unwrap();
    assert!(error.to_string().contains("spacing changed"));
    let indexed = std::fs::read(root.join("mp3-decoder/48000-1ch.mp3")).unwrap();
    assert!(decoder.decode(&indexed[..960]).is_err());
    let actual = decoder.decode(second).unwrap();
    let bytes: Vec<u8> = actual
        .samples
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    assert_eq!(bytes, expected[1152 * 4..2304 * 4]);
    decoder.reset();
    assert!(decoder.decode(&indexed[..960]).is_ok());
    assert!(decoder.decode(first).is_err());
    decoder.reset();
    assert!(decoder.decode(first).is_ok());
}
