use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
    time::Duration,
};
#[test]
fn opus_library_preserves_one_tick_gaps_and_positive_origins() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/opus-clock");
    let head = [
        b"OpusHead".as_slice(),
        &[1, 1, 0, 0, 0x80, 0xbb, 0, 0, 0, 0, 0],
    ]
    .concat();
    for (name, times) in [("gap", [0, 21, 41]), ("offset", [1, 21, 41])] {
        let mut decoder = fvid_media::owned_opus::OpusDecoder::new(&head, 48000, 1).unwrap();
        let mut expected = Vec::new();
        for time in times {
            expected.resize(time * 48 * 4, 0);
            expected.extend(
                decoder
                    .decode(&[0xf8, 0xff, 0xfe])
                    .unwrap()
                    .iter()
                    .flat_map(|v| v.to_le_bytes()),
            );
        }
        let movie = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
        let directory =
            std::env::temp_dir().join(format!("fvid-opus-clock-{}-{name}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let wave = directory.join("audio.wav");
        fvid_media::decode_audio(
            &root.join(format!("{name}.mkv")),
            &wave,
            &Default::default(),
        )
        .unwrap();
        let mut file = std::fs::File::open(wave).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert!(info.float);
        assert_eq!(info.bits_per_sample, 32);
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut public = vec![0; info.data_bytes as usize];
        file.read_exact(&mut public).unwrap();
        assert_eq!(public, expected, "{name} public WAVE");

        for interval in [
            None,
            Some((Duration::from_millis(19), Duration::from_millis(22))),
        ] {
            let wanted = if interval.is_some() {
                &expected[19 * 48 * 4..22 * 48 * 4]
            } else {
                &expected
            };
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
                Cursor::new(&movie),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(pcm.len(), wanted.len(), "{name} timeline length");
            assert_eq!(pcm, wanted, "{name} timeline PCM");
            assert_eq!(stats.sample_frames, wanted.len() as u64 / 4);
        }
    }
}

#[test]
fn short_opus_fractional_grid_exports_exact_pcm_and_intervals() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/opus-clock");
    for name in ["short-third", "short-shift-third", "short-lace-third"] {
        let source = root.join(format!("{name}.mkv"));
        let head = [
            b"OpusHead".as_slice(),
            &[1, 1, 0, 0, 0x80, 0xbb, 0, 0, 0, 0, 0],
        ]
        .concat();
        let mut decoder = fvid_media::owned_opus::OpusDecoder::new(&head, 48000, 1).unwrap();
        let mut expected = Vec::new();
        for _ in 0..4 {
            let frame = decoder.decode(&[0x80, 0xff, 0xfe]).unwrap();
            assert_eq!(frame.len(), 120);
            expected.extend(frame.iter().flat_map(|v| v.to_le_bytes()));
        }
        let bytes = std::fs::read(source).unwrap();
        for interval in [
            None,
            Some((Duration::from_millis(3), Duration::from_millis(7))),
        ] {
            let wanted = if interval.is_some() {
                &expected[144 * 4..336 * 4]
            } else {
                &expected
            };
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
                Cursor::new(&bytes),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(pcm, wanted);
            assert_eq!(stats.sample_frames, wanted.len() as u64 / 4);
        }
    }
}

#[test]
fn opus_rounded_delay_and_signed_padding_export_exact_windows() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/opus-clock");
    let head = [
        b"OpusHead".as_slice(),
        &[1, 1, 17, 0, 0x80, 0xbb, 0, 0, 0, 0, 0],
    ]
    .concat();
    for (name, trim) in [("short-delay-tail", 0), ("short-delay-head-tail", 9)] {
        let mut decoder = fvid_media::owned_opus::OpusDecoder::new(&head, 48000, 1).unwrap();
        let mut source = Vec::new();
        for _ in 0..4 {
            source.extend(
                decoder
                    .decode(&[0x80, 0xff, 0xfe])
                    .unwrap()
                    .iter()
                    .flat_map(|v| v.to_le_bytes()),
            );
        }
        let mut expected = source[17 * 4..(480 - 13) * 4].to_vec();
        expected[(240 - 17) * 4..(240 - 17 + trim) * 4].fill(0);
        let bytes = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
        for interval in [
            None,
            Some((Duration::from_nanos(1_000_001), Duration::from_millis(4))),
        ] {
            let wanted = if interval.is_some() {
                &expected[49 * 4..192 * 4]
            } else {
                &expected
            };
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
                Cursor::new(&bytes),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(pcm, wanted, "{name}");
            assert_eq!(stats.sample_frames, wanted.len() as u64 / 4);
        }
    }
}
