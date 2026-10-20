use std::io::Write;
#[test]
fn every_named_speaker_has_a_defined_stereo_fold_in_both_precisions() {
    let k = std::f64::consts::FRAC_1_SQRT_2;
    let expected = [
        [1., 0.],
        [0., 1.],
        [1., 1.],
        [0., 0.],
        [k, 0.],
        [0., k],
        [k, 0.],
        [0., k],
        [k, k],
        [k, 0.],
        [0., k],
        [k, k],
        [k, 0.],
        [k, k],
        [0., k],
        [k, 0.],
        [k, k],
        [0., k],
    ];
    for (speaker, wanted) in expected.iter().enumerate() {
        let mut out = Vec::new();
        let mut mix =
            fvid_media::owned_pcm_gain_f64::PcmGain::with_mask(&mut out, 1., 1, 2, 1 << speaker)
                .unwrap();
        mix.write_all(&1f64.to_le_bytes()).unwrap();
        drop(mix);
        let values: Vec<_> = out
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(values, wanted, "speaker {speaker}");
        let mut out = Vec::new();
        let mut mix =
            fvid_media::owned_pcm_gain::PcmGain::with_mask(&mut out, 1., 1, 2, 1 << speaker)
                .unwrap();
        mix.write_all(&1f32.to_le_bytes()).unwrap();
        drop(mix);
        let values: Vec<_> = out
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(values, wanted.map(|v| v as f32), "speaker {speaker}");
    }
}
#[test]
fn partial_masks_ignore_unassigned_channels_and_preserve_fragmented_frames() {
    for (mask, expected) in [(3, [1f64, 2.]), (0, [0., 0.])] {
        let mut out = Vec::new();
        let mut mix =
            fvid_media::owned_pcm_gain_f64::PcmGain::with_mask(&mut out, 1., 4, 2, mask).unwrap();
        for value in [1f64, 2., 100., 200.] {
            mix.write_all(&value.to_le_bytes()).unwrap();
        }
        assert!(mix.frame_complete());
        drop(mix);
        let values: Vec<_> = out
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(values, expected);
    }
}
#[test]
fn expansion_routes_matching_speakers_and_folds_only_missing_positions() {
    for (mask, output, expected) in [
        (8, 6, vec![0f64, 0., 0., 1., 0., 0.]),
        (4, 3, vec![0., 0., 1.]),
        (256, 4, vec![0., 0., 0., 1.]),
        (
            1 << 12,
            6,
            vec![std::f64::consts::FRAC_1_SQRT_2, 0., 0., 0., 0., 0.],
        ),
    ] {
        let mut bytes = Vec::new();
        let mut mix =
            fvid_media::owned_pcm_gain_f64::PcmGain::with_mask(&mut bytes, 1., 1, output, mask)
                .unwrap();
        mix.write_all(&1f64.to_le_bytes()).unwrap();
        drop(mix);
        let actual: Vec<_> = bytes
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(actual, expected);
    }
}
#[test]
fn explicit_directout_zero_is_distinct_from_classic_wave_implicit_mono() {
    use std::{
        io::{Read, Seek, SeekFrom},
        path::PathBuf,
    };
    let dir = std::env::temp_dir().join(format!("fvid-directout-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let mut classic = b"RIFF".to_vec();
    classic.extend_from_slice(&40u32.to_le_bytes());
    classic.extend_from_slice(b"WAVEfmt ");
    classic.extend_from_slice(&16u32.to_le_bytes());
    classic.extend_from_slice(&3u16.to_le_bytes());
    classic.extend_from_slice(&1u16.to_le_bytes());
    classic.extend_from_slice(&8000u32.to_le_bytes());
    classic.extend_from_slice(&32000u32.to_le_bytes());
    classic.extend_from_slice(&4u16.to_le_bytes());
    classic.extend_from_slice(&32u16.to_le_bytes());
    classic.extend_from_slice(b"data");
    classic.extend_from_slice(&4u32.to_le_bytes());
    classic.extend_from_slice(&0.25f32.to_le_bytes());
    let mut explicit = fvid_media::owned_wav::float_wav_header_with_mask(8000, 1, 1, 0).unwrap();
    explicit.extend_from_slice(&0.25f32.to_le_bytes());
    for (label, source, wanted, declared) in [
        ("classic", classic, [0.25f32, 0.25], false),
        ("explicit", explicit, [0., 0.], true),
    ] {
        let input = dir.join(format!("{label}-input.wav"));
        let output = dir.join(format!("{label}-output.wav"));
        std::fs::write(&input, source).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(
            &mut std::fs::File::open(&input).unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(info.channel_mask_explicit, declared);
        fvid_media::decode_audio_transformed(
            &input,
            &output,
            fvid_media_info::AudioDecodeTransform {
                channels: Some(2),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let mut file = std::fs::File::open(output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut bytes = vec![0; info.data_bytes as usize];
        file.read_exact(&mut bytes).unwrap();
        let actual: Vec<_> = bytes
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(actual, wanted);
    }
}
