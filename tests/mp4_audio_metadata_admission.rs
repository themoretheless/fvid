use fvid::media::{AudioDecodeTransform, CopyOptions};
use std::path::PathBuf;
fn source(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-metadata-admission")
        .join(name)
}
#[test]
fn audio_geometry_respects_parser_metadata_ceiling() {
    let options = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let error =
        fvid_media::owned_audio_plan::source_geometry(&source("padded.m4a"), &options).unwrap_err();
    assert!(error.contains("metadata exceeds budget"), "{error}");
}
#[test]
fn audio_decode_plan_respects_parser_metadata_ceiling() {
    let options = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let error = fvid::media::plan_decode_audio(
        &source("padded.m4a"),
        &AudioDecodeTransform::default(),
        &options,
    )
    .unwrap_err();
    assert!(error.contains("metadata exceeds budget"), "{error}");
}
#[test]
fn small_and_ample_budget_audio_controls_are_valid() {
    for (name, budget) in [("small.m4a", 1 << 20), ("padded.m4a", 8 << 20)] {
        let options = CopyOptions {
            max_controlled_bytes: Some(budget),
            ..Default::default()
        };
        assert_eq!(
            fvid_media::owned_audio_plan::source_geometry(&source(name), &options).unwrap(),
            (8000, 1)
        );
        assert!(
            fvid::media::plan_decode_audio(
                &source(name),
                &AudioDecodeTransform::default(),
                &options
            )
            .is_ok()
        );
    }
}
#[test]
fn native_probes_respect_the_same_metadata_ceiling() {
    let path = source("padded.m4a");
    let options = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let error =
        fvid::native_media::is_aac_source_with_metadata_limit(&path, options.max_controlled_bytes)
            .unwrap_err();
    assert!(
        error.to_string().contains("metadata exceeds budget"),
        "{error}"
    );
    let error =
        match fvid::native_media::audio_source_info_selected_with_options(&path, None, &options) {
            Ok(_) => panic!("native audio probe must refuse oversized metadata"),
            Err(error) => error,
        };
    assert!(
        error.to_string().contains("metadata exceeds budget"),
        "{error}"
    );
}
struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let p =
            std::env::temp_dir().join(format!("fvid-audio-metadata-{name}-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn decode_refuses_before_output_and_leaves_no_temporary_files() {
    let dir = TempDir::new("refusal");
    let options = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    for route in 0..2 {
        let output = dir.0.join(format!("out-{route}.wav"));
        let result = if route == 0 {
            fvid::media::decode_audio(&source("padded.m4a"), &output, &options)
        } else {
            fvid_media::decode_audio(&source("padded.m4a"), &output, &options)
        };
        let error = result.unwrap_err();
        assert!(error.contains("metadata exceeds budget"), "{error}");
        assert!(!output.exists());
    }
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
}
fn wave_chunks(data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    assert_eq!(&data[..4], b"RIFF");
    assert_eq!(&data[8..12], b"WAVE");
    let mut at = 12;
    let mut format = None;
    let mut pcm = None;
    while at + 8 <= data.len() {
        let size = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &data[at + 8..at + 8 + size];
        match &data[at..at + 4] {
            b"fmt " => format = Some(body.to_vec()),
            b"data" => pcm = Some(body.to_vec()),
            _ => {}
        }
        at += 8 + size + (size % 2);
    }
    (format.unwrap(), pcm.unwrap())
}
#[test]
fn ample_budget_decodes_exactly_the_same_pcm_as_unpadded_input() {
    let dir = TempDir::new("acceptance");
    let options = CopyOptions {
        max_controlled_bytes: Some(8 << 20),
        ..Default::default()
    };
    let mut reference = None;
    for name in ["small.m4a", "padded.m4a"] {
        let output = dir.0.join(format!("{name}.wav"));
        let stats = fvid::media::decode_audio(&source(name), &output, &options).unwrap();
        assert_eq!(stats.sample_frames, 136);
        assert_eq!(stats.sample_rate, 8000);
        assert_eq!(stats.channels, 1);
        let chunks = wave_chunks(&std::fs::read(output).unwrap());
        assert!(!chunks.1.is_empty());
        if let Some(expected) = &reference {
            assert_eq!(&chunks, expected);
        } else {
            reference = Some(chunks);
        }
    }
}

#[test]
fn loudness_loudnorm_mix_merge_concat_and_spatial_routing_respect_mp4_metadata_ceiling() {
    let dir = TempDir::new("audio-routes");
    let padded = source("padded.m4a");
    let small = source("small.m4a");
    let tight = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let ample = CopyOptions {
        max_controlled_bytes: Some(8 << 20),
        ..Default::default()
    };

    // 1. Owned MP4 audio export qualification (`supports_plan`) keeps the owned route under tight budget
    assert!(fvid_media::owned_container_loudnorm::supports_plan(
        &padded, None, &tight,
    ));

    // 2. Loudness and loudnorm (plan and execution across fvid and fvid_media)
    let out_wav = dir.0.join("out.wav");
    let out_mka = dir.0.join("out.mka");
    for err in [
        fvid::media::plan_loudness(&padded, &tight).unwrap_err(),
        fvid::media::measure_loudness(&padded, &tight).unwrap_err(),
        fvid_media::plan_loudness(&padded, &tight).unwrap_err(),
        fvid_media::measure_loudness(&padded, &tight).unwrap_err(),
        fvid::media::plan_loudnorm(&padded, None, false, &tight).unwrap_err(),
        fvid::media::plan_loudnorm(&padded, None, true, &tight).unwrap_err(),
        fvid::media::apply_loudnorm(&padded, &out_wav, None, &tight).unwrap_err(),
        fvid::media::apply_loudnorm_dual(&padded, &out_wav, None, &tight).unwrap_err(),
        fvid_media::plan_loudnorm(&padded, None, false, &tight).unwrap_err(),
        fvid_media::plan_loudnorm(&padded, None, true, &tight).unwrap_err(),
        fvid_media::apply_loudnorm(&padded, &out_wav, None, &tight).unwrap_err(),
        fvid_media::apply_loudnorm_dual(&padded, &out_wav, None, &tight).unwrap_err(),
    ] {
        assert!(err.contains("metadata exceeds budget"), "{err}");
    }
    assert!(!out_wav.exists());

    // 3. Mix, merge, and sample-domain concat (plan and execution)
    let pair_padded = vec![padded.clone(), padded.clone()];
    let pair_small = vec![small.clone(), small.clone()];
    for err in [
        fvid::media::plan_mix_audio_with_controls(&pair_padded, &Default::default(), &tight)
            .unwrap_err(),
        fvid::media::mix_audio_with_controls(&pair_padded, &out_wav, &Default::default(), &tight)
            .unwrap_err(),
        fvid::media::plan_merge_audio_with_controls(&pair_padded, &tight).unwrap_err(),
        fvid::media::merge_audio_with_controls(&pair_padded, &out_wav, &tight).unwrap_err(),
        fvid::native_plan::concat_matroska_to_with_options(&pair_padded, &out_wav, &tight)
            .unwrap_err(),
        fvid::native_audio_mix::concat_audio_with_options(&pair_padded, &out_wav, &tight)
            .unwrap_err()
            .to_string(),
        fvid::media::decode_audio(&padded, &out_mka, &tight).unwrap_err(),
        fvid::media::trim(&padded, &out_wav, 0, 17_000, &tight).unwrap_err(),
    ] {
        assert!(err.contains("metadata exceeds budget"), "{err}");
    }
    assert!(!out_wav.exists());
    assert!(!out_mka.exists());

    // 4. Positive controls: small input under 1 MiB and padded input under 8 MiB succeed
    for (pair, single, opts) in [
        (&pair_small, &small, &tight),
        (&pair_padded, &padded, &ample),
    ] {
        fvid::media::plan_loudness(single, opts).unwrap();
        fvid::media::measure_loudness(single, opts).unwrap();
        fvid_media::plan_loudness(single, opts).unwrap();
        fvid_media::measure_loudness(single, opts).unwrap();
        fvid::media::plan_loudnorm(single, None, false, opts).unwrap();
        let linear_args = Some("I=-24:TP=-2:LRA=7:measured_I=-24:measured_TP=-10:measured_LRA=2:measured_thresh=-34:linear=true");
        fvid::media::plan_loudnorm(single, linear_args, false, opts).unwrap();
        fvid::media::apply_loudnorm(single, &out_wav, linear_args, opts).unwrap();
        std::fs::remove_file(&out_wav).unwrap();
        fvid::media::plan_mix_audio_with_controls(pair, &Default::default(), opts).unwrap();
        let mix_stats =
            fvid::media::mix_audio_with_controls(pair, &out_wav, &Default::default(), opts)
                .unwrap();
        assert_eq!(mix_stats.sample_frames, 136);
        std::fs::remove_file(&out_wav).unwrap();
        fvid::media::plan_merge_audio_with_controls(pair, opts).unwrap();
        let merge_stats = fvid::media::merge_audio_with_controls(pair, &out_wav, opts).unwrap();
        assert_eq!((merge_stats.sample_frames, merge_stats.channels), (136, 2));
        std::fs::remove_file(&out_wav).unwrap();
        fvid::native_plan::concat_matroska_to_with_options(pair, &out_wav, opts)
            .unwrap()
            .unwrap();
        let concat_stats =
            fvid::native_audio_mix::concat_audio_with_options(pair, &out_wav, opts).unwrap();
        assert_eq!(concat_stats.sample_frames, 272);
        std::fs::remove_file(&out_wav).unwrap();
        let mka_stats = fvid::media::decode_audio(single, &out_mka, opts).unwrap();
        assert_eq!(mka_stats.sample_frames, 136);
        std::fs::remove_file(&out_mka).unwrap();
    }

    // 5. Multichannel AAC spatial routing in MP4 (`owned_aac_export_routing`) respects metadata ceiling
    let multichannel_mp4 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-device-layout/normal-ten.mp4");
    let fold_stereo = AudioDecodeTransform {
        channels: Some(2),
        ..Default::default()
    };
    let tiny_meta = CopyOptions {
        max_controlled_bytes: Some(64),
        ..Default::default()
    };
    let fold_err =
        fvid_media::plan_decode_audio(&multichannel_mp4, &fold_stereo, &tiny_meta).unwrap_err();
    assert!(fold_err.contains("metadata exceeds budget"), "{fold_err}");
}

#[test]
fn native_plan_audio_trim_and_decode_entrypoints_enforce_mp4_metadata_and_matroska_packet_index_budgets() {
    let padded = source("padded.m4a");
    let small = source("small.m4a");
    let tight_mp4 = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let ample_mp4 = CopyOptions {
        max_controlled_bytes: Some(8 << 20),
        ..Default::default()
    };

    let multichannel_mp4 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-device-layout/normal-ten.mp4");
    let tight_aac = CopyOptions {
        streams: vec![1],
        max_controlled_bytes: Some(64),
        ..Default::default()
    };
    let ample_aac = CopyOptions {
        streams: vec![1],
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };

    for err in [
        fvid::native_plan::decode_audio_with_options(
            &padded,
            &AudioDecodeTransform::default(),
            &tight_mp4,
        )
        .unwrap_err(),
        fvid::native_plan::trim_audio_with_options(&padded, 0, 17_000, &tight_mp4).unwrap_err(),
        fvid::native_plan::trim_aac_with_options(&multichannel_mp4, 0, 17_000, &tight_aac)
            .unwrap_err(),
        fvid::native_plan::audio_trim_selection_with_options(&padded, None, &tight_mp4)
            .unwrap_err(),
    ] {
        assert!(err.contains("metadata exceeds budget"), "{err}");
    }
    assert!(
        !fvid::native_media::is_owned_audio_trim_source_with_options(&padded, &tight_mp4)
            .unwrap_or(false)
    );
    fvid::native_plan::trim_aac_with_options(&multichannel_mp4, 0, 17_000, &ample_aac).unwrap();

    for (input, opts) in [(&small, &tight_mp4), (&padded, &ample_mp4)] {
        fvid::native_plan::decode_audio_with_options(
            input,
            &AudioDecodeTransform::default(),
            opts,
        )
        .unwrap();
        fvid::native_plan::trim_audio_with_options(input, 0, 17_000, opts).unwrap();
        assert_eq!(
            fvid::native_plan::audio_trim_selection_with_options(input, None, opts).unwrap(),
            0
        );
        assert!(fvid::native_media::is_owned_audio_trim_source_with_options(input, opts).unwrap());
    }

    let dir = TempDir::new("mkv-audio-packet-index");
    let mkv_audio = dir.0.join("small-audio.mka");
    fvid::media::decode_audio(&small, &mkv_audio, &CopyOptions::default()).unwrap();
    let tight_mkv = CopyOptions {
        max_controlled_bytes: Some(40),
        ..Default::default()
    };
    let ample_mkv = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };
    for err in [
        fvid::native_plan::decode_audio_with_options(
            &mkv_audio,
            &AudioDecodeTransform::default(),
            &tight_mkv,
        )
        .unwrap_err(),
        fvid::native_plan::trim_audio_with_options(&mkv_audio, 0, 17_000, &tight_mkv).unwrap_err(),
        fvid::native_plan::audio_trim_selection_with_options(&mkv_audio, None, &tight_mkv)
            .unwrap_err(),
        match fvid::native_media::audio_source_info_selected_with_options(
            &mkv_audio, None, &tight_mkv,
        ) {
            Ok(_) => panic!("audio_source_info_selected_with_options must refuse Matroska packet index over budget"),
            Err(error) => error.to_string(),
        },
    ] {
        assert!(
            err.contains("controlled memory budget exceeded: Matroska packet index limit"),
            "{err}"
        );
    }
    fvid::native_plan::decode_audio_with_options(
        &mkv_audio,
        &AudioDecodeTransform::default(),
        &ample_mkv,
    )
    .unwrap();
    fvid::native_plan::trim_audio_with_options(&mkv_audio, 0, 17_000, &ample_mkv).unwrap();
}

#[test]
fn mix_merge_concat_and_loudnorm_matroska_output_and_eligibility_propagate_controlled_memory_budgets() {
    let dir = TempDir::new("mix-merge-concat-mka-budget");
    let small = source("small.m4a");
    let padded = source("padded.m4a");
    let six_packet_mkv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-device-layout/normal-ten.mkv");

    // 1.Eligibility checks (`native_audio_mix::eligible_with_options`, `concat_eligible_with_options`)
    //    must reject oversized MP4 metadata (padded.m4a under 1 MiB) and Matroska packet indices
    //    (normal-ten.mkv under 200 bytes) without falling back to default_controls.
    let tight_mp4 = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let tight_mkv = CopyOptions {
        max_controlled_bytes: Some(200),
        ..Default::default()
    };
    let pair_padded = vec![padded.clone(), padded.clone()];
    let pair_mkv = vec![six_packet_mkv.clone(), six_packet_mkv.clone()];
    assert!(
        !fvid::native_audio_mix::eligible_with_options(&pair_padded, &tight_mp4).unwrap_or(false)
    );
    assert!(
        !fvid::native_audio_mix::concat_eligible_with_options(&pair_padded, &tight_mp4)
            .unwrap_or(false)
    );
    assert!(
        !fvid::native_audio_mix::eligible_with_options(&pair_mkv, &tight_mkv).unwrap_or(false)
    );
    assert!(
        !fvid::native_audio_mix::concat_eligible_with_options(&pair_mkv, &tight_mkv)
            .unwrap_or(false)
    );

    // 1b. When `max_packets` is exhausted by the first concat segment (`small.m4a`, 1 packet),
    //     geometry validation of trailing segments (`padded.m4a`) must still propagate
    //     `max_controlled_bytes` rather than falling back to unconstrained `default_probe`.
    let one_packet_tight_mp4 = CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let small_then_padded = vec![small.clone(), padded.clone()];
    assert!(
        !fvid::native_audio_mix::concat_eligible_with_options(
            &small_then_padded,
            &one_packet_tight_mp4
        )
        .unwrap_or(false)
    );
    let out_concat_wav = dir.0.join("concat-small-padded.wav");
    let concat_err = fvid::native_plan::concat_matroska_to_with_options(
        &small_then_padded,
        &out_concat_wav,
        &one_packet_tight_mp4,
    )
    .unwrap_err()
    .to_string();
    assert!(
        concat_err.contains("metadata exceeds budget") || concat_err.contains("packet budget"),
        "{concat_err}"
    );
    assert!(!out_concat_wav.exists());

    // 2. Mix, merge, and loudnorm to `.mka` must enforce `max_controlled_bytes` during
    //    `extract_source_pcm_metadata` as well as planning/decoding.
    let out_mka = dir.0.join("out.mka");
    for err in [
        fvid::media::mix_audio_with_controls(
            &pair_padded,
            &out_mka,
            &Default::default(),
            &tight_mp4,
        )
        .unwrap_err(),
        fvid::media::merge_audio_with_controls(&pair_padded, &out_mka, &tight_mp4).unwrap_err(),
        fvid::media::apply_loudnorm(&padded, &out_mka, None, &tight_mp4).unwrap_err(),
    ] {
        assert!(err.contains("metadata exceeds budget"), "{err}");
    }
    assert!(!out_mka.exists());

    // 3. Positive controls with ample 256 KiB budget on small.m4a succeed for `.mka` outputs
    let ample = CopyOptions {
        max_controlled_bytes: Some(256 << 10),
        ..Default::default()
    };
    let pair_small = vec![small.clone(), small.clone()];
    assert!(fvid::native_audio_mix::eligible_with_options(&pair_small, &ample).unwrap());
    assert!(fvid::native_audio_mix::concat_eligible_with_options(&pair_small, &ample).unwrap());
    let mix_stats =
        fvid::media::mix_audio_with_controls(&pair_small, &out_mka, &Default::default(), &ample)
            .unwrap();
    assert_eq!(mix_stats.sample_frames, 136);
    std::fs::remove_file(&out_mka).unwrap();
    let merge_stats =
        fvid::media::merge_audio_with_controls(&pair_small, &out_mka, &ample).unwrap();
    assert_eq!((merge_stats.sample_frames, merge_stats.channels), (136, 2));
    std::fs::remove_file(&out_mka).unwrap();
    let linear_args = Some("I=-24:TP=-2:LRA=7:measured_I=-24:measured_TP=-10:measured_LRA=2:measured_thresh=-34:linear=true");
    fvid::media::apply_loudnorm(&small, &out_mka, linear_args, &ample).unwrap();
    std::fs::remove_file(&out_mka).unwrap();
}

#[test]
fn adts_packet_limit_clamps_to_controlled_memory_and_wave_loudness_plan_notes_report_budgets() {
    let dir = TempDir::new("adts-wave-budget-contract");
    let adts_pce = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-pce-wide8.aac");
    let tiny_budget = CopyOptions {
        max_controlled_bytes: Some(8),
        ..Default::default()
    };
    // 1. ADTS PCE bootstrap frame payload exceeds 8-byte controlled memory budget before packet allocation
    for err in [
        fvid_media::plan_decode_audio(&adts_pce, &AudioDecodeTransform::default(), &tiny_budget)
            .unwrap_err(),
        fvid_media::plan_loudness(&adts_pce, &tiny_budget).unwrap_err(),
        fvid_media::measure_loudness(&adts_pce, &tiny_budget).unwrap_err(),
    ] {
        assert!(
            err.contains("ADTS packet exceeds budget")
                || err.contains("controlled memory budget exceeded"),
            "{err}"
        );
    }
    let mut sink = Vec::new();
    let stream_err = fvid_media::owned_aac::decode_adts_pcm(
        std::fs::File::open(&adts_pce).unwrap(),
        &mut sink,
        None,
        &tiny_budget,
    )
    .unwrap_err()
    .to_string();
    assert!(
        stream_err.contains("ADTS packet exceeds budget"),
        "{stream_err}"
    );

    // 2. Owned WAVE `plan_loudness` emits the standard budget note when `CopyOptions` has non-default budgets
    let wav = dir.0.join("tone.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav, 48_000, 1, &[0.1; 4800]).unwrap();
    let budgeted_wav_opts = CopyOptions {
        max_controlled_bytes: Some(2 << 20),
        max_packets: Some(2),
        ..Default::default()
    };
    let wav_plan = fvid_media::plan_loudness(&wav, &budgeted_wav_opts).unwrap();
    assert!(
        wav_plan.notes.iter().any(|note| note.contains(
            "packet work limit: 2 packets; controlled memory limit: 2097152 bytes"
        )),
        "expected budget note in WAVE loudness plan: {:?}",
        wav_plan.notes
    );
}

#[test]
fn native_probe_and_pcm_matroska_metadata_extraction_enforce_budget_across_wave_ogg_flac_mpeg_adts_and_spool_exports() {
    let dir = TempDir::new("native-probe-pcm-matroska-budget");
    let wav_info = dir.0.join("info.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_info, 48_000, 1, &[0.1; 480]).unwrap();
    let mut wav_bytes = std::fs::read(&wav_info).unwrap();
    let comment = vec![b'B'; 4096];
    let list_payload_len = 4 + 8 + comment.len();
    wav_bytes.extend_from_slice(b"LIST");
    wav_bytes.extend_from_slice(&(list_payload_len as u32).to_le_bytes());
    wav_bytes.extend_from_slice(b"INFOICMT");
    wav_bytes.extend_from_slice(&(comment.len() as u32).to_le_bytes());
    wav_bytes.extend_from_slice(&comment);
    let riff_size = u32::try_from(wav_bytes.len() - 8).unwrap();
    wav_bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
    std::fs::write(&wav_info, &wav_bytes).unwrap();

    // 1. `native_probe::try_probe_as_with_options` and `native_audio_mix::eligible_with_options`
    // must enforce `max_controlled_bytes` on WAVE `LIST INFO`, Ogg, FLAC, MPEG, and ADTS sources.
    let flac_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-streaminfo/valid_baseline.flac");
    let mp3_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp3-id3-metadata/tagged-id3v24.mp3");
    let ogg_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-g711-loudnorm-budget/ogg-opus-mono-4byte-pkt.ogg");
    let adts_fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-mono-44k.aac");

    let tight_wav = CopyOptions {
        max_controlled_bytes: Some(1024),
        ..Default::default()
    };
    let tight_wav_err = fvid::native_audio_mix::eligible_with_options(
        &[wav_info.clone(), wav_info.clone()],
        &tight_wav,
    );
    assert!(
        !tight_wav_err.unwrap_or(false),
        "eligible_with_options must reject WAVE LIST INFO exceeding max_controlled_bytes"
    );
    let out_mka = dir.0.join("out.mka");
    for err in [
        fvid::media::decode_audio(&wav_info, &out_mka, &tight_wav).unwrap_err(),
        fvid::media::apply_loudnorm(&wav_info, &out_mka, None, &tight_wav).unwrap_err(),
    ] {
        assert!(
            err.contains("memory budget") || err.contains("budget exceeded"),
            "unexpected error for WAVE -> .mka under tight budget: {err}"
        );
    }
    assert!(!out_mka.exists());

    // 2. Ogg, FLAC, and MP3 sources in `native_audio_mix::eligible_with_options`, `plan_with_controls`,
    // `mix_audio_with_controls`, and `concat_audio_with_options` must propagate `max_controlled_bytes`
    // through `native_probe::try_probe_as_with_options` and `extract_source_pcm_metadata_with_interval_packet_and_memory_limit`.
    let tiny_mem = CopyOptions {
        max_controlled_bytes: Some(64),
        ..Default::default()
    };
    for source in [&ogg_fixture, &flac_fixture, &mp3_fixture, &adts_fixture] {
        let pair = vec![source.clone(), source.clone()];
        let elig = fvid::native_audio_mix::eligible_with_options(&pair, &tiny_mem);
        assert!(
            elig.is_err() || !elig.unwrap(),
            "eligible_with_options must not admit {} under 64-byte controlled memory budget",
            source.display()
        );
        let concat_elig = fvid::native_audio_mix::concat_eligible_with_options(&pair, &tiny_mem);
        assert!(
            concat_elig.is_err() || !concat_elig.unwrap(),
            "concat_eligible_with_options must not admit {} under 64-byte controlled memory budget",
            source.display()
        );
        let mix_err = fvid::native_audio_mix::plan_with_controls(
            &pair,
            &fvid::media_info::MixAudioOptions::default(),
            &tiny_mem,
        )
        .unwrap_err()
        .to_string();
        assert!(
            mix_err.contains("budget") || mix_err.contains("limit"),
            "unexpected mix plan error for {}: {mix_err}",
            source.display()
        );
    }

    // 3. `native_probe::try_probe_as_with_options` and `extract_source_pcm_metadata_with_interval_packet_and_memory_limit`
    // must enforce `max_controlled_bytes`, `max_packet_bytes`, and `max_packets` on WAVE `LIST INFO`, Ogg, FLAC, MP3, and ADTS.
    // Specifically: in `validate_concat_segments_with_options(&[wav_short, raw_flac_48k], &trailing_opts)` where
    // `max_packets: Some(1)` and `max_controlled_bytes: Some(48 << 10)`:
    // - Segment 0 (`wav_short`, 480 frames = 1 packet) consumes `max_packets = 1`, so segment 1 (`raw_flac_48k`)
    //   is skipped by the first loop (`remaining_packets == Some(0)`).
    // - In the second loop, `rem_geom` becomes `Some(0)` for segment 1, so `geom_probe.max_packets` is `None`.
    // - Inside `eligible_with_options(&[raw_flac_48k], &geom_probe)`, `is_single_stream_fvid_media_audio_with_options`
    //   calls `native_probe::try_probe_as_with_options(raw_flac_48k, None, &geom_probe)`!
    // - Previously, `native_probe::try_probe_as_with_options` called `probe_flac` / `probe_mpeg` / `probe_ogg` / `probe_wave`
    //   with default `CopyOptions` (ignoring `geom_probe.max_controlled_bytes`), returning `Ok(...)` before
    //   `plan_decode_audio` converted the budget failure into `Ok(false)` (a generic segment compatibility error),
    //   whereas forwarding `options` in `native_probe::try_probe_as_with_options` propagates the `controlled memory budget exceeded` error!
    let wav_short = dir.0.join("short.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_short, 48_000, 1, &[0.1; 480]).unwrap();
    let raw_flac_48k = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/owned-audio-f32le-matroska/raw-flac-48k-440ms.flac");
    let trailing_opts = CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(48 << 10),
        ..Default::default()
    };
    for trailing in [&ogg_fixture, &raw_flac_48k, &mp3_fixture] {
        let elig_err = fvid::native_audio_mix::eligible_with_options(
            std::slice::from_ref(trailing),
            &trailing_opts,
        )
        .unwrap_err()
        .to_string();
        assert!(
            elig_err.contains("controlled memory budget exceeded"),
            "expected `controlled memory budget exceeded` from eligible_with_options on {}, got: {elig_err}",
            trailing.display()
        );
        let concat_err = fvid::native_audio_mix::validate_concat_segments_with_options(
            &[wav_short.clone(), trailing.clone()],
            &trailing_opts,
        )
        .unwrap_err()
        .to_string();
        assert!(
            concat_err.contains("controlled memory budget exceeded"),
            "expected `controlled memory budget exceeded` from validate_concat_segments_with_options on trailing {}, got: {concat_err}",
            trailing.display()
        );
    }
}

#[test]
fn xfade_three_frame_working_set_and_adts_pce_open_packet_limits_are_enforced_before_allocating() {
    let dir = TempDir::new("xfade-and-adts-pce-budget");

    // 1. `owned_xfade` retains up to 3 decoded frames simultaneously (`primary.current`,
    // `secondary.current`, and `output = current.data.clone()`).
    // For a 4x4 C420 Y4M (frame_len = 24 bytes), 2 * frame_len = 48 <= 60,
    // while 3 * frame_len = 72 > 60.
    // Under `max_controlled_bytes: Some(60)`, `plan_xfade`, `try_xfade_video`, and `xfade_video`
    // must reject with `cross-fade frame exceeds controlled memory budget` before reading frames
    // or creating an output file!
    let main_y4m = dir.0.join("main.y4m");
    let other_y4m = dir.0.join("other.y4m");
    let out_mkv = dir.0.join("out.mkv");
    let mut y4m_bytes = b"YUV4MPEG2 W4 H4 F2:1 Ip A1:1 C420\nFRAME\n".to_vec();
    y4m_bytes.extend_from_slice(&[64u8; 24]);
    std::fs::write(&main_y4m, &y4m_bytes).unwrap();
    std::fs::write(&other_y4m, &y4m_bytes).unwrap();

    let xfade_opts = CopyOptions {
        max_controlled_bytes: Some(60),
        ..Default::default()
    };
    let plan_err = fvid_media::owned_xfade::plan_xfade(
        &main_y4m,
        &other_y4m,
        "fade",
        500_000,
        0,
        &xfade_opts,
    )
    .unwrap_err();
    assert!(
        plan_err.contains("cross-fade frame exceeds controlled memory budget"),
        "expected 3-frame working-set rejection in plan_xfade, got: {plan_err}"
    );
    let try_err = fvid_media::owned_xfade::try_xfade_video(
        &main_y4m,
        &other_y4m,
        &out_mkv,
        "fade",
        500_000,
        0,
        &xfade_opts,
    )
    .unwrap_err();
    assert!(
        try_err.contains("cross-fade frame exceeds controlled memory budget"),
        "expected 3-frame working-set rejection in try_xfade_video, got: {try_err}"
    );
    assert!(!out_mkv.exists());

    // 2. Explicit-layout ADTS (channel_configuration = 0) with a 200-byte first frame
    // containing a truncated PCE: `StreamReader::open` without packet limit allocates the
    // 193-byte payload and fails with `unexpected end of bitstream` while parsing the PCE,
    // whereas opening with `max_packet_bytes: 32` must reject with `exceeds packet byte limit`
    // before allocating the first frame!
    let adts_pce = dir.0.join("explicit_pce.aac");
    let frame_len: usize = 200;
    let mut adts_bytes = vec![0u8; frame_len];
    adts_bytes[0] = 0xFF;
    adts_bytes[1] = 0xF1; // MPEG-4, Layer 0, no CRC
    // profile=1 (AAC-LC -> 01), freq_idx=3 (48kHz -> 0011), private=0, chan_cfg=0 (000)
    // byte 2: 01_0011_0_0 = 0x4C
    adts_bytes[2] = 0x4C;
    // byte 3: chan_cfg_low(00)_orig(0)_home(0)_copy_id(0)_copy_start(0)_frame_len_top2(00)
    adts_bytes[3] = ((frame_len >> 11) & 0x03) as u8;
    adts_bytes[4] = ((frame_len >> 3) & 0xFF) as u8;
    adts_bytes[5] = (((frame_len & 0x07) << 5) | 0x1F) as u8;
    adts_bytes[6] = 0xFC;
    // Payload starts with ID_PCE (0x05 in top 3 bits -> 0b101_00000 = 0xA0)
    adts_bytes[7] = 0xA0;
    std::fs::write(&adts_pce, &adts_bytes).unwrap();

    let adts_pkt_opts = CopyOptions {
        max_packet_bytes: 32,
        ..Default::default()
    };
    let concat_plan_err = fvid_media::owned_concat::plan_concat(
        &[adts_pce.clone(), adts_pce.clone()],
        &adts_pkt_opts,
    )
    .unwrap_err();
    assert!(
        concat_plan_err.contains("ADTS packet exceeds budget")
            || concat_plan_err.contains("packet byte limit"),
        "expected ADTS packet budget error in owned_concat::plan_concat, got: {concat_plan_err}"
    );

    // 3. `native_loudnorm` on ADTS must also enforce `max_controlled_bytes` on the first ADTS frame
    // before PCE bootstrap inspection.
    let adts_mem_opts = CopyOptions {
        max_controlled_bytes: Some(32),
        ..Default::default()
    };
    let loudnorm_err = fvid::native_loudnorm::try_apply(
        &adts_pce,
        &dir.0.join("loudnorm.wav"),
        None,
        false,
        &adts_mem_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        loudnorm_err.contains("ADTS packet exceeds budget"),
        "expected `ADTS packet exceeds budget` from native_loudnorm::try_apply on ADTS, got: {loudnorm_err}"
    );
    let loudnorm_plan_err = fvid::native_loudnorm::try_plan(
        &adts_pce,
        None,
        false,
        &adts_mem_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        loudnorm_plan_err.contains("ADTS packet exceeds budget"),
        "expected `ADTS packet exceeds budget` from native_loudnorm::try_plan on ADTS, got: {loudnorm_plan_err}"
    );
}

#[test]
fn wave_loudnorm_plan_and_concat_segment_validation_preserve_budget_and_cancellation_controls() {
    let dir = TempDir::new("wave-loudnorm-and-concat-cancel");

    // 1. `owned_loudnorm::plan_loudnorm` on a WAVE file with a 512-byte `LIST INFO` tag
    // and small PCM data (960 frames = 3840 bytes) where `max_controlled_bytes` is `Some(65536)`
    // (enough for `metadata_working + size + output`, which is ~12 KiB, so line 282 passes!)
    // must still forward `options` to `owned_probe::probe_wave_with_options`, OR wait:
    // if the WAVE file has a 20 KiB `LIST INFO` `INAM` tag and `max_controlled_bytes` is `Some(16_384)`:
    // Wait: if `INAM` is 1024 bytes, `metadata_working` in `plan_loudnorm` is `8 * (1036 + 12) = 8384` bytes,
    // and with 100 frames (400 bytes), `estimate` in `plan_loudnorm` is `400 + 400 + 8384 = 9184` bytes.
    // Wait: what if `cancel` is triggered or `probe_wave_with_options` is called, AND what if
    // `owned_container_loudness::recognizes_standalone_audio_with_options` is called with a pre-cancelled flag
    // on a G.711 WAVE file with a large preceding `JUNK` chunk?
    // First, test `validate_concat_segments_with_options` with `max_packet_bytes: 16` on `[wav_a, wav_b]`
    // when `max_packets: Some(1)`:
    // Previously, in the second loop of `validate_concat_segments_with_options`, `default_probe` did NOT copy
    // `options.max_packet_bytes`, so for a trailing ADTS segment (`[wav_short, adts_pce]`) with `max_packets: Some(1)`
    // and `max_packet_bytes: 32` (and `max_controlled_bytes: None`), the first loop skipped `adts_pce`
    // because `remaining_packets` became `Some(0)`, AND the second loop checked `eligible_with_options(&[adts_pce], &geom_probe)`
    // with `geom_probe.max_packet_bytes = 64 MiB` (default!) instead of `32`, allowing `validate_concat_segments_with_options`
    // to accept or fail with a generic PCE bitstream error instead of rejecting `adts_pce` in `eligible_with_options`!
    let wav_short = dir.0.join("short.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_short, 48_000, 1, &[0.1; 480]).unwrap();

    // Create a valid 2-channel 48kHz ADTS file with a 120-byte frame (> 32 bytes)
    let adts_large_frame = dir.0.join("large_frame.aac");
    let frame_len: usize = 120;
    let mut adts_bytes = vec![0u8; frame_len];
    adts_bytes[0] = 0xFF;
    adts_bytes[1] = 0xF1;
    // profile=1 (AAC-LC -> 01), freq_idx=3 (48kHz -> 0011), chan_cfg=1 (mono -> 001)
    adts_bytes[2] = 0x4C;
    adts_bytes[3] = 0x40 | (((frame_len >> 11) & 0x03) as u8);
    adts_bytes[4] = ((frame_len >> 3) & 0xFF) as u8;
    adts_bytes[5] = (((frame_len & 0x07) << 5) | 0x1F) as u8;
    adts_bytes[6] = 0xFC;
    // ID_END (0x07 in top 3 bits -> 0xE0)
    adts_bytes[7] = 0xE0;
    std::fs::write(&adts_large_frame, &adts_bytes).unwrap();

    let pkt_limit_opts = CopyOptions {
        max_packets: Some(1),
        max_packet_bytes: 32,
        ..Default::default()
    };
    assert!(
        !fvid::native_audio_mix::concat_eligible_with_options(
            &[wav_short.clone(), adts_large_frame.clone()],
            &pkt_limit_opts,
        )
        .unwrap(),
        "concat_eligible_with_options must reject trailing segment whose frame exceeds max_packet_bytes even when max_packets is exhausted by segment 0"
    );

    // 2. Pre-cancelled `CopyOptions` on a WAVE file with chunks must be honored by
    // `owned_container_loudness::recognizes_standalone_audio_with_options`, `owned_loudnorm::supports_request`,
    // and `owned_wave_loudness::measure_loudness`.
    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    assert!(
        !fvid_media::owned_loudnorm::supports_request(
            &wav_short,
            &dir.0.join("out.wav"),
            None,
            false,
            &cancelled_opts,
        ),
        "supports_request must return false when cancel flag is set"
    );
    let measure_cancel_err =
        fvid_media::measure_loudness(&wav_short, &cancelled_opts).unwrap_err();
    assert!(
        measure_cancel_err.contains("cancelled"),
        "expected cancellation error from measure_loudness, got: {measure_cancel_err}"
    );
}

#[test]
fn native_plan_adts_concat_and_wave_inspection_honor_cancellation_and_packet_budgets() {
    let dir = TempDir::new("native-plan-adts-concat-and-wave-cancel");

    // 1. `native_plan::concat_adts_with_options` must pass `options.max_packet_bytes` / `options.max_controlled_bytes`
    // to `StreamReader::open_with_packet_limit` BEFORE reading the first frame during PCE bootstrap inspection.
    // Construct a channel_configuration == 0 (PCE-in-bitstream) ADTS file with a 120-byte frame.
    let adts_pce = dir.0.join("pce_concat.aac");
    let frame_len: usize = 120;
    let mut adts_bytes = vec![0u8; frame_len];
    adts_bytes[0] = 0xFF;
    adts_bytes[1] = 0xF1;
    // profile=1 (AAC-LC -> 01), freq_idx=3 (48kHz -> 0011), chan_cfg=0 (000)
    adts_bytes[2] = 0x4C;
    adts_bytes[3] = ((frame_len >> 11) & 0x03) as u8;
    adts_bytes[4] = ((frame_len >> 3) & 0xFF) as u8;
    adts_bytes[5] = (((frame_len & 0x07) << 5) | 0x1F) as u8;
    adts_bytes[6] = 0xFC;
    std::fs::write(&adts_pce, &adts_bytes).unwrap();

    let adts_pkt_opts = CopyOptions {
        max_packet_bytes: 32,
        ..Default::default()
    };
    let concat_plan_err = fvid::native_plan::concat_adts_with_options(
        &[adts_pce.clone(), adts_pce.clone()],
        &adts_pkt_opts,
    )
    .unwrap_err();
    assert!(
        concat_plan_err.contains("ADTS packet exceeds budget"),
        "expected `ADTS packet exceeds budget` from native_plan::concat_adts_with_options, got: {concat_plan_err}"
    );

    // 2. `native_plan::decode_audio_with_options`, `native_plan::trim_pcm`, and
    // `native_audio_mix::validate_and_inspect_source_with_options` (via `plan_mix_audio_with_controls` / `plan_merge_audio_with_controls`)
    // on a WAVE file must honor `options.cancel` and `options.max_packet_bytes`.
    let wav = dir.0.join("tone.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav, 48_000, 2, &[0.1; 960]).unwrap();

    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let decode_plan_err = fvid::native_plan::decode_audio_with_options(
        &wav,
        &Default::default(),
        &cancelled_opts,
    )
    .unwrap_err();
    assert!(
        decode_plan_err.contains("cancelled"),
        "expected cancellation error from native_plan::decode_audio_with_options on WAVE, got: {decode_plan_err}"
    );

    let trim_pcm_plan_err = fvid::media::plan_trim_pcm(&wav, 0, 5_000, &cancelled_opts).unwrap_err();
    assert!(
        trim_pcm_plan_err.contains("cancelled"),
        "expected cancellation error from media::plan_trim_pcm on WAVE, got: {trim_pcm_plan_err}"
    );

    // 3. `validate_and_inspect_source_with_options` on a stereo f32le WAVE file (frame_bytes = 8)
    // with `max_packet_bytes: 4` (< 8) must fail with `packet byte limit is smaller than one PCM sample frame`,
    // not with a default-options `native_plan::decode_audio` bypass.
    let tiny_pkt_opts = CopyOptions {
        max_packet_bytes: 4,
        ..Default::default()
    };
    let mix_plan_err = fvid::native_audio_mix::plan_with_controls(
        &[wav.clone(), wav.clone()],
        &Default::default(),
        &tiny_pkt_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        mix_plan_err.contains("packet byte limit is smaller than one PCM sample frame"),
        "expected `packet byte limit is smaller than one PCM sample frame` from native_audio_mix::plan_with_controls, got: {mix_plan_err}"
    );
}

#[test]
fn native_export_adts_concat_audio_trim_and_wave_concat_honor_packet_and_cancellation_controls() {
    let dir = TempDir::new("native-export-adts-concat-and-wave-cancel");

    // 1. `native_export::concat_adts_aac_with_options` must pass `options.max_packet_bytes` / `options.max_controlled_bytes`
    // to `StreamReader::open_with_packet_limit` BEFORE reading the first frame during PCE bootstrap inspection.
    let adts_pce = dir.0.join("pce_exec.aac");
    let frame_len: usize = 120;
    let mut adts_bytes = vec![0u8; frame_len];
    adts_bytes[0] = 0xFF;
    adts_bytes[1] = 0xF1;
    // profile=1 (AAC-LC -> 01), freq_idx=3 (48kHz -> 0011), chan_cfg=0 (000)
    adts_bytes[2] = 0x4C;
    adts_bytes[3] = ((frame_len >> 11) & 0x03) as u8;
    adts_bytes[4] = ((frame_len >> 3) & 0xFF) as u8;
    adts_bytes[5] = (((frame_len & 0x07) << 5) | 0x1F) as u8;
    adts_bytes[6] = 0xFC;
    std::fs::write(&adts_pce, &adts_bytes).unwrap();

    let adts_pkt_opts = CopyOptions {
        max_packet_bytes: 32,
        ..Default::default()
    };
    let concat_exec_err = fvid::native_export::concat_adts_aac_with_options(
        &[adts_pce.clone(), adts_pce.clone()],
        &dir.0.join("joined.mka"),
        &adts_pkt_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        concat_exec_err.contains("ADTS packet exceeds budget"),
        "expected `ADTS packet exceeds budget` from native_export::concat_adts_aac_with_options, got: {concat_exec_err}"
    );

    // 2. `media::plan_concat` on WAVE files with a pre-cancelled flag must fail immediately
    // with `cancelled` rather than scanning all WAVE chunks via `concat_info(sources)`.
    let wav = dir.0.join("tone.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav, 48_000, 2, &[0.1; 960]).unwrap();

    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let concat_wave_plan_err =
        fvid::media::plan_concat(&[wav.clone(), wav.clone()], &cancelled_opts).unwrap_err();
    assert!(
        concat_wave_plan_err.contains("cancelled"),
        "expected cancellation error from media::plan_concat on WAVE, got: {concat_wave_plan_err}"
    );
}

#[test]
fn owned_audio_mix_and_merge_with_controls_and_native_loudnorm_fallback_enforce_budgets_and_cancellation() {
    let dir = TempDir::new("owned-audio-mix-controls-and-loudnorm-fallback");
    let wav_a = dir.0.join("a.wav");
    let wav_b = dir.0.join("b.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_a, 48_000, 2, &[0.25; 960]).unwrap();
    fvid_media::owned_wav_file::write_wav_f32le(&wav_b, 48_000, 2, &[0.5; 960]).unwrap();
    let pair = [wav_a.clone(), wav_b.clone()];

    // 1. Pre-cancelled `CopyOptions` on `fvid::media::{mix_audio_with_controls, plan_mix_audio_with_controls,
    // merge_audio_with_controls, plan_merge_audio_with_controls}` and `fvid_media::owned_audio_mix`
    // must fail with `"media operation cancelled"`, even when fallback to `fvid_media` is taken.
    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    for err in [
        fvid::media::plan_mix_audio_with_controls(&pair, &Default::default(), &cancelled_opts).unwrap_err(),
        fvid::media::mix_audio_with_controls(&pair, &dir.0.join("mix-cancel.wav"), &Default::default(), &cancelled_opts).unwrap_err(),
        fvid::media::plan_merge_audio_with_controls(&pair, &cancelled_opts).unwrap_err(),
        fvid::media::merge_audio_with_controls(&pair, &dir.0.join("merge-cancel.wav"), &cancelled_opts).unwrap_err(),
    ] {
        assert!(
            err.contains("cancelled"),
            "expected cancellation error from mix/merge with_controls, got: {err}"
        );
    }

    // 2. `fvid_media::owned_audio_mix::{mix_audio_with_controls, plan_mix_audio_with_controls,
    // merge_audio_with_controls, plan_merge_audio_with_controls}` must enforce `max_packet_bytes`,
    // `max_packets`, and `max_controlled_bytes` during both planning and execution!
    let tiny_pkt = CopyOptions {
        max_packet_bytes: 4, // smaller than 1 stereo f32 frame (8 bytes)
        ..Default::default()
    };
    let tiny_mem = CopyOptions {
        max_controlled_bytes: Some(512), // smaller than 960 * 4 = 3840 bytes per input
        ..Default::default()
    };
    let one_pkt_limit = CopyOptions {
        max_packet_bytes: 160, // 20 stereo frames (160 bytes) per block
        max_packets: Some(2),  // 2 blocks -> 40 frames instead of 480 frames
        ..Default::default()
    };
    for opts in [&tiny_pkt, &tiny_mem] {
        assert!(
            fvid_media::owned_audio_mix::plan_mix_audio_with_controls(&pair, &Default::default(), opts).is_err(),
            "plan_mix_audio_with_controls must fail for {opts:?}"
        );
        assert!(
            fvid_media::owned_audio_mix::mix_audio_with_controls(&pair, &dir.0.join("mix-fail.wav"), &Default::default(), opts).is_err(),
            "mix_audio_with_controls must fail for {opts:?}"
        );
        assert!(
            fvid_media::owned_audio_mix::plan_merge_audio_with_controls(&pair, opts).is_err(),
            "plan_merge_audio_with_controls must fail for {opts:?}"
        );
        assert!(
            fvid_media::owned_audio_mix::merge_audio_with_controls(&pair, &dir.0.join("merge-fail.wav"), opts).is_err(),
            "merge_audio_with_controls must fail for {opts:?}"
        );
    }
    let mix_out = dir.0.join("mix-limited.wav");
    let mix_stats = fvid_media::owned_audio_mix::mix_audio_with_controls(
        &pair,
        &mix_out,
        &Default::default(),
        &one_pkt_limit,
    )
    .unwrap();
    assert_eq!(mix_stats.sample_frames, 40);
    let merge_out = dir.0.join("merge-limited.wav");
    let merge_stats = fvid_media::owned_audio_mix::merge_audio_with_controls(
        &pair,
        &merge_out,
        &one_pkt_limit,
    )
    .unwrap();
    assert_eq!(merge_stats.sample_frames, 40);

    // 3. In `src/native_loudnorm.rs`, when `try_plan` falls back to
    // `owned_container_loudnorm::supports_plan` because `native_stream` is `None` (e.g. a Matroska
    // audio container produced by `decode_audio`, which is not an MP4/WebM `is_owned_audio_trim_source`),
    // `default_opts` passed to `supports_plan` must preserve `options.cancel`,
    // `options.max_packet_bytes`, `options.max_packets`, and `options.max_controlled_bytes`.
    let mkv_audio = dir.0.join("small-audio.mka");
    fvid::media::decode_audio(&source("small.m4a"), &mkv_audio, &CopyOptions::default()).unwrap();
    let tiny_loudnorm_pkt = CopyOptions {
        max_packet_bytes: 2,
        ..Default::default()
    };
    let loudnorm_plan_err =
        fvid::media::plan_loudnorm(&mkv_audio, None, false, &tiny_loudnorm_pkt).unwrap_err();
    assert!(
        loudnorm_plan_err.contains("packet")
            || loudnorm_plan_err.contains("budget")
            || loudnorm_plan_err.contains("limit"),
        "expected packet budget error from plan_loudnorm on Matroska audio with max_packet_bytes=2, got: {loudnorm_plan_err}"
    );
}

#[test]
fn adts_source_info_and_native_plan_wave_trim_concat_and_remux_enforce_budgets_and_cancellation() {
    let dir = TempDir::new("adts-info-and-native-plan-budgets");

    // 1. Create an explicit-PCE ADTS file (`channel_configuration == 0`) with a 120-byte frame.
    let adts_pce = dir.0.join("pce.aac");
    let frame_len = 120usize;
    let mut adts_bytes = vec![0u8; frame_len];
    adts_bytes[0] = 0xFF;
    adts_bytes[1] = 0xF1;
    adts_bytes[2] = 0x4C;
    adts_bytes[3] = ((frame_len >> 11) & 0x03) as u8;
    adts_bytes[4] = ((frame_len >> 3) & 0xFF) as u8;
    adts_bytes[5] = (((frame_len & 0x07) << 5) | 0x1F) as u8;
    adts_bytes[6] = 0xFC;
    adts_bytes[7] = 0xA0; // ID_PCE
    std::fs::write(&adts_pce, &adts_bytes).unwrap();

    let adts_mem_opts = CopyOptions {
        max_controlled_bytes: Some(32),
        ..Default::default()
    };
    let info_err = fvid::native_media::aac_source_info_selected_with_options(
        &adts_pce,
        None,
        None,
        &adts_mem_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        info_err.contains("ADTS packet exceeds budget"),
        "expected `ADTS packet exceeds budget` from aac_source_info_selected_with_options, got: {info_err}"
    );
    let inspect_err = fvid::native_media::inspect_adts_with_options(
        std::fs::File::open(&adts_pce).unwrap(),
        &adts_mem_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        inspect_err.contains("ADTS packet exceeds budget"),
        "expected `ADTS packet exceeds budget` from inspect_adts_with_options, got: {inspect_err}"
    );

    let flag = fvid::media_control::CancelFlag::new();
    flag.cancel();
    let cancelled = CopyOptions {
        cancel: Some(flag),
        ..Default::default()
    };
    assert!(
        fvid::native_media::inspect_adts_with_options(
            std::fs::File::open(&adts_pce).unwrap(),
            &cancelled,
        )
        .unwrap_err()
        .to_string()
        .contains("cancelled")
    );

    // 2. `native_plan::trim_pcm_with_options` and `native_plan::concat_wave_with_options`
    // must enforce `max_packet_bytes` (< 1 PCM frame), `max_packets: Some(0)`, and `max_rss_bytes: Some(1)`
    // just like `fvid_media::plan_trim_pcm` and `fvid_media::plan_concat`.
    let wav_a = dir.0.join("a.wav");
    let wav_b = dir.0.join("b.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_a, 48_000, 2, &[0.1; 960]).unwrap();
    fvid_media::owned_wav_file::write_wav_f32le(&wav_b, 48_000, 2, &[0.2; 960]).unwrap();

    let tiny_pkt = CopyOptions {
        max_packet_bytes: 4, // smaller than 1 stereo f32le frame (8 bytes)
        ..Default::default()
    };
    let zero_pkts = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let tiny_rss = CopyOptions {
        max_rss_bytes: Some(1),
        ..Default::default()
    };
    for opts in [&tiny_pkt, &zero_pkts, &tiny_rss] {
        assert!(
            fvid::native_plan::trim_pcm_with_options(&wav_a, 0, 10_000, opts).is_err(),
            "native_plan::trim_pcm_with_options must reject {opts:?}"
        );
        assert!(
            fvid::native_plan::concat_wave_with_options(&[wav_a.clone(), wav_b.clone()], opts)
                .is_err(),
            "native_plan::concat_wave_with_options must reject {opts:?}"
        );
    }

    // 3. `native_plan::remux_with_options` on a Matroska file must enforce `max_packets == Some(0)`
    // (`packet work limit requires at least one packet`), and on a multi-packet Matroska file
    // must enforce `max_packets: Some(1)`.
    let adts_two = dir.0.join("two.aac");
    let valid_frame_len = 16usize;
    let mut one_adts_frame = vec![0u8; valid_frame_len];
    one_adts_frame[0] = 0xFF;
    one_adts_frame[1] = 0xF1;
    one_adts_frame[2] = 0x4C;
    one_adts_frame[3] = 0x40 | (((valid_frame_len >> 11) & 0x03) as u8);
    one_adts_frame[4] = ((valid_frame_len >> 3) & 0xFF) as u8;
    one_adts_frame[5] = (((valid_frame_len & 0x07) << 5) | 0x1F) as u8;
    one_adts_frame[6] = 0xFC;
    one_adts_frame[7] = 0xE0; // ID_END
    let mut two_adts_frames = one_adts_frame.clone();
    two_adts_frames.extend_from_slice(&one_adts_frame);
    std::fs::write(&adts_two, &two_adts_frames).unwrap();

    let mkv_audio = dir.0.join("multi.mka");
    fvid::media::remux(&adts_two, &mkv_audio, &CopyOptions::default()).unwrap();
    let zero_pkt_err =
        fvid::native_plan::remux_with_options(&mkv_audio, &zero_pkts).unwrap_err();
    assert!(
        zero_pkt_err.contains("packet work limit"),
        "expected `packet work limit` from native_plan::remux_with_options with max_packets=Some(0), got: {zero_pkt_err}"
    );
    let one_pkt = CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    let remux_plan_err = fvid::native_plan::remux_with_options(&mkv_audio, &one_pkt).unwrap_err();
    assert!(
        remux_plan_err.contains("packet count exceeds limit")
            || remux_plan_err.contains("packet work limit"),
        "expected packet limit error from native_plan::remux_with_options on Matroska, got: {remux_plan_err}"
    );
}

#[test]
fn wave_metadata_matroska_scan_errors_and_audio_reader_rss_cancellation_budgets_are_enforced() {
    let dir = TempDir(
        std::env::temp_dir().join(format!("fvid-batch-351-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();

    // 1. Build a valid Matroska PCM file (`pcm-int.mkv`), change its Segment size field to unknown
    // (`0x01FFFFFFFFFFFFFF`), and append a truncated trailing EBML Tags element
    // (ID `0x1254C367`, size `0x88` = 8 bytes, but only 2 bytes of payload) after the valid Clusters.
    // Because `owned_matroska_pcm_export::geometry_for_decode` does not call `scan_all()` when
    // `max_controlled_bytes` is `None` and `max_packet_bytes` is default, `owned_matroska_pcm_export::apply`
    // reaches `owned_wave_metadata::matroska_source_info_with_interval`, which must propagate the
    // `scan_all()` error instead of silently ignoring `let _ = reader.scan_all();`!
    let valid_flac_mka = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/pcm-int.mkv");
    let mut corrupt_tags_bytes = std::fs::read(&valid_flac_mka).unwrap();
    let seg_pos = corrupt_tags_bytes
        .windows(4)
        .position(|w| w == [0x18, 0x53, 0x80, 0x67])
        .unwrap();
    // Read EBML VINT width of Segment size and replace with unknown size of the same width
    let vint_first = corrupt_tags_bytes[seg_pos + 4];
    let vint_len = vint_first.leading_zeros() as usize + 1;
    corrupt_tags_bytes[seg_pos + 4] = (1u8 << (8 - vint_len)) | ((1u8 << (8 - vint_len)) - 1);
    for b in &mut corrupt_tags_bytes[seg_pos + 5..seg_pos + 4 + vint_len] {
        *b = 0xFF;
    }
    corrupt_tags_bytes.extend_from_slice(&[0x12, 0x54, 0xC3, 0x67, 0x88, 0x00, 0x00]);
    let corrupt_mka = dir.0.join("corrupt-tags.mka");
    std::fs::write(&corrupt_mka, &corrupt_tags_bytes).unwrap();
    let out_wav = dir.0.join("out.wav");
    let decode_corrupt_err = fvid_media::decode_audio(&corrupt_mka, &out_wav, &CopyOptions::default())
        .unwrap_err();
    assert!(
        !decode_corrupt_err.is_empty(),
        "expected decode_audio to fail when trailing Matroska Tags element is truncated"
    );

    // 2. `plan_decode_audio` on MP4 and Matroska (which open the container via
    // `owned_mp4_audio_export::open` and `owned_matroska_audio::open_audio_reader`)
    // must honor pre-cancelled `CancelFlag` and `max_rss_bytes: Some(1)` before parsing container headers.
    let flag = fvid::media_control::CancelFlag::new();
    flag.cancel();
    let cancelled = CopyOptions {
        cancel: Some(flag),
        ..Default::default()
    };
    let tiny_rss = CopyOptions {
        max_rss_bytes: Some(1),
        ..Default::default()
    };
    for opts in [&cancelled, &tiny_rss] {
        assert!(
            fvid_media::plan_decode_audio(&source("small.m4a"), &Default::default(), opts).is_err(),
            "plan_decode_audio on MP4 must reject {opts:?}"
        );
        assert!(
            fvid_media::plan_decode_audio(&valid_flac_mka, &Default::default(), opts).is_err(),
            "plan_decode_audio on Matroska must reject {opts:?}"
        );
    }

    // 3. `owned_raw_flac::decode_raw_flac_pcm` must enforce `max_controlled_bytes` against
    // the per-frame decoded float PCM buffer (`decoded.samples.len() * 4`) in addition to
    // the file/index/decoder admission estimate.
    let flac_fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-index-growth/frames-129.flac");
    let flac_len = std::fs::metadata(&flac_fixture).unwrap().len() as usize;
    // Allow file_size + 32 MiB + 129 * sizeof(Frame) (129 * 24 = 3096 bytes) so `open_with_interval`
    // succeeds, but leave 0 bytes for the decoded frame's `pcm: Vec<f32>` buffer!
    let flac_index_only_budget = flac_len + 32 * 1024 * 1024 + 130 * 24;
    let flac_tight_opts = CopyOptions {
        max_controlled_bytes: Some(flac_index_only_budget),
        ..Default::default()
    };
    let mut sink = Vec::new();
    let flac_err = fvid_media::owned_raw_flac::decode_raw_flac_pcm(
        &flac_fixture,
        &mut sink,
        None,
        &flac_tight_opts,
    )
    .unwrap_err();
    assert!(
        flac_err.contains("controlled memory budget exceeded"),
        "expected `controlled memory budget exceeded` from decode_raw_flac_pcm when decoded PCM buffer exceeds budget, got: {flac_err}"
    );
}

#[test]
fn matroska_remux_zero_packets_and_native_export_wave_metadata_budgets_are_enforced() {
    let dir = TempDir(
        std::env::temp_dir().join(format!("fvid-batch-352-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();

    // 1. `fvid_media::plan_remux` and `fvid_media::remux` on a Matroska source with
    // `max_packets: Some(0)` must reject with `"packet work limit requires at least one packet"`
    // rather than `"request is not an owned Matroska identity remux"`.
    let mka_source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/pcm-int.mkv");
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let plan_err = fvid_media::plan_remux(&mka_source, &zero_packets).unwrap_err();
    assert!(
        plan_err.contains("packet work limit requires at least one packet"),
        "expected `packet work limit requires at least one packet` from plan_remux on Matroska with max_packets: Some(0), got: {plan_err}"
    );
    let out_mka = dir.0.join("out.mka");
    let remux_err = fvid_media::remux(&mka_source, &out_mka, &zero_packets).unwrap_err();
    assert!(
        remux_err.contains("packet work limit requires at least one packet"),
        "expected `packet work limit requires at least one packet` from remux on Matroska with max_packets: Some(0), got: {remux_err}"
    );

    // 2. `fvid::native_plan::remux_with_options` on a multi-packet ADTS source must bound
    // its packet scan by `options.max_packets` so `max_controlled_bytes` is only charged for
    // retained packets (and `max_packets: Some(0)` is rejected).
    let adts_path = dir.0.join("multi.aac");
    let valid_frame_len = 16usize;
    let mut single_adts = vec![0u8; valid_frame_len];
    single_adts[0] = 0xFF;
    single_adts[1] = 0xF1;
    single_adts[2] = 0x4C;
    single_adts[3] = 0x40 | (((valid_frame_len >> 11) & 0x03) as u8);
    single_adts[4] = ((valid_frame_len >> 3) & 0xFF) as u8;
    single_adts[5] = (((valid_frame_len & 0x07) << 5) | 0x1F) as u8;
    single_adts[6] = 0xFC;
    single_adts[7] = 0xE0; // ID_END
    let mut multi_adts = Vec::with_capacity(single_adts.len() * 10);
    for _ in 0..10 {
        multi_adts.extend_from_slice(&single_adts);
    }
    std::fs::write(&adts_path, &multi_adts).unwrap();
    let adts_zero_err = fvid::native_plan::remux_with_options(&adts_path, &zero_packets).unwrap_err();
    assert!(
        adts_zero_err.contains("packet work limit requires at least one packet"),
        "expected `packet work limit requires at least one packet` from native_plan::remux_with_options on ADTS with max_packets: Some(0), got: {adts_zero_err}"
    );
    // The retained cue and reusable encoded payload coexist with the base
    // reserve. Keep an exact one-packet budget and prove both its lower bound
    // and the refusal when the selected prefix grows to two packets.
    let one_packet_bytes = 16384 + 32 + valid_frame_len - 7;
    let one_packet_budget = CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(one_packet_bytes),
        ..Default::default()
    };
    for options in [
        CopyOptions {
            max_controlled_bytes: Some(one_packet_bytes - 1),
            ..one_packet_budget.clone()
        },
        CopyOptions {
            max_packets: Some(2),
            ..one_packet_budget.clone()
        },
    ] {
        let error = fvid::native_plan::remux_with_options(&adts_path, &options).unwrap_err();
        assert_eq!(error, "controlled memory budget exceeded");
    }
    let plan = fvid::native_plan::remux_with_options(&adts_path, &one_packet_budget)
        .expect("native_plan::remux_with_options on ADTS with max_packets: Some(1) should stop scanning after 1 packet")
        .expect("ADTS source should be recognized");
    assert_eq!(plan.command, "remux");

    // 3. `fvid::native_export::export_audio_pcm_selected_with_controls` to `.wav` with `metadata_set`
    // exceeding `max_controlled_bytes` must reject with `"controlled memory budget exceeded"` before writing output.
    let wav_source = dir.0.join("src.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_source, 48_000, 2, &[0.1; 960]).unwrap();
    let out_wav = dir.0.join("meta-budget.wav");
    let tight_meta_opts = CopyOptions {
        metadata_set: vec![("title".to_string(), "T".repeat(4096))],
        max_controlled_bytes: Some(1024),
        ..Default::default()
    };
    let export_err = fvid::native_export::export_audio_pcm_selected_with_controls(
        &wav_source,
        &out_wav,
        None,
        &tight_meta_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        export_err.contains("controlled memory budget exceeded"),
        "expected `controlled memory budget exceeded` from native_export::export_audio_pcm_selected_with_controls when WAVE LIST INFO metadata exceeds max_controlled_bytes, got: {export_err}"
    );
    assert!(!out_wav.exists());
}

#[test]
fn raw_mp3_flac_ogg_source_info_and_decoded_pcm_budgets_are_enforced() {
    let dir = TempDir(
        std::env::temp_dir().join(format!("fvid-batch-353-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();

    // 1. `fvid_media::owned_raw_mp3::decode_raw_mpeg_pcm` must charge the per-frame decoded float PCM
    // buffer (`pcm.len() * 4`) against `options.max_controlled_bytes` in addition to the file/index/decoder
    // admission estimate.
    let mp3_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp3-id3-metadata/tagged-id3v24.mp3");
    let mp3_len = std::fs::metadata(&mp3_fixture).unwrap().len() as usize;
    // Allow file_size + decode_admission_bytes (32 KiB) + index_bytes (4 * 2 * (16 + 8 + 32) = 448 bytes)
    // so `open_with_interval` succeeds, but leave < 4608 bytes so the decoded frame's `pcm: Vec<f32>`
    // (1152 samples * 4 bytes = 4608 bytes) exceeds `max_controlled_bytes`!
    let mp3_index_only_budget = mp3_len + 32 * 1024 + 512;
    let mp3_tight_opts = CopyOptions {
        max_controlled_bytes: Some(mp3_index_only_budget),
        ..Default::default()
    };
    let mut sink = Vec::new();
    let mp3_err = fvid_media::owned_raw_mp3::decode_raw_mpeg_pcm(
        &mp3_fixture,
        &mut sink,
        None,
        &mp3_tight_opts,
    )
    .unwrap_err();
    assert!(
        mp3_err.contains("controlled memory budget exceeded"),
        "expected `controlled memory budget exceeded` from decode_raw_mpeg_pcm when decoded PCM buffer exceeds budget, got: {mp3_err}"
    );

    // 2. `probe_mpeg_with_options` and `probe_ogg_with_options` must reject `max_controlled_bytes: Some(0)`
    // with `"controlled memory limit must be positive"`, and `probe_mpeg_with_options` must reject
    // `max_packet_bytes: 0` with `"packet byte limit must be positive"` and `max_packets: Some(0)`
    // with `"packet work limit requires at least one packet"`.
    let ogg_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-g711-loudnorm-budget/ogg-opus-mono-4byte-pkt.ogg");
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_pkt_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_pkts = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_mpeg_probe::probe_mpeg_with_options(&mp3_fixture, &zero_mem)
            .unwrap_err()
            .contains("controlled memory limit must be positive"),
    );
    assert!(
        fvid_media::owned_mpeg_probe::probe_mpeg_with_options(&mp3_fixture, &zero_pkt_bytes)
            .unwrap_err()
            .contains("packet byte limit must be positive"),
    );
    assert!(
        fvid_media::owned_mpeg_probe::probe_mpeg_with_options(&mp3_fixture, &zero_pkts)
            .unwrap_err()
            .contains("packet work limit requires at least one packet"),
    );
    assert!(
        fvid_media::owned_probe::probe_ogg_with_options(&ogg_fixture, &zero_mem)
            .unwrap_err()
            .contains("controlled memory limit must be positive"),
    );
}

#[test]
fn probe_flac_adts_wave_webm_mp4_and_y4m_enforce_zero_limits_and_rss_budgets() {
    let dir = TempDir(
        std::env::temp_dir().join(format!("fvid-batch-354-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();

    let flac_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-streaminfo/valid_baseline.flac");
    let adts_fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-mono-44k.aac");
    let mka_fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/pcm-int.mkv");
    let mp4_fixture = source("small.m4a");
    let wav_path = dir.0.join("probe.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_path, 48_000, 2, &[0.1; 960]).unwrap();
    let y4m_path = dir.0.join("probe.y4m");
    let mut y4m_bytes = b"YUV4MPEG2 W4 H4 F2:1 Ip A1:1 C420\nFRAME\n".to_vec();
    y4m_bytes.extend_from_slice(&[64u8; 24]);
    std::fs::write(&y4m_path, &y4m_bytes).unwrap();

    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_pkt_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_pkts = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let tiny_rss = CopyOptions {
        max_rss_bytes: Some(1),
        ..Default::default()
    };

    // 1. `probe_flac_with_options`, `probe_adts_with_options`, `probe_wave_with_options`,
    // `probe_webm_with_options`, `probe_y4m_with_options`, and `fvid::media::probe_with_options`
    // must reject `max_controlled_bytes: Some(0)`, `max_packet_bytes: 0`, `max_packets: Some(0)`,
    // and `max_rss_bytes: Some(1)`.
    for path in [
        &flac_fixture,
        &adts_fixture,
        &mka_fixture,
        &mp4_fixture,
        &wav_path,
        &y4m_path,
    ] {
        let mem_err = fvid_media::owned_probe::probe_with_options(path, &zero_mem).unwrap_err();
        assert!(
            mem_err.contains("controlled memory limit must be positive"),
            "expected `controlled memory limit must be positive` from probe_with_options on {}, got: {mem_err}",
            path.display()
        );
        let pkt_bytes_err =
            fvid_media::owned_probe::probe_with_options(path, &zero_pkt_bytes).unwrap_err();
        assert!(
            pkt_bytes_err.contains("packet byte limit must be positive"),
            "expected `packet byte limit must be positive` from probe_with_options on {}, got: {pkt_bytes_err}",
            path.display()
        );
        let pkts_err = fvid_media::owned_probe::probe_with_options(path, &zero_pkts).unwrap_err();
        assert!(
            pkts_err.contains("packet work limit requires at least one packet"),
            "expected `packet work limit requires at least one packet` from probe_with_options on {}, got: {pkts_err}",
            path.display()
        );
        let rss_err = fvid_media::owned_probe::probe_with_options(path, &tiny_rss).unwrap_err();
        assert!(
            rss_err.to_ascii_lowercase().contains("rss"),
            "expected RSS budget error from probe_with_options on {}, got: {rss_err}",
            path.display()
        );
    }

    // 2. Direct calls to `probe_wave_with_options`, `probe_adts_with_options`, `probe_flac_with_options`,
    // `probe_webm_with_options`, and `probe_y4m_with_options` also enforce those invariants.
    assert!(
        fvid_media::owned_probe::probe_wave_with_options(&wav_path, &tiny_rss)
            .unwrap_err()
            .to_string()
            .to_ascii_lowercase()
            .contains("rss"),
    );
    assert!(
        fvid_media::owned_probe::probe_wave_with_options(&wav_path, & CopyOptions {
            max_packet_bytes: 4, // smaller than 1 stereo f32le frame (8 bytes)
            ..Default::default()
        })
        .is_err(),
        "probe_wave_with_options must reject max_packet_bytes smaller than 1 PCM sample frame"
    );
    assert!(
        fvid_media::owned_probe::probe_adts_with_options(&adts_fixture, &zero_pkts)
            .unwrap_err()
            .contains("packet work limit requires at least one packet"),
    );
    assert!(
        fvid_media::owned_flac_probe::probe_flac_with_options(&flac_fixture, &zero_mem)
            .unwrap_err()
            .contains("controlled memory limit must be positive"),
    );
    assert!(
        fvid_media::owned_webm_probe::probe_webm_with_options(&mka_fixture, &zero_mem)
            .unwrap_err()
            .contains("controlled memory limit must be positive"),
    );
    assert!(
        fvid_media::owned_y4m_probe::probe_y4m_with_options(&y4m_path, &zero_mem)
            .unwrap_err()
            .contains("controlled memory limit must be positive"),
    );
}

#[test]
fn wave_adts_and_mp4_remux_trim_and_concat_enforce_zero_limits_and_metadata_budgets() {
    let dir = TempDir(
        std::env::temp_dir().join(format!("fvid-batch-355-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();

    let wav_a = dir.0.join("a.wav");
    let wav_b = dir.0.join("b.wav");
    let wav_out = dir.0.join("out.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_a, 48_000, 2, &[0.1; 960]).unwrap();
    fvid_media::owned_wav_file::write_wav_f32le(&wav_b, 48_000, 2, &[0.2; 960]).unwrap();
    let adts_fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-mono-44k.aac");
    let mp4_fixture = source("small.m4a");
    let mka_out = dir.0.join("out.mka");
    let mkv_out = dir.0.join("out.mkv");

    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_pkt_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_pkts = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };

    // 1. `owned_wave_remux::{remux, trim, trim_pcm, concat}` and `owned_wave_plan::{plan_remux,
    // plan_trim, plan_trim_pcm, plan_concat}` must reject `max_packet_bytes: 0` with
    // `"packet byte limit must be positive"`, `max_packets: Some(0)` with
    // `"packet work limit requires at least one packet"`, and `max_controlled_bytes: Some(0)`
    // with `"controlled memory limit must be positive"`.
    for (opts, expected) in [
        (&zero_pkt_bytes, "packet byte limit must be positive"),
        (&zero_pkts, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
    ] {
        let remux_err = fvid_media::remux(&wav_a, &wav_out, opts).unwrap_err();
        assert!(
            remux_err.contains(expected),
            "expected `{expected}` from remux on WAV with {opts:?}, got: {remux_err}"
        );
        let plan_remux_err = fvid_media::plan_remux(&wav_a, opts).unwrap_err();
        assert!(
            plan_remux_err.contains(expected),
            "expected `{expected}` from plan_remux on WAV with {opts:?}, got: {plan_remux_err}"
        );
        let trim_err = fvid_media::trim(&wav_a, &wav_out, 0, 5_000, opts).unwrap_err();
        assert!(
            trim_err.contains(expected),
            "expected `{expected}` from trim on WAV with {opts:?}, got: {trim_err}"
        );
        let plan_trim_err = fvid_media::plan_trim(&wav_a, 0, 5_000, opts).unwrap_err();
        assert!(
            plan_trim_err.contains(expected),
            "expected `{expected}` from plan_trim on WAV with {opts:?}, got: {plan_trim_err}"
        );
        let trim_pcm_err = fvid_media::trim_pcm(&wav_a, &wav_out, 0, 5_000, opts).unwrap_err();
        assert!(
            trim_pcm_err.contains(expected),
            "expected `{expected}` from trim_pcm on WAV with {opts:?}, got: {trim_pcm_err}"
        );
        let plan_trim_pcm_err = fvid_media::plan_trim_pcm(&wav_a, 0, 5_000, opts).unwrap_err();
        assert!(
            plan_trim_pcm_err.contains(expected),
            "expected `{expected}` from plan_trim_pcm on WAV with {opts:?}, got: {plan_trim_pcm_err}"
        );
        let concat_err =
            fvid_media::concat(&[wav_a.clone(), wav_b.clone()], &wav_out, opts).unwrap_err();
        assert!(
            concat_err.contains(expected),
            "expected `{expected}` from concat on WAV with {opts:?}, got: {concat_err}"
        );
        let plan_concat_err =
            fvid_media::plan_concat(&[wav_a.clone(), wav_b.clone()], opts).unwrap_err();
        assert!(
            plan_concat_err.contains(expected),
            "expected `{expected}` from plan_concat on WAV with {opts:?}, got: {plan_concat_err}"
        );
    }

    // 2. Direct calls to `owned_adts_remux::{remux, plan_remux}` and `owned_mp4_remux::{remux, plan_remux}`
    // must also reject `max_packet_bytes: 0`, `max_packets: Some(0)`, and `max_controlled_bytes: Some(0)`
    // with the specific limit diagnostic rather than `"request is not an owned ... remux"`.
    for (opts, expected) in [
        (&zero_pkt_bytes, "packet byte limit must be positive"),
        (&zero_pkts, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
    ] {
        let adts_remux_err =
            fvid_media::owned_adts_remux::remux(&adts_fixture, &mka_out, opts).unwrap_err();
        assert!(
            adts_remux_err.contains(expected),
            "expected `{expected}` from owned_adts_remux::remux with {opts:?}, got: {adts_remux_err}"
        );
        let adts_plan_err =
            fvid_media::owned_adts_remux::plan_remux(&adts_fixture, opts).unwrap_err();
        assert!(
            adts_plan_err.contains(expected),
            "expected `{expected}` from owned_adts_remux::plan_remux with {opts:?}, got: {adts_plan_err}"
        );
        let mp4_remux_err =
            fvid_media::owned_mp4_remux::remux(&mp4_fixture, &mkv_out, opts).unwrap_err();
        assert!(
            mp4_remux_err.contains(expected),
            "expected `{expected}` from owned_mp4_remux::remux with {opts:?}, got: {mp4_remux_err}"
        );
        let mp4_plan_err =
            fvid_media::owned_mp4_remux::plan_remux(&mp4_fixture, opts).unwrap_err();
        assert!(
            mp4_plan_err.contains(expected),
            "expected `{expected}` from owned_mp4_remux::plan_remux with {opts:?}, got: {mp4_plan_err}"
        );
    }
}

#[test]
fn loudness_loudnorm_and_audio_mix_merge_concat_enforce_zero_limits_and_metadata_budgets() {
    let dir = TempDir(
        std::env::temp_dir().join(format!("fvid-batch-356-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();

    let wav_a = dir.0.join("a.wav");
    let wav_b = dir.0.join("b.wav");
    let wav_out = dir.0.join("out.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_a, 48_000, 2, &[0.1; 960]).unwrap();
    fvid_media::owned_wav_file::write_wav_f32le(&wav_b, 48_000, 2, &[0.2; 960]).unwrap();
    let adts_fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-mono-44k.aac");
    let mp4_fixture = source("small.m4a");

    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_pkt_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_pkts = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };

    // 1. `measure_loudness`, `plan_loudness`, `apply_loudnorm`, `apply_loudnorm_dual`,
    // `plan_loudnorm`, `native_loudnorm::try_plan`, and `native_loudnorm::try_apply`
    // across WAVE, ADTS, and MP4 must reject `max_packet_bytes: 0` with
    // `"packet byte limit must be positive"`, `max_packets: Some(0)` with
    // `"packet work limit requires at least one packet"`, and `max_controlled_bytes: Some(0)`
    // with `"controlled memory limit must be positive"`.
    for path in [&wav_a, &adts_fixture, &mp4_fixture] {
        for (opts, expected) in [
            (&zero_pkt_bytes, "packet byte limit must be positive"),
            (&zero_pkts, "packet work limit requires at least one packet"),
            (&zero_mem, "controlled memory limit must be positive"),
        ] {
            let loud_err = fvid_media::measure_loudness(path, opts).unwrap_err();
            assert!(
                loud_err.contains(expected),
                "expected `{expected}` from measure_loudness on {} with {opts:?}, got: {loud_err}",
                path.display()
            );
            let plan_loud_err = fvid_media::plan_loudness(path, opts).unwrap_err();
            assert!(
                plan_loud_err.contains(expected),
                "expected `{expected}` from plan_loudness on {} with {opts:?}, got: {plan_loud_err}",
                path.display()
            );
            let norm_err = fvid_media::apply_loudnorm(path, &wav_out, None, opts).unwrap_err();
            assert!(
                norm_err.contains(expected),
                "expected `{expected}` from apply_loudnorm on {} with {opts:?}, got: {norm_err}",
                path.display()
            );
            let norm_dual_err =
                fvid_media::apply_loudnorm_dual(path, &wav_out, None, opts).unwrap_err();
            assert!(
                norm_dual_err.contains(expected),
                "expected `{expected}` from apply_loudnorm_dual on {} with {opts:?}, got: {norm_dual_err}",
                path.display()
            );
            let plan_norm_err = fvid_media::plan_loudnorm(path, None, false, opts).unwrap_err();
            assert!(
                plan_norm_err.contains(expected),
                "expected `{expected}` from plan_loudnorm on {} with {opts:?}, got: {plan_norm_err}",
                path.display()
            );
            let try_plan_err =
                fvid::native_loudnorm::try_plan(path, None, false, opts).unwrap_err();
            assert!(
                try_plan_err.to_string().contains(expected),
                "expected `{expected}` from native_loudnorm::try_plan on {} with {opts:?}, got: {try_plan_err}",
                path.display()
            );
            let try_apply_err =
                fvid::native_loudnorm::try_apply(path, &wav_out, None, false, opts).unwrap_err();
            assert!(
                try_apply_err.to_string().contains(expected),
                "expected `{expected}` from native_loudnorm::try_apply on {} with {opts:?}, got: {try_apply_err}",
                path.display()
            );
        }
    }

    // 2. `mix_audio_with_controls`, `plan_mix_audio_with_controls`, `merge_audio_with_controls`,
    // `plan_merge_audio_with_controls`, `native_audio_mix::plan_with_controls`,
    // `native_audio_mix::mix_audio_with_controls`, `native_audio_mix::plan_merge_with_controls`,
    // `native_audio_mix::merge_audio_with_controls`, and `native_audio_mix::concat_audio_with_options`
    // must reject `max_controlled_bytes: Some(0)` with `"controlled memory limit must be positive"`.
    let sources = vec![wav_a.clone(), wav_b.clone()];
    for err in [
        fvid_media::mix_audio_with_controls(
            &sources,
            &wav_out,
            &Default::default(),
            &zero_mem,
        )
        .unwrap_err(),
        fvid_media::plan_mix_audio_with_controls(&sources, &Default::default(), &zero_mem)
            .unwrap_err(),
        fvid_media::merge_audio_with_controls(&sources, &wav_out, &zero_mem).unwrap_err(),
        fvid_media::plan_merge_audio_with_controls(&sources, &zero_mem).unwrap_err(),
        fvid::native_audio_mix::plan_with_controls(&sources, &Default::default(), &zero_mem)
            .unwrap_err()
            .to_string(),
        fvid::native_audio_mix::mix_audio_with_controls(
            &sources,
            &wav_out,
            &Default::default(),
            &zero_mem,
        )
        .unwrap_err()
        .to_string(),
        fvid::native_audio_mix::plan_merge_with_controls(&sources, &zero_mem)
            .unwrap_err()
            .to_string(),
        fvid::native_audio_mix::merge_audio_with_controls(&sources, &wav_out, &zero_mem)
            .unwrap_err()
            .to_string(),
        fvid::native_audio_mix::concat_audio_with_options(&sources, &wav_out, &zero_mem)
            .unwrap_err()
            .to_string(),
    ] {
        assert!(
            err.contains("controlled memory limit must be positive"),
            "expected `controlled memory limit must be positive`, got: {err}"
        );
    }

    // 3. `native_audio_mix::mix_audio_with_controls`, `merge_audio_with_controls`, and
    // `concat_audio_with_options` into `.wav` must enforce `max_controlled_bytes` against
    // the edited `info_chunks` (`LIST INFO`) metadata when source + user metadata exceeds budget.
    let mut big_meta_set = Vec::new();
    for (k, v) in [
        ("title", "T".repeat(1024)),
        ("artist", "A".repeat(1024)),
        ("album", "B".repeat(1024)),
        ("genre", "G".repeat(1024)),
        ("copyright", "C".repeat(1024)),
        ("encoder", "E".repeat(1024)),
    ] {
        big_meta_set.push((k.to_string(), v));
    }
    let wav_meta_14k = dir.0.join("meta14k.wav");
    fvid_media::remux(
        &wav_a,
        &wav_meta_14k,
        &CopyOptions {
            metadata_set: vec![("comment".to_string(), "M".repeat(14_000))],
            ..Default::default()
        },
    )
    .unwrap();
    let concat_meta_opts = CopyOptions {
        max_controlled_bytes: Some(18_000),
        metadata_set: big_meta_set,
        ..Default::default()
    };
    let concat_meta_err = fvid::native_audio_mix::concat_audio_with_options(
        &[wav_meta_14k, wav_b],
        &wav_out,
        &concat_meta_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        concat_meta_err.contains("controlled memory budget exceeded"),
        "expected `controlled memory budget exceeded` from concat_audio_with_options when edited WAVE metadata exceeds budget, got: {concat_meta_err}"
    );
}

#[test]
fn lossless_xfade_subtitle_burn_convert_and_y4m_export_enforce_zero_limits() {
    let dir = TempDir::new("lossless-xfade-subtitle-zero-limits");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let y4m_src = root.join("subtitle-burn.y4m");
    let srt_src = root.join("subtitle-burn.srt");
    let mkv_out = dir.0.join("out.mkv");
    let y4m_out = dir.0.join("out.y4m");

    let zero_pkt = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };

    // 1. max_packet_bytes == 0 -> "packet byte limit must be positive"
    for err in [
        fvid_media::transcode_lossless(&y4m_src, &mkv_out, Default::default(), &zero_pkt)
            .unwrap_err(),
        fvid_media::plan_transcode_lossless(&y4m_src, &Default::default(), &zero_pkt, None)
            .unwrap_err(),
        fvid::media::transcode_lossless(&y4m_src, &mkv_out, Default::default(), &zero_pkt)
            .unwrap_err(),
        fvid::media::plan_transcode_lossless(&y4m_src, &Default::default(), &zero_pkt, None)
            .unwrap_err(),
        fvid_media::overlay_video(&y4m_src, &y4m_src, &mkv_out, 0, 0, &zero_pkt).unwrap_err(),
        fvid_media::plan_overlay(&y4m_src, &y4m_src, 0, 0, &zero_pkt).unwrap_err(),
        fvid_media::xfade_video(&y4m_src, &y4m_src, &mkv_out, "fade", 500_000, 500_000, &zero_pkt)
            .unwrap_err(),
        fvid_media::plan_xfade(&y4m_src, &y4m_src, "fade", 500_000, 500_000, &zero_pkt)
            .unwrap_err(),
        fvid_media::burn_subtitles(&y4m_src, &mkv_out, &srt_src, &zero_pkt).unwrap_err(),
        fvid_media::plan_burn_subtitles(&y4m_src, &srt_src, &zero_pkt).unwrap_err(),
        fvid_media::convert_subtitles_with_controls(
            &srt_src,
            &mkv_out,
            &Default::default(),
            &zero_pkt,
        )
        .unwrap_err(),
        fvid_media::plan_convert_subtitles_with_controls(&srt_src, &Default::default(), &zero_pkt)
            .unwrap_err(),
        fvid::native_export::export_y4m_pipeline_with_options(
            &y4m_src,
            &y4m_out,
            None,
            &Default::default(),
            &Default::default(),
            &zero_pkt,
        )
        .unwrap_err()
        .to_string(),
        fvid::native_plan::export_y4m_with_options(
            &y4m_src,
            None,
            &Default::default(),
            &Default::default(),
            &zero_pkt,
        )
        .unwrap_err(),
        fvid::media::plan_concat_y4m(&[y4m_src.clone(), y4m_src.clone()], &zero_pkt).unwrap_err(),
        fvid::media::plan_trim_y4m(&y4m_src, 0, 500_000, &zero_pkt).unwrap_err(),
    ] {
        assert!(
            err.contains("packet byte limit must be positive"),
            "expected `packet byte limit must be positive`, got: {err}"
        );
    }

    // 2. max_packets == Some(0) -> "packet work limit requires at least one packet"
    for err in [
        fvid_media::transcode_lossless(&y4m_src, &mkv_out, Default::default(), &zero_packets)
            .unwrap_err(),
        fvid_media::plan_transcode_lossless(&y4m_src, &Default::default(), &zero_packets, None)
            .unwrap_err(),
        fvid::media::transcode_lossless(&y4m_src, &mkv_out, Default::default(), &zero_packets)
            .unwrap_err(),
        fvid::media::plan_transcode_lossless(&y4m_src, &Default::default(), &zero_packets, None)
            .unwrap_err(),
        fvid_media::overlay_video(&y4m_src, &y4m_src, &mkv_out, 0, 0, &zero_packets).unwrap_err(),
        fvid_media::plan_overlay(&y4m_src, &y4m_src, 0, 0, &zero_packets).unwrap_err(),
        fvid_media::xfade_video(
            &y4m_src,
            &y4m_src,
            &mkv_out,
            "fade",
            500_000,
            500_000,
            &zero_packets,
        )
        .unwrap_err(),
        fvid_media::plan_xfade(&y4m_src, &y4m_src, "fade", 500_000, 500_000, &zero_packets)
            .unwrap_err(),
        fvid_media::burn_subtitles(&y4m_src, &mkv_out, &srt_src, &zero_packets).unwrap_err(),
        fvid_media::plan_burn_subtitles(&y4m_src, &srt_src, &zero_packets).unwrap_err(),
        fvid_media::convert_subtitles_with_controls(
            &srt_src,
            &mkv_out,
            &Default::default(),
            &zero_packets,
        )
        .unwrap_err(),
        fvid_media::plan_convert_subtitles_with_controls(
            &srt_src,
            &Default::default(),
            &zero_packets,
        )
        .unwrap_err(),
        fvid::native_export::export_y4m_pipeline_with_options(
            &y4m_src,
            &y4m_out,
            None,
            &Default::default(),
            &Default::default(),
            &zero_packets,
        )
        .unwrap_err()
        .to_string(),
        fvid::native_plan::export_y4m_with_options(
            &y4m_src,
            None,
            &Default::default(),
            &Default::default(),
            &zero_packets,
        )
        .unwrap_err(),
        fvid::media::plan_concat_y4m(&[y4m_src.clone(), y4m_src.clone()], &zero_packets)
            .unwrap_err(),
        fvid::media::plan_trim_y4m(&y4m_src, 0, 500_000, &zero_packets).unwrap_err(),
    ] {
        assert!(
            err.contains("packet work limit requires at least one packet"),
            "expected `packet work limit requires at least one packet`, got: {err}"
        );
    }

    // 3. max_controlled_bytes == Some(0) -> "controlled memory limit must be positive"
    for err in [
        fvid_media::transcode_lossless(&y4m_src, &mkv_out, Default::default(), &zero_mem)
            .unwrap_err(),
        fvid_media::plan_transcode_lossless(&y4m_src, &Default::default(), &zero_mem, None)
            .unwrap_err(),
        fvid::media::transcode_lossless(&y4m_src, &mkv_out, Default::default(), &zero_mem)
            .unwrap_err(),
        fvid::media::plan_transcode_lossless(&y4m_src, &Default::default(), &zero_mem, None)
            .unwrap_err(),
        fvid_media::overlay_video(&y4m_src, &y4m_src, &mkv_out, 0, 0, &zero_mem).unwrap_err(),
        fvid_media::plan_overlay(&y4m_src, &y4m_src, 0, 0, &zero_mem).unwrap_err(),
        fvid_media::xfade_video(
            &y4m_src,
            &y4m_src,
            &mkv_out,
            "fade",
            500_000,
            500_000,
            &zero_mem,
        )
        .unwrap_err(),
        fvid_media::plan_xfade(&y4m_src, &y4m_src, "fade", 500_000, 500_000, &zero_mem)
            .unwrap_err(),
        fvid_media::burn_subtitles(&y4m_src, &mkv_out, &srt_src, &zero_mem).unwrap_err(),
        fvid_media::plan_burn_subtitles(&y4m_src, &srt_src, &zero_mem).unwrap_err(),
        fvid_media::convert_subtitles_with_controls(
            &srt_src,
            &mkv_out,
            &Default::default(),
            &zero_mem,
        )
        .unwrap_err(),
        fvid_media::plan_convert_subtitles_with_controls(&srt_src, &Default::default(), &zero_mem)
            .unwrap_err(),
        fvid::native_export::export_y4m_pipeline_with_options(
            &y4m_src,
            &y4m_out,
            None,
            &Default::default(),
            &Default::default(),
            &zero_mem,
        )
        .unwrap_err()
        .to_string(),
        fvid::native_plan::export_y4m_with_options(
            &y4m_src,
            None,
            &Default::default(),
            &Default::default(),
            &zero_mem,
        )
        .unwrap_err(),
        fvid::media::plan_concat_y4m(&[y4m_src.clone(), y4m_src.clone()], &zero_mem).unwrap_err(),
        fvid::media::plan_trim_y4m(&y4m_src, 0, 500_000, &zero_mem).unwrap_err(),
    ] {
        assert!(
            err.contains("controlled memory limit must be positive"),
            "expected `controlled memory limit must be positive`, got: {err}"
        );
    }
}

#[test]
fn audio_export_plan_source_geometry_and_adts_concat_enforce_zero_limits() {
    let dir = TempDir::new("audio-export-plan-source-geometry-zero-limits");
    let wav_src = dir.0.join("src.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    let adts_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-mono-44k.aac");
    let wav_out = dir.0.join("out.wav");
    let mka_out = dir.0.join("out.mka");

    let zero_packet_bytes = fvid_control::CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = fvid_control::CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = fvid_control::CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };

    // 1. max_packet_bytes == 0 -> "packet byte limit must be positive"
    for err in [
        fvid_media::decode_audio(&wav_src, &wav_out, &zero_packet_bytes).unwrap_err(),
        fvid_media::plan_decode_audio(&wav_src, &Default::default(), &zero_packet_bytes)
            .unwrap_err(),
        fvid::media::decode_audio(&wav_src, &wav_out, &zero_packet_bytes).unwrap_err(),
        fvid::media::decode_audio(&wav_src, &mka_out, &zero_packet_bytes).unwrap_err(),
        fvid::media::plan_decode_audio(&wav_src, &Default::default(), &zero_packet_bytes)
            .unwrap_err(),
        fvid_media::owned_audio_plan::source_geometry(&wav_src, &zero_packet_bytes).unwrap_err(),
        fvid_media::owned_audio_plan::source_geometry_with_mask(&wav_src, &zero_packet_bytes)
            .unwrap_err(),
        fvid_media::concat(&[adts_src.clone(), adts_src.clone()], &mka_out, &zero_packet_bytes)
            .unwrap_err(),
        fvid_media::plan_concat(&[adts_src.clone(), adts_src.clone()], &zero_packet_bytes)
            .unwrap_err(),
    ] {
        assert!(
            err.contains("packet byte limit must be positive"),
            "expected `packet byte limit must be positive`, got: {err}"
        );
    }

    // 2. max_packets == Some(0) -> "packet work limit requires at least one packet"
    for err in [
        fvid_media::decode_audio(&wav_src, &wav_out, &zero_packets).unwrap_err(),
        fvid_media::plan_decode_audio(&wav_src, &Default::default(), &zero_packets).unwrap_err(),
        fvid::media::decode_audio(&wav_src, &wav_out, &zero_packets).unwrap_err(),
        fvid::media::decode_audio(&wav_src, &mka_out, &zero_packets).unwrap_err(),
        fvid::media::plan_decode_audio(&wav_src, &Default::default(), &zero_packets).unwrap_err(),
        fvid_media::owned_audio_plan::source_geometry(&wav_src, &zero_packets).unwrap_err(),
        fvid_media::owned_audio_plan::source_geometry_with_mask(&wav_src, &zero_packets)
            .unwrap_err(),
        fvid_media::concat(&[adts_src.clone(), adts_src.clone()], &mka_out, &zero_packets)
            .unwrap_err(),
        fvid_media::plan_concat(&[adts_src.clone(), adts_src.clone()], &zero_packets).unwrap_err(),
        fvid::native_export::remux_adts_aac_with_options(&adts_src, &mka_out, &zero_packets)
            .unwrap_err()
            .to_string(),
        fvid::native_export::concat_adts_aac_with_options(
            &[adts_src.clone(), adts_src.clone()],
            &mka_out,
            &zero_packets,
        )
        .unwrap_err()
        .to_string(),
        fvid::media::remux(&adts_src, &mka_out, &zero_packets).unwrap_err(),
        fvid::media::concat(&[adts_src.clone(), adts_src.clone()], &mka_out, &zero_packets)
            .unwrap_err(),
    ] {
        assert!(
            err.contains("packet work limit requires at least one packet"),
            "expected `packet work limit requires at least one packet`, got: {err}"
        );
    }

    // 3. max_controlled_bytes == Some(0) -> "controlled memory limit must be positive"
    for err in [
        fvid_media::decode_audio(&wav_src, &wav_out, &zero_mem).unwrap_err(),
        fvid_media::plan_decode_audio(&wav_src, &Default::default(), &zero_mem).unwrap_err(),
        fvid::media::decode_audio(&wav_src, &wav_out, &zero_mem).unwrap_err(),
        fvid::media::decode_audio(&wav_src, &mka_out, &zero_mem).unwrap_err(),
        fvid::media::plan_decode_audio(&wav_src, &Default::default(), &zero_mem).unwrap_err(),
        fvid_media::owned_audio_plan::source_geometry(&wav_src, &zero_mem).unwrap_err(),
        fvid_media::owned_audio_plan::source_geometry_with_mask(&wav_src, &zero_mem).unwrap_err(),
        fvid_media::concat(&[adts_src.clone(), adts_src.clone()], &mka_out, &zero_mem)
            .unwrap_err(),
        fvid_media::plan_concat(&[adts_src.clone(), adts_src.clone()], &zero_mem).unwrap_err(),
        fvid::media::concat(&[adts_src.clone(), adts_src.clone()], &mka_out, &zero_mem)
            .unwrap_err(),
        fvid::media::plan_concat(&[adts_src.clone(), adts_src.clone()], &zero_mem).unwrap_err(),
    ] {
        assert!(
            err.contains("controlled memory limit must be positive"),
            "expected `controlled memory limit must be positive`, got: {err}"
        );
    }
}

#[test]
fn raw_pcm_stream_decoders_enforce_zero_limits() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let adts_bytes =
        std::fs::read(root.join("tests/fixtures/audio/aac-mono-44k.aac")).unwrap();
    let mp4_bytes = std::fs::read(source("small.m4a")).unwrap();
    let mka_bytes =
        std::fs::read(root.join("tests/fixtures/audio/pcm-int.mkv")).unwrap();
    let flac_src =
        root.join("tests/fixtures/playback-errors/flac-streaminfo/valid_baseline.flac");
    let mp3_src =
        root.join("tests/fixtures/playback-errors/mp3-id3-metadata/tagged-id3v24.mp3");

    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };

    for (opts, expected) in [
        (&zero_packet_bytes, "packet byte limit must be positive"),
        (&zero_packets, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
    ] {
        let mut sink = std::io::sink();
        for err in [
            fvid_media::owned_aac::decode_adts_pcm(
                std::io::Cursor::new(&adts_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                std::io::Cursor::new(&mp4_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
                std::io::Cursor::new(&mp4_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_matroska_pcm::decode_matroska_pcm(
                std::io::Cursor::new(&mka_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_matroska_pcm::decode_matroska_pcm_f64(
                std::io::Cursor::new(&mka_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                std::io::Cursor::new(&mka_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
                std::io::Cursor::new(&mka_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
                std::io::Cursor::new(&mka_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_matroska_flac::decode_matroska_flac_pcm(
                std::io::Cursor::new(&mka_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_matroska_mp3::decode_matroska_mp3_pcm(
                std::io::Cursor::new(&mka_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_matroska_mpeg::decode_matroska_mpeg_pcm(
                std::io::Cursor::new(&mka_bytes),
                &mut sink,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_raw_flac::decode_raw_flac_pcm(&flac_src, &mut sink, None, opts)
                .unwrap_err(),
            fvid_media::owned_raw_mp3::decode_raw_mp3_pcm(&mp3_src, &mut sink, None, opts)
                .unwrap_err(),
            fvid_media::owned_raw_mp3::decode_raw_mpeg_pcm(&mp3_src, &mut sink, None, opts)
                .unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` for {opts:?}, got: {err}"
            );
        }
    }
}

#[test]
fn native_pcm_loudness_normalize_and_audio_source_info_enforce_zero_limits() {
    let dir = TempDir::new("native-pcm-loudness-normalize-zero-limits");
    let wav_src = dir.0.join("src.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    let wav_out = dir.0.join("out.wav");
    let adts_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-mono-44k.aac");
    let adts_bytes = std::fs::read(&adts_src).unwrap();
    let mp4_src = source("small.m4a");
    let target = fvid::native_pcm::NormalizeTarget {
        integrated_lufs: -16.0,
        sample_peak_dbfs: -1.0,
    };

    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };

    for (opts, expected) in [
        (&zero_packet_bytes, "packet byte limit must be positive"),
        (&zero_packets, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
    ] {
        for err in [
            fvid::media::measure_loudness(&wav_src, opts).unwrap_err(),
            fvid::media::measure_loudness(&adts_src, opts).unwrap_err(),
            fvid::media::measure_loudness(&mp4_src, opts).unwrap_err(),
            fvid::media::measure_loudness_with_weights(&wav_src, opts, &[1.0, 1.0]).unwrap_err(),
            fvid::media::normalize_loudness(&wav_src, &wav_out, opts, None, target).unwrap_err(),
            fvid::media::normalize_loudness(&adts_src, &wav_out, opts, None, target).unwrap_err(),
            fvid::media::normalize_loudness(
                &wav_src,
                &wav_out,
                opts,
                Some(&[1.0, 1.0]),
                target,
            )
            .unwrap_err(),
            fvid::native_pcm::loudness_channel_weights_with_options(&wav_src, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_pcm::measure_loudness_file_with_options(&wav_src, &[1.0, 1.0], opts)
                .unwrap_err()
                .to_string(),
            fvid::native_pcm::normalize_loudness_file_with_options(
                &wav_src,
                &wav_out,
                &[1.0, 1.0],
                target,
                opts,
                None,
            )
            .unwrap_err()
            .to_string(),
            fvid::native_media::aac_source_info_selected_with_options(&adts_src, None, None, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_media::aac_source_info_selected_with_options(&mp4_src, None, None, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_media::audio_source_info_selected_with_options(&adts_src, None, opts)
                .err()
                .expect("expected error")
                .to_string(),
            fvid::native_media::audio_source_info_selected_with_options(&mp4_src, None, opts)
                .err()
                .expect("expected error")
                .to_string(),
            fvid::native_media::inspect_adts_with_options(
                std::io::Cursor::new(&adts_bytes),
                opts,
            )
            .unwrap_err()
            .to_string(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` for {opts:?}, got: {err}"
            );
        }
    }
}

#[test]
fn native_audio_trim_mix_merge_and_source_admission_enforce_zero_limits() {
    let dir = TempDir::new("native-audio-trim-mix-merge-zero-limits");
    let wav_src = dir.0.join("src.wav");
    let wav_src2 = dir.0.join("src2.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src2, 48_000, 2, &[0.2; 960]).unwrap();
    let wav_out = dir.0.join("out.wav");
    let mp4_out = dir.0.join("out.mp4");
    let mka_out = dir.0.join("out.mka");
    let adts_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-mono-44k.aac");
    let mp4_src = source("small.m4a");
    let mix_opts = fvid::media::MixAudioOptions::default();

    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };

    for opts in [&zero_packet_bytes, &zero_packets, &zero_mem] {
        assert!(
            !fvid::native_audio_mix::eligible_with_options(
                &[wav_src.clone(), wav_src2.clone()],
                opts
            )
            .unwrap()
        );
    }

    assert!(
        fvid::native_media::is_aac_source_with_metadata_limit(&mp4_src, Some(0))
            .unwrap_err()
            .to_string()
            .contains("controlled memory limit must be positive")
    );
    assert!(
        fvid::native_export::remux_mp4_stats_with_memory_limit(
            &mp4_src,
            &mp4_out,
            None,
            None,
            Some(0),
        )
        .unwrap_err()
        .to_string()
        .contains("controlled memory limit must be positive")
    );
    assert!(
        fvid::native_export::remux_matroska_with_packet_and_memory_limits(
            &mp4_src,
            &mka_out,
            None,
            None,
            0,
            None,
        )
        .unwrap_err()
        .to_string()
        .contains("packet byte limit must be positive")
    );
    assert!(
        fvid::native_export::remux_matroska_with_packet_and_memory_limits(
            &mp4_src,
            &mka_out,
            None,
            None,
            4096,
            Some(0),
        )
        .unwrap_err()
        .to_string()
        .contains("controlled memory limit must be positive")
    );

    for (opts, expected) in [
        (&zero_packet_bytes, "packet byte limit must be positive"),
        (&zero_packets, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
    ] {
        for err in [
            fvid::native_media::is_owned_audio_source_with_options(&mp4_src, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_media::is_owned_audio_source_with_options(&adts_src, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_media::is_owned_audio_trim_source_with_options(&mp4_src, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_media::is_owned_audio_trim_source_with_options(&adts_src, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_plan::audio_trim_selection_with_options(&mp4_src, None, opts)
                .unwrap_err(),
            fvid::native_plan::trim_audio_with_options(&mp4_src, 0, 5_000, opts).unwrap_err(),
            fvid::native_plan::trim_audio_with_options(&adts_src, 0, 5_000, opts).unwrap_err(),
            fvid::native_plan::trim_aac_with_options(&mp4_src, 0, 5_000, opts).unwrap_err(),
            fvid::native_plan::trim_aac_with_options(&adts_src, 0, 5_000, opts).unwrap_err(),
            fvid::native_export::trim_audio_pcm_with_controls(&mp4_src, &wav_out, 0, 5_000, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_export::trim_audio_pcm_with_controls(&adts_src, &wav_out, 0, 5_000, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_export::export_audio_pcm_selected_with_controls(
                &wav_src,
                &wav_out,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid::native_export::export_audio_pcm_transformed_with_controls(
                &wav_src,
                &wav_out,
                None,
                1.0,
                None,
                None,
                opts,
            )
            .unwrap_err()
            .to_string(),
            fvid::media::mix_audio_with_controls(
                &[wav_src.clone(), wav_src2.clone()],
                &wav_out,
                &mix_opts,
                opts,
            )
            .unwrap_err(),
            fvid::media::plan_mix_audio_with_controls(
                &[wav_src.clone(), wav_src2.clone()],
                &mix_opts,
                opts,
            )
            .unwrap_err(),
            fvid::media::merge_audio_with_controls(
                &[wav_src.clone(), wav_src2.clone()],
                &wav_out,
                opts,
            )
            .unwrap_err(),
            fvid::media::plan_merge_audio_with_controls(
                &[wav_src.clone(), wav_src2.clone()],
                opts,
            )
            .unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` for {opts:?}, got: {err}"
            );
        }
    }
}

#[test]
fn public_media_routes_validate_copy_options_before_unbounded_source_io() {
    let dir = TempDir::new("public-media-routes-validate-options-before-io");
    let wav_src = dir.0.join("src.wav");
    let wav_src2 = dir.0.join("src2.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src2, 48_000, 2, &[0.2; 960]).unwrap();
    let wav_out = dir.0.join("out.wav");
    let mkv_out = dir.0.join("out.mkv");
    let m4a_out = dir.0.join("out.m4a");
    let adts_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-mono-44k.aac");
    let mp4_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-device-layout/normal-ten.mp4");
    let audio_mp4 = source("small.m4a");
    let corrupt_mp4 = dir.0.join("corrupt.mp4");
    let mut corrupt_bytes = vec![0u8; 32];
    corrupt_bytes[..12].copy_from_slice(b"\x00\x00\x00\x18ftypisom");
    corrupt_bytes[24..32].copy_from_slice(b"\x00\x00\x10\x00moov");
    std::fs::write(&corrupt_mp4, &corrupt_bytes).unwrap();
    let corrupt_wav = dir.0.join("corrupt.wav");
    std::fs::write(&corrupt_wav, b"RIFF\x10\x00\x00\x00WAVEfmt ").unwrap();

    let cancel = fvid::media::CancelFlag::new();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let tiny_mem = CopyOptions {
        max_controlled_bytes: Some(64),
        ..Default::default()
    };
    let deband_transform = fvid::media::LosslessTransform {
        deband: Some(String::new()),
        ..Default::default()
    };
    let settings = fvid::media::EncoderSettings {
        name: "ffv1".into(),
        options: Vec::new(),
    };

    for (opts, expected) in [
        (&cancelled_opts, "media operation cancelled"),
        (&zero_packet_bytes, "packet byte limit must be positive"),
        (&zero_packets, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
    ] {
        for err in [
            fvid::media::plan_overlay(&corrupt_mp4, &corrupt_mp4, 0, 0, opts).unwrap_err(),
            fvid::media::overlay_video(&corrupt_mp4, &corrupt_mp4, &mkv_out, 0, 0, opts)
                .unwrap_err(),
            fvid::media::transcode_lossless(&corrupt_mp4, &mkv_out, deband_transform.clone(), opts)
                .unwrap_err(),
            fvid::media::transcode(
                &corrupt_mp4,
                &mkv_out,
                deband_transform.clone(),
                opts,
                &settings,
            )
            .unwrap_err(),
            fvid::media::plan_transcode_lossless(&corrupt_mp4, &deband_transform, opts, None)
                .unwrap_err(),
            fvid::media::decode_audio(&corrupt_mp4, &wav_out, opts).unwrap_err(),
            fvid::media::plan_decode_audio(&corrupt_mp4, &Default::default(), opts).unwrap_err(),
            fvid::media::decode_audio(&audio_mp4, &wav_out, opts).unwrap_err(),
            fvid::media::plan_decode_audio(&audio_mp4, &Default::default(), opts).unwrap_err(),
            fvid::media::remux(&adts_src, &m4a_out, opts).unwrap_err(),
            fvid::media::plan_remux(&adts_src, opts).unwrap_err(),
            fvid::media::concat(&[adts_src.clone(), adts_src.clone()], &m4a_out, opts)
                .unwrap_err(),
            fvid::media::plan_concat(&[adts_src.clone(), adts_src.clone()], opts).unwrap_err(),
            fvid::media::plan_concat(&[wav_src.clone(), wav_src2.clone()], opts).unwrap_err(),
            fvid::media::plan_concat(&[corrupt_wav.clone(), corrupt_wav.clone()], opts)
                .unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` for {opts:?}, got: {err}"
            );
        }

        let mut wave_non_info_meta = opts.clone();
        wave_non_info_meta
            .stream_metadata_set
            .push((0, "language".into(), "eng".into()));
        let err = fvid::media::plan_concat(&[wav_src.clone(), wav_src2.clone()], &wave_non_info_meta)
            .unwrap_err();
        assert!(
            err.contains(expected),
            "expected `{expected}` for WAVE non-INFO metadata concat plan, got: {err}"
        );
    }

    for err in [
        fvid::media::plan_overlay(&mp4_src, &mp4_src, 0, 0, &tiny_mem).unwrap_err(),
        fvid::media::overlay_video(&mp4_src, &mp4_src, &mkv_out, 0, 0, &tiny_mem).unwrap_err(),
        fvid::media::transcode_lossless(&mp4_src, &mkv_out, deband_transform.clone(), &tiny_mem)
            .unwrap_err(),
        fvid::media::transcode(
            &mp4_src,
            &mkv_out,
            deband_transform.clone(),
            &tiny_mem,
            &settings,
        )
        .unwrap_err(),
        fvid::media::plan_transcode_lossless(&mp4_src, &deband_transform, &tiny_mem, None)
            .unwrap_err(),
    ] {
        assert!(
            err.contains("MP4 metadata exceeds budget"),
            "expected `MP4 metadata exceeds budget` for tiny_mem, got: {err}"
        );
    }
}

#[test]
fn owned_lossless_supports_ffv1_export_and_mp4_video_decode_enforce_zero_limits_and_preserve_plan_budgets() {
    let dir = TempDir::new("owned-lossless-supports-and-mp4-video-decode-zero-limits");
    let mp4_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-field-pcm-8bit-top-first.mp4");
    let mkv_src = dir.0.join("source.mkv");
    fvid_media::transcode_lossless(
        &mp4_src,
        &mkv_src,
        fvid_media::LosslessTransform::default(),
        &CopyOptions::default(),
    )
    .unwrap();

    let corrupt_mp4 = dir.0.join("corrupt.mp4");
    let mut corrupt_bytes = vec![0u8; 32];
    corrupt_bytes[..12].copy_from_slice(b"\x00\x00\x00\x18ftypisom");
    corrupt_bytes[24..32].copy_from_slice(b"\x00\x00\x10\x00moov");
    std::fs::write(&corrupt_mp4, &corrupt_bytes).unwrap();

    let corrupt_mkv = dir.0.join("corrupt.mkv");
    std::fs::write(&corrupt_mkv, b"\x1a\x45\xdf\xa3\x01\x00\x00\x00").unwrap();

    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_rss = CopyOptions {
        max_rss_bytes: Some(0),
        ..Default::default()
    };

    // 1. `fvid_media::supports_lossless` must reject zero-limit and RSS-exceeded `CopyOptions`
    // immediately without opening/decoding MP4 or Matroska containers.
    for opts in [
        &cancelled_opts,
        &zero_packet_bytes,
        &zero_packets,
        &zero_mem,
        &zero_rss,
    ] {
        assert!(
            !fvid_media::owned_lossless::supports(
                &mp4_src,
                &fvid_media::LosslessTransform::default(),
                opts,
            ),
            "supports_lossless on MP4 must return false for {opts:?}"
        );
        assert!(
            !fvid_media::owned_lossless::supports(
                &mkv_src,
                &fvid_media::LosslessTransform::default(),
                opts,
            ),
            "supports_lossless on MKV must return false for {opts:?}"
        );
        assert!(
            !fvid_media::owned_lossless::supports(
                &corrupt_mp4,
                &fvid_media::LosslessTransform::default(),
                opts,
            ),
            "supports_lossless on corrupt MP4 must return false for {opts:?}"
        );
        assert!(
            !fvid_media::owned_lossless::supports(
                &corrupt_mkv,
                &fvid_media::LosslessTransform::default(),
                opts,
            ),
            "supports_lossless on corrupt MKV must return false for {opts:?}"
        );
    }

    // 2. `fvid_media::plan_transcode_lossless` must check `check_rss_budget` before `supports()`
    // and preserve `max_packet_bytes`, `max_packets`, `max_controlled_bytes`, `max_rss_bytes`,
    // and `cancel` in its fallback `supports` probe.
    let rss_err = fvid_media::plan_transcode_lossless(
        &corrupt_mp4,
        &fvid_media::LosslessTransform::default(),
        &zero_rss,
        None,
    )
    .unwrap_err();
    assert!(
        rss_err.contains("rss budget exceeded"),
        "expected `rss budget exceeded` from plan_transcode_lossless before probing corrupt MP4, got: {rss_err}"
    );
}

#[test]
fn owned_remux_plan_and_wave_remux_trim_concat_enforce_zero_limits_and_rss_before_policy_errors() {
    let dir = TempDir::new("owned-remux-and-wave-remux-zero-limits-and-rss");
    let wav_src = dir.0.join("src.wav");
    let wav_src2 = dir.0.join("src2.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src2, 48_000, 2, &[0.2; 960]).unwrap();
    let wav_out = dir.0.join("out.wav");
    let mkv_out = dir.0.join("out.mkv");

    let corrupt_mp4 = dir.0.join("corrupt.mp4");
    let mut corrupt_bytes = vec![0u8; 32];
    corrupt_bytes[..12].copy_from_slice(b"\x00\x00\x00\x18ftypisom");
    corrupt_bytes[24..32].copy_from_slice(b"\x00\x00\x10\x00moov");
    std::fs::write(&corrupt_mp4, &corrupt_bytes).unwrap();

    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_rss = CopyOptions {
        max_rss_bytes: Some(0),
        ..Default::default()
    };

    for (opts, expected) in [
        (&cancelled_opts, "media operation cancelled"),
        (&zero_packet_bytes, "packet byte limit must be positive"),
        (&zero_packets, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
        (&zero_rss, "rss budget exceeded"),
    ] {
        // 1. `fvid_media::remux`, `fvid_media::plan_remux`, `fvid_media::trim`, and `fvid_media::plan_trim`
        // on a corrupt MP4 must validate `CopyOptions` before opening MP4 headers in `supports()`.
        for err in [
            fvid_media::remux(&corrupt_mp4, &mkv_out, opts).unwrap_err(),
            fvid_media::plan_remux(&corrupt_mp4, opts).unwrap_err(),
            fvid_media::trim(&corrupt_mp4, &wav_out, 0, 10_000, opts).unwrap_err(),
            fvid_media::plan_trim(&corrupt_mp4, 0, 10_000, opts).unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` on corrupt MP4 for {opts:?}, got: {err}"
            );
        }

        // 2. `fvid_media::{remux, plan_remux, trim, plan_trim, trim_pcm, plan_trim_pcm, concat, plan_concat}`
        // on WAVE files with unsupported stream metadata or non-`.wav` destination or invalid interval
        // must still report canonical `CopyOptions` limit/cancellation/RSS errors first!
        let mut unsupported_wave_opts = opts.clone();
        unsupported_wave_opts
            .stream_metadata_set
            .push((0, "language".into(), "eng".into()));
        for err in [
            fvid_media::remux(&wav_src, &mkv_out, &unsupported_wave_opts).unwrap_err(),
            fvid_media::plan_remux(&wav_src, &unsupported_wave_opts).unwrap_err(),
            fvid_media::trim(&wav_src, &wav_out, 10_000, 0, &unsupported_wave_opts).unwrap_err(),
            fvid_media::plan_trim(&wav_src, 10_000, 0, &unsupported_wave_opts).unwrap_err(),
            fvid_media::trim_pcm(&wav_src, &wav_out, 10_000, 0, &unsupported_wave_opts)
                .unwrap_err(),
            fvid_media::plan_trim_pcm(&wav_src, 10_000, 0, &unsupported_wave_opts).unwrap_err(),
            fvid_media::concat(
                &[wav_src.clone(), wav_src2.clone()],
                &wav_out,
                &unsupported_wave_opts,
            )
            .unwrap_err(),
            fvid_media::plan_concat(&[wav_src.clone(), wav_src2.clone()], &unsupported_wave_opts)
                .unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` on WAVE with unsupported policy/interval for {opts:?}, got: {err}"
            );
        }
    }
}

#[test]
fn owned_loudness_loudnorm_audio_export_supports_and_ogg_y4m_probe_enforce_options_before_policy_or_io() {
    let dir = TempDir::new("loudness-loudnorm-ogg-y4m-admission");
    let wav_src = dir.0.join("src.wav");
    let wav_out = dir.0.join("out.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    let adts_src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-mono-44k.aac");
    let mp4_src = source("small.m4a");
    let y4m_src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/subtitle-burn.y4m");

    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_rss = CopyOptions {
        max_rss_bytes: Some(0),
        ..Default::default()
    };

    for (opts, expected) in [
        (&cancelled_opts, "media operation cancelled"),
        (&zero_packet_bytes, "packet byte limit must be positive"),
        (&zero_packets, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
        (&zero_rss, "rss budget exceeded"),
    ] {
        // 1. `fvid_media::plan_loudnorm` on WAVE with cancelled options, and `measure_loudness`,
        // `plan_loudness`, `apply_loudnorm`, `apply_loudnorm_dual`, `plan_loudnorm` across WAVE,
        // ADTS, and MP4 even when `stream_metadata_set` is populated, must report canonical
        // `CopyOptions` errors first instead of route/policy errors!
        let mut unsupported_meta_opts = opts.clone();
        unsupported_meta_opts
            .stream_metadata_set
            .push((0, "language".into(), "eng".into()));

        for path in [&wav_src, &adts_src, &mp4_src] {
            for candidate_opts in [opts, &unsupported_meta_opts] {
                for err in [
                    fvid_media::measure_loudness(path, candidate_opts).unwrap_err(),
                    fvid_media::plan_loudness(path, candidate_opts).unwrap_err(),
                    fvid_media::apply_loudnorm(path, &wav_out, None, candidate_opts).unwrap_err(),
                    fvid_media::apply_loudnorm_dual(path, &wav_out, None, candidate_opts)
                        .unwrap_err(),
                    fvid_media::plan_loudnorm(path, None, false, candidate_opts).unwrap_err(),
                    fvid_media::plan_loudnorm(path, None, true, candidate_opts).unwrap_err(),
                ] {
                    assert!(
                        err.contains(expected),
                        "expected `{expected}` on {} for {candidate_opts:?}, got: {err}",
                        path.display()
                    );
                }
            }
        }

        // 2. `fvid_media::owned_probe::probe_ogg_with_options` and `fvid_media::owned_y4m_probe::try_y4m_with_options` must
        // validate `CopyOptions` before opening or signature-checking the file!
        for err in [
            fvid_media::owned_probe::probe_ogg_with_options(&wav_src, opts).unwrap_err(),
            fvid_media::owned_y4m_probe::try_y4m_with_options(&y4m_src, opts).unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` from probe_ogg_with_options / try_y4m_with_options for {opts:?}, got: {err}"
            );
        }
    }
}

#[test]
fn audio_mix_merge_concat_native_loudnorm_and_multitrack_supports_enforce_options_before_input_count_or_args() {
    let dir = TempDir::new("mix-merge-concat-loudnorm-multitrack-admission");
    let wav_src = dir.0.join("src.wav");
    let wav_out = dir.0.join("out.wav");
    let mkv_out = dir.0.join("out.mkv");
    let bad_out = dir.0.join("out.mp3");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    let y4m_src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/subtitle-burn.y4m");
    let srt_src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/subtitle-burn.srt");

    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_rss = CopyOptions {
        max_rss_bytes: Some(0),
        ..Default::default()
    };

    for (opts, expected) in [
        (&cancelled_opts, "media operation cancelled"),
        (&zero_packet_bytes, "packet byte limit must be positive"),
        (&zero_packets, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
        (&zero_rss, "rss budget exceeded"),
    ] {
        let mut unsupported_meta_opts = opts.clone();
        unsupported_meta_opts
            .stream_metadata_set
            .push((0, "language".into(), "eng".into()));

        // 1. `plan_mix_audio_with_controls`, `plan_merge_audio_with_controls`, `mix_audio_with_controls`,
        // `merge_audio_with_controls`, `native_audio_mix::{plan_with_controls, mix_audio_with_controls,
        // plan_merge_with_controls, merge_audio_with_controls, validate_concat_segments_with_options,
        // concat_audio_with_options}` must report canonical `CopyOptions` errors even when input count
        // is 1 or destination extension is `.mp3` or `stream_metadata_set` is populated!
        let one_src = vec![wav_src.clone()];
        let two_srcs = vec![wav_src.clone(), wav_src.clone()];
        for candidate_opts in [opts, &unsupported_meta_opts] {
            for err in [
                fvid_media::plan_mix_audio_with_controls(&one_src, &Default::default(), candidate_opts)
                    .unwrap_err(),
                fvid_media::plan_merge_audio_with_controls(&one_src, candidate_opts).unwrap_err(),
                fvid_media::mix_audio_with_controls(
                    &two_srcs,
                    &bad_out,
                    &Default::default(),
                    candidate_opts,
                )
                .unwrap_err(),
                fvid_media::merge_audio_with_controls(&two_srcs, &bad_out, candidate_opts)
                    .unwrap_err(),
                fvid::native_audio_mix::plan_with_controls(&one_src, &Default::default(), candidate_opts)
                    .unwrap_err()
                    .to_string(),
                fvid::native_audio_mix::mix_audio_with_controls(
                    &one_src,
                    &bad_out,
                    &Default::default(),
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid::native_audio_mix::plan_merge_with_controls(&one_src, candidate_opts)
                    .unwrap_err()
                    .to_string(),
                fvid::native_audio_mix::merge_audio_with_controls(&one_src, &bad_out, candidate_opts)
                    .unwrap_err()
                    .to_string(),
                fvid::native_audio_mix::validate_concat_segments_with_options(&one_src, candidate_opts)
                    .unwrap_err()
                    .to_string(),
                fvid::native_audio_mix::concat_audio_with_options(&one_src, &bad_out, candidate_opts)
                    .unwrap_err()
                    .to_string(),
                fvid::native_loudnorm::try_plan(&wav_src, Some("bad_arg"), false, candidate_opts)
                    .unwrap_err()
                    .to_string(),
                fvid::native_loudnorm::try_apply(
                    &wav_src,
                    &mkv_out,
                    Some("bad_arg"),
                    false,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid::native_loudnorm::try_apply(
                    &wav_src,
                    &wav_out,
                    Some("bad_arg"),
                    false,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
            ] {
                assert!(
                    err.contains(expected),
                    "expected `{expected}` for {candidate_opts:?}, got: {err}"
                );
            }
        }

        assert!(
            !fvid::native_audio_mix::eligible_with_options(&two_srcs, opts).unwrap(),
            "expected eligible_with_options == false for {opts:?}"
        );
        assert!(
            !fvid_media::owned_subtitle_burn::supports(&y4m_src, &srt_src, opts),
            "expected owned_subtitle_burn::supports == false for {opts:?}"
        );
        assert_eq!(
            fvid_media::owned_xfade::try_plan_xfade(&y4m_src, &y4m_src, "fade", 500_000, 500_000, opts)
                .unwrap(),
            None,
            "expected try_plan_xfade == Ok(None) for {opts:?}"
        );
        assert!(
            fvid_media::owned_xfade::try_xfade_video(
                &y4m_src,
                &y4m_src,
                &mkv_out,
                "fade",
                500_000,
                500_000,
                opts,
            )
            .unwrap()
            .is_none(),
            "expected try_xfade_video == Ok(None) for {opts:?}"
        );
    }
}

#[test]
fn raw_pcm_decoders_and_audio_export_backends_enforce_options_before_stream_metadata_or_destination_checks() {
    let dir = TempDir::new("raw-pcm-decoders-and-export-admission");
    let wav_src = dir.0.join("src.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    let adts_src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-stereo.aac");
    let mp4_src = source("small.m4a");
    let mka_src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-stereo.mka");
    let flac_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-index-growth/frames-129.flac");
    let mp3_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp3-id3-metadata/tagged-id3v24.mp3");

    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_rss = CopyOptions {
        max_rss_bytes: Some(0),
        ..Default::default()
    };

    for (opts, expected) in [
        (&cancelled_opts, "media operation cancelled"),
        (&zero_packet_bytes, "packet byte limit must be positive"),
        (&zero_packets, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
        (&zero_rss, "rss budget exceeded"),
    ] {
        //Even when `metadata_set`, `streams: vec![1, 2]`, or an invalid interval `Some((1, 0))`
        //is supplied, raw PCM stream decoders and containerrecognizers must report the canonical
        //`CopyOptions` error first!
        let mut bad_stream_meta_opts = opts.clone();
        bad_stream_meta_opts.streams = vec![1, 2];
        bad_stream_meta_opts
            .metadata_set
            .push(("title".into(), "test".into()));
        let bad_interval = Some((
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(1),
        ));

        for candidate_opts in [opts, &bad_stream_meta_opts] {
            for err in [
                fvid_media::owned_aac::decode_adts_pcm(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_raw_flac::decode_raw_flac_pcm(
                    &flac_src,
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err(),
                fvid_media::owned_raw_mp3::decode_raw_mp3_pcm(
                    &mp3_src,
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err(),
                fvid_media::owned_raw_mp3::decode_raw_mpeg_pcm(
                    &mp3_src,
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err(),
                fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_matroska_pcm::decode_matroska_pcm(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_matroska_pcm::decode_matroska_pcm_f64(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_matroska_flac::decode_matroska_flac_pcm(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_matroska_mpeg::decode_matroska_mpeg_pcm(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_matroska_mp3::decode_matroska_mp3_pcm(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
                    std::io::Cursor::new(&[0u8; 16]),
                    &mut std::io::sink(),
                    bad_interval,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid_media::owned_container_loudness::recognizes_standalone_audio_with_options(
                    &wav_src,
                    candidate_opts,
                )
                .unwrap_err(),
                fvid_media::owned_container_loudness::decode_adts_to_wave(&adts_src, candidate_opts)
                    .err()
                    .expect("expected decode_adts_to_wave to fail"),
                fvid_media::owned_audio_plan::source_geometry(&mp4_src, candidate_opts)
                    .unwrap_err(),
                fvid_media::owned_audio_plan::source_geometry(&mka_src, candidate_opts)
                    .unwrap_err(),
            ] {
                assert!(
                    err.contains(expected),
                    "expected `{expected}` for {candidate_opts:?}, got: {err}"
                );
            }
        }
    }
}

#[test]
fn native_export_native_plan_and_video_decode_enforce_options_before_extension_input_count_or_interval_checks() {
    let dir = TempDir::new("native-export-plan-video-admission");
    let y4m_src = dir.0.join("src.y4m");
    std::fs::write(&y4m_src, b"YUV4MPEG2 W2 H2 F24:1 Ip A1:1 C420\nFRAME\n\0\0\0\0\0\0").unwrap();
    let adts_src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-stereo.aac");
    let mp4_src = source("small.m4a");
    let bad_ext_dst = dir.0.join("out.txt");
    let existing_mkv = dir.0.join("existing.mkv");
    std::fs::write(&existing_mkv, b"already-present").unwrap();

    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_rss = CopyOptions {
        max_rss_bytes: Some(0),
        ..Default::default()
    };

    for (opts, expected) in [
        (&cancelled_opts, "media operation cancelled"),
        (&zero_packet_bytes, "packet byte limit must be positive"),
        (&zero_packets, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
        (&zero_rss, "rss budget exceeded"),
    ] {
        let mut bad_stream_meta_opts = opts.clone();
        bad_stream_meta_opts.streams = vec![1, 2];
        bad_stream_meta_opts
            .metadata_set
            .push(("title".into(), "test".into()));
        let bad_interval = Some((
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(1),
        ));

        for candidate_opts in [opts, &bad_stream_meta_opts] {
            for err in [
                fvid::native_export::export_y4m_pipeline_with_options(
                    &y4m_src,
                    &bad_ext_dst,
                    bad_interval,
                    &Default::default(),
                    &Default::default(),
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid::native_export::concat_y4m_with_options(
                    std::slice::from_ref(&y4m_src),
                    &bad_ext_dst,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid::native_export::trim_y4m_with_options(
                    &y4m_src,
                    &bad_ext_dst,
                    5,
                    1,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid::native_export::concat_adts_aac_with_options(
                    std::slice::from_ref(&adts_src),
                    &bad_ext_dst,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid::native_export::trim_audio_pcm_with_controls(
                    &mp4_src,
                    &bad_ext_dst,
                    5,
                    1,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid::native_export::try_concat_mp4_matroska_with_options(
                    std::slice::from_ref(&mp4_src),
                    &existing_mkv,
                    candidate_opts,
                )
                .unwrap_err()
                .to_string(),
                fvid::native_plan::decode_audio_with_options(
                    &mp4_src,
                    &fvid::media_info::AudioDecodeTransform {
                        interval: Some((5, 1)),
                        ..Default::default()
                    },
                    candidate_opts,
                )
                .unwrap_err(),
                fvid::native_plan::remux_adts_with_options(&adts_src, candidate_opts).unwrap_err(),
                fvid::native_plan::concat_adts_with_options(
                    std::slice::from_ref(&adts_src),
                    candidate_opts,
                )
                .unwrap_err(),
                fvid::native_plan::trim_adts_with_options(&adts_src, 5, 1, candidate_opts)
                    .unwrap_err(),
                fvid::native_plan::concat_y4m_with_options(
                    std::slice::from_ref(&y4m_src),
                    candidate_opts,
                )
                .unwrap_err(),
                fvid::native_plan::trim_y4m_with_options(&y4m_src, 5, 1, candidate_opts)
                    .unwrap_err(),
                fvid::native_plan::export_y4m_with_options(
                    &y4m_src,
                    bad_interval,
                    &Default::default(),
                    &Default::default(),
                    candidate_opts,
                )
                .unwrap_err(),
                fvid::native_plan::loudness_with_options(&mp4_src, candidate_opts, None)
                    .unwrap_err(),
                fvid::native_plan::normalize_loudness_with_options(
                    &mp4_src,
                    candidate_opts,
                    None,
                    fvid::native_pcm::NormalizeTarget {
                        integrated_lufs: f64::NAN,
                        sample_peak_dbfs: -1.0,
                    },
                )
                .unwrap_err(),
                fvid::native_plan::concat_matroska_to_with_options(
                    std::slice::from_ref(&mp4_src),
                    &existing_mkv,
                    candidate_opts,
                )
                .unwrap_err(),
                fvid::native_plan::concat_mp4_matroska_with_options(
                    std::slice::from_ref(&mp4_src),
                    candidate_opts,
                )
                .unwrap_err(),
                fvid::media::measure_loudness(&mp4_src, candidate_opts)
                    .unwrap_err(),
                fvid::media::normalize_loudness(
                    &mp4_src,
                    &bad_ext_dst,
                    candidate_opts,
                    None,
                    fvid::native_pcm::NormalizeTarget {
                        integrated_lufs: f64::NAN,
                        sample_peak_dbfs: -1.0,
                    },
                )
                .unwrap_err(),
                fvid_media::owned_lossless::transcode_lossless(
                    &y4m_src,
                    &bad_ext_dst,
                    Default::default(),
                    candidate_opts,
                )
                .unwrap_err(),
                fvid_media::owned_matroska_remux::remux(
                    &existing_mkv,
                    &existing_mkv,
                    candidate_opts,
                )
                .unwrap_err(),
            ] {
                assert!(
                    err.contains(expected),
                    "expected `{expected}` for {candidate_opts:?}, got: {err}"
                );
            }
        }
    }
}

#[test]
fn media_entrypoints_matroska_probe_and_subtitle_conversion_enforce_options_and_memory_packet_bounds() {
    let dir = TempDir::new("media-entrypoints-probe-subtitle-admission");
    let missing_src = dir.0.join("missing.mp4");
    let wav_src = dir.0.join("src.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    let adts_src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-stereo.aac");
    let mkv_src = dir.0.join("src.mka");
    fvid::native_export::remux_adts_aac_with_options(
        &adts_src,
        &mkv_src,
        &CopyOptions::default(),
    )
    .unwrap();
    let mkv_dst = dir.0.join("out.mkv");
    let mp4_dst = dir.0.join("out.mp4");
    let bad_ext_dst = dir.0.join("out.txt");
    let bad_settings = fvid::media_info::EncoderSettings {
        name: String::new(),
        options: Vec::new(),
    };

    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_mem = CopyOptions {
        max_controlled_bytes: Some(0),
        ..Default::default()
    };
    let zero_rss = CopyOptions {
        max_rss_bytes: Some(0),
        ..Default::default()
    };

    for (opts, expected) in [
        (&cancelled_opts, "media operation cancelled"),
        (&zero_packet_bytes, "packet byte limit must be positive"),
        (&zero_packets, "packet work limit requires at least one packet"),
        (&zero_mem, "controlled memory limit must be positive"),
        (&zero_rss, "rss budget exceeded"),
    ] {
        for err in [
            fvid::media::remux(&missing_src, &mp4_dst, opts).unwrap_err(),
            fvid::media::plan_remux(&missing_src, opts).unwrap_err(),
            fvid::media::trim_pcm(&missing_src, &mkv_dst, 5, 1, opts).unwrap_err(),
            fvid::media::plan_trim_pcm(&missing_src, 5, 1, opts).unwrap_err(),
            fvid::media::trim(&missing_src, &mkv_dst, 5, 1, opts).unwrap_err(),
            fvid::media::plan_trim(&missing_src, 5, 1, opts).unwrap_err(),
            fvid::media::concat(std::slice::from_ref(&missing_src), &mkv_dst, opts).unwrap_err(),
            fvid::media::plan_concat(std::slice::from_ref(&missing_src), opts).unwrap_err(),
            fvid::media::transcode_lossless(&missing_src, &bad_ext_dst, Default::default(), opts)
                .unwrap_err(),
            fvid::media::transcode(
                &missing_src,
                &bad_ext_dst,
                Default::default(),
                opts,
                &bad_settings,
            )
            .unwrap_err(),
            fvid_media::owned_lossless::transcode(
                &missing_src,
                &bad_ext_dst,
                Default::default(),
                opts,
                &bad_settings,
            )
            .unwrap_err(),
            fvid::media::plan_transcode_lossless(
                &missing_src,
                &Default::default(),
                opts,
                Some("unknown"),
            )
            .unwrap_err(),
            fvid::media::mix_audio_with_controls(
                std::slice::from_ref(&missing_src),
                &bad_ext_dst,
                &Default::default(),
                opts,
            )
            .unwrap_err(),
            fvid::media::plan_mix_audio_with_controls(
                std::slice::from_ref(&missing_src),
                &Default::default(),
                opts,
            )
            .unwrap_err(),
            fvid::media::merge_audio_with_controls(
                std::slice::from_ref(&missing_src),
                &bad_ext_dst,
                opts,
            )
            .unwrap_err(),
            fvid::media::plan_merge_audio_with_controls(
                std::slice::from_ref(&missing_src),
                opts,
            )
            .unwrap_err(),
            fvid::media::convert_subtitles_with_controls(
                &missing_src,
                &bad_ext_dst,
                &Default::default(),
                opts,
            )
            .unwrap_err(),
            fvid::media::plan_convert_subtitles_for_destination(
                &missing_src,
                Some(&bad_ext_dst),
                &Default::default(),
                opts,
            )
            .unwrap_err(),
            fvid_media::owned_concat::concat(
                std::slice::from_ref(&wav_src),
                &mkv_dst,
                opts,
            )
            .unwrap_err(),
            fvid_media::owned_concat::plan_concat(
                std::slice::from_ref(&wav_src),
                opts,
            )
            .unwrap_err(),
            fvid_media::owned_webm_probe::probe_webm_with_options(&mkv_src, opts).unwrap_err(),
            fvid::media::probe_as_with_options(&mkv_src, Some("matroska"), opts).unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` for {opts:?}, got: {err}"
            );
        }
    }

    // Dual-failure: cancelled + max_packet_bytes=0 + max_rss_bytes=0 must report cancellation first,
    // and max_packet_bytes=0 + max_rss_bytes=0 must report packet byte limit before RSS budget!
    let cancel2 = fvid_media::CancelFlag::default();
    cancel2.cancel();
    let cancelled_and_zero = CopyOptions {
        cancel: Some(cancel2),
        max_packet_bytes: 0,
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_pkt_and_rss = CopyOptions {
        max_packet_bytes: 0,
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    for err in [
        fvid_media::owned_webm_probe::probe_webm_with_options(&mkv_src, &cancelled_and_zero)
            .unwrap_err(),
        fvid::media::probe_as_with_options(&mkv_src, Some("matroska"), &cancelled_and_zero)
            .unwrap_err(),
        fvid::native_subtitle::try_plan_with_controls(
            &mkv_src,
            Some(&mkv_dst),
            &Default::default(),
            &cancelled_and_zero,
        )
        .unwrap_err()
        .to_string(),
        fvid::native_subtitle::try_convert_with_controls(
            &mkv_src,
            &mkv_dst,
            &Default::default(),
            &cancelled_and_zero,
        )
        .unwrap_err()
        .to_string(),
        fvid_media::owned_concat::concat(
            std::slice::from_ref(&wav_src),
            &mkv_dst,
            &cancelled_and_zero,
        )
        .unwrap_err(),
        fvid_media::owned_concat::plan_concat(
            std::slice::from_ref(&wav_src),
            &cancelled_and_zero,
        )
        .unwrap_err(),
    ] {
        assert!(
            err.contains("media operation cancelled"),
            "expected cancellation first, got: {err}"
        );
    }
    for err in [
        fvid_media::owned_webm_probe::probe_webm_with_options(&mkv_src, &zero_pkt_and_rss)
            .unwrap_err(),
        fvid::media::probe_as_with_options(&mkv_src, Some("matroska"), &zero_pkt_and_rss)
            .unwrap_err(),
    ] {
        assert!(
            err.contains("packet byte limit must be positive"),
            "expected packet byte limit before RSS budget, got: {err}"
        );
    }

    // Matroska probe must bound packet index by max_controlled_bytes!
    let tiny_mem_opts = CopyOptions {
        max_controlled_bytes: Some(1),
        ..Default::default()
    };
    let probe_err =
        fvid::media::probe_as_with_options(&mkv_src, Some("matroska"), &tiny_mem_opts).unwrap_err();
    assert!(
        probe_err.contains("controlled memory budget exceeded"),
        "expected Matroska probe to reject packet index exceeding max_controlled_bytes=1, got: {probe_err}"
    );
}

#[test]
fn native_pcm_native_media_and_owned_remux_video_bridge_burn_and_xfade_enforce_canonical_option_order() {
    let dir = TempDir::new("native-pcm-media-owned-remux-bridge-burn-xfade-order");
    let wav_src = source("small.wav");
    let mp4_src = source("small.m4a");
    let adts_src = source("small.aac");
    let mkv_src = dir.0.join("small.mka");
    fvid::media::decode_audio(&mp4_src, &mkv_src, &CopyOptions::default()).unwrap();
    let bad_dst = dir.0.join("out.txt");

    let cancel = fvid_media::CancelFlag::default();
    cancel.cancel();

    let cancelled_bad_stream = CopyOptions {
        cancel: Some(cancel.clone()),
        streams: vec![0, 1],
        max_packet_bytes: 0,
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_pkt_bad_stream = CopyOptions {
        streams: vec![0, 1],
        max_packet_bytes: 0,
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_cnt_bad_stream = CopyOptions {
        streams: vec![0, 1],
        max_packets: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_mem_bad_stream = CopyOptions {
        streams: vec![0, 1],
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_rss_bad_stream = CopyOptions {
        streams: vec![0, 1],
        max_rss_bytes: Some(0),
        ..Default::default()
    };

    let invalid_target = fvid::native_pcm::NormalizeTarget {
        integrated_lufs: f64::NAN,
        sample_peak_dbfs: -1.0,
    };

    for (opts, expected) in [
        (&cancelled_bad_stream, "media operation cancelled"),
        (&zero_pkt_bad_stream, "packet byte limit must be positive"),
        (
            &zero_cnt_bad_stream,
            "packet work limit requires at least one packet",
        ),
        (
            &zero_mem_bad_stream,
            "controlled memory limit must be positive",
        ),
        (
            &zero_rss_bad_stream,
            "rss",
        ),
    ] {
        for err in [
            fvid::native_pcm::measure_loudness_file_with_options(&wav_src, &[1.0, 1.0], opts)
                .unwrap_err()
                .to_string(),
            fvid::native_pcm::loudness_channel_weights_with_options(&wav_src, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_pcm::normalize_loudness_file_with_options(
                &wav_src,
                &bad_dst,
                &[1.0, 1.0],
                invalid_target,
                opts,
                None,
            )
            .unwrap_err()
            .to_string(),
            fvid::native_media::aac_source_info_selected_with_options(&adts_src, None, None, opts)
                .unwrap_err()
                .to_string(),
            fvid::native_media::audio_source_info_selected_with_interval_and_options(
                &mp4_src, None, None, opts,
            )
            .err()
            .unwrap()
            .to_string(),
            fvid_media::owned_adts_remux::plan_remux(&adts_src, opts).unwrap_err(),
            fvid_media::owned_matroska_remux::remux(&mkv_src, &bad_dst, opts).unwrap_err(),
            fvid_media::owned_subtitle_burn::plan_burn_subtitles(&wav_src, &wav_src, opts)
                .unwrap_err(),
            fvid_media::owned_xfade::xfade_video(
                &wav_src,
                &wav_src,
                &bad_dst,
                "wipeleft",
                0,
                0,
                opts,
            )
            .unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` for {opts:?}, got: {err}"
            );
        }
    }

    // Also verify `native_pcm::{trim_wave, concat_info_with_cancel, concat_wave}` check cancellation
    // before interval, destination-extension, or input-count checks!
    for err in [
        fvid::native_pcm::trim_wave(&wav_src, &bad_dst, 10, 5, Some(&cancel), None)
            .unwrap_err()
            .to_string(),
        fvid::native_pcm::concat_info_with_cancel(std::slice::from_ref(&wav_src), Some(&cancel))
            .unwrap_err()
            .to_string(),
        fvid::native_pcm::concat_wave(
            std::slice::from_ref(&wav_src),
            &bad_dst,
            Some(&cancel),
            None,
        )
        .unwrap_err()
        .to_string(),
    ] {
        assert!(
            err.contains("media operation cancelled"),
            "expected cancellation before interval/extension/input-count in native_pcm, got: {err}"
        );
    }
}

#[test]
fn overlay_pcm_export_mp4_concat_and_wave_loudness_enforce_canonical_option_order() {
    let dir = TempDir::new("overlay_pcm_concat_wave_loudness_order");
    let wav_src = source("small.wav");
    let bad_mp4_dst = dir.0.join("out.mp4");
    let wav_dst = dir.0.join("out.wav");

    let cancel = fvid::media_control::CancelFlag::new();
    cancel.cancel();

    let cancelled_opts = CopyOptions {
        cancel: Some(cancel.clone()),
        max_packet_bytes: 0,
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_pkt_opts = CopyOptions {
        max_packet_bytes: 0,
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_cnt_opts = CopyOptions {
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_mem_opts = CopyOptions {
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };

    // 1. overlay_video_transformed checks cancellation before destination `.mkv` extension check.
    let overlay_err = fvid::native_export::overlay_video_transformed(
        &wav_src,
        &wav_src,
        &bad_mp4_dst,
        0,
        0,
        Some(&cancel),
        None,
        &fvid::native_geometry::VideoGeometry::default(),
        &fvid::native_pixels::PixelFilters::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        overlay_err.contains("media operation cancelled"),
        "expected cancellation before .mkv check in overlay_video_transformed, got: {overlay_err}"
    );

    // 2. export_audio_pcm_selected_with_controls checks cancellation before zero packet/work/memory limits.
    let pcm_err = fvid::native_export::export_audio_pcm_selected_with_controls(
        &wav_src,
        &wav_dst,
        None,
        &cancelled_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        pcm_err.contains("media operation cancelled"),
        "expected cancellation before zero packet limit in export_audio_pcm_selected_with_controls, got: {pcm_err}"
    );

    // 3. mp4_concat::open_with_options and write_with_options check canonical option order.
    let concat_open_cnt_err = fvid::container::mp4_concat::open_with_options(
        std::slice::from_ref(&wav_src),
        &zero_cnt_opts,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(
        concat_open_cnt_err.contains("packet work limit requires at least one packet"),
        "expected packet work limit before controlled memory limit in mp4_concat::open_with_options, got: {concat_open_cnt_err}"
    );

    let mut empty_segments: [fvid::container::mp4_concat::Segment<std::io::Cursor<Vec<u8>>>; 0] =
        [];
    let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
    let concat_write_cancel_err = fvid::container::mp4_concat::write_with_options(
        &mut empty_segments,
        &mut cursor,
        &cancelled_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        concat_write_cancel_err.contains("media operation cancelled"),
        "expected cancellation before zero packet limit in mp4_concat::write_with_options, got: {concat_write_cancel_err}"
    );
    let concat_write_pkt_err = fvid::container::mp4_concat::write_with_options(
        &mut empty_segments,
        &mut cursor,
        &zero_pkt_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        concat_write_pkt_err.contains("packet byte limit must be positive"),
        "expected packet byte limit in mp4_concat::write_with_options, got: {concat_write_pkt_err}"
    );
    let concat_write_mem_err = fvid::container::mp4_concat::write_with_options(
        &mut empty_segments,
        &mut cursor,
        &zero_mem_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        concat_write_mem_err.contains("controlled memory limit must be positive"),
        "expected controlled memory limit in mp4_concat::write_with_options, got: {concat_write_mem_err}"
    );

    // 4. fvid_media::measure_loudness and plan_loudness check zero packet/work/memory limits before RSS budget.
    for (opts, expected) in [
        (&cancelled_opts, "media operation cancelled"),
        (&zero_pkt_opts, "packet byte limit must be positive"),
        (
            &zero_cnt_opts,
            "packet work limit requires at least one packet",
        ),
        (&zero_mem_opts, "controlled memory limit must be positive"),
    ] {
        for err in [
            fvid_media::measure_loudness(&wav_src, opts).unwrap_err(),
            fvid_media::plan_loudness(&wav_src, opts).unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` for {opts:?}, got: {err}"
            );
        }
    }
}

#[test]
fn remux_adts_concat_mp4_relocate_and_matroska_copy_enforce_canonical_option_order() {
    let dir = TempDir(
        std::env::temp_dir().join(format!("fvid-batch-372-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();

    let adts_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-mono-44k.aac");
    let mp4_src = source("small.m4a");
    let mka_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/pcm-int.mkv");
    let mp4_dst = dir.0.join("out.mp4");
    let mka_dst = dir.0.join("out.mka");

    let cancel = fvid_control::CancelFlag::new();
    cancel.cancel();

    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        max_packet_bytes: 0,
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_pkt_opts = CopyOptions {
        max_packet_bytes: 0,
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_cnt_opts = CopyOptions {
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_mem_opts = CopyOptions {
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };

    // 1. remux_adts_aac_with_options and concat_adts_aac_with_options must check
    // max_packets == Some(0) BEFORE max_controlled_bytes == Some(0).
    let remux_adts_cnt_err = fvid::native_export::remux_adts_aac_with_options(
        &adts_src,
        &mp4_dst,
        &zero_cnt_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        remux_adts_cnt_err.contains("packet work limit requires at least one packet"),
        "expected packet work limit before controlled memory limit in remux_adts_aac_with_options, got: {remux_adts_cnt_err}"
    );

    let concat_adts_cnt_err = fvid::native_export::concat_adts_aac_with_options(
        &[adts_src.clone(), adts_src.clone()],
        &mp4_dst,
        &zero_cnt_opts,
    )
    .unwrap_err()
    .to_string();
    assert!(
        concat_adts_cnt_err.contains("packet work limit requires at least one packet"),
        "expected packet work limit before controlled memory limit in concat_adts_aac_with_options, got: {concat_adts_cnt_err}"
    );

    // 2. fvid::media::remux and fvid::media::plan_remux on MP4 relocation and Matroska copy
    // must enforce canonical order: cancel -> max_packet_bytes == 0 -> max_packets == Some(0) -> max_controlled_bytes == Some(0).
    for (opts, expected) in [
        (&cancelled_opts, "media operation cancelled"),
        (&zero_pkt_opts, "packet byte limit must be positive"),
        (
            &zero_cnt_opts,
            "packet work limit requires at least one packet",
        ),
        (&zero_mem_opts, "controlled memory limit must be positive"),
    ] {
        for err in [
            fvid::media::remux(&mp4_src, &mp4_dst, opts).unwrap_err(),
            fvid::media::plan_remux(&mp4_src, opts).unwrap_err(),
            fvid::media::remux(&mka_src, &mka_dst, opts).unwrap_err(),
            fvid::media::plan_remux(&mka_src, opts).unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` for {opts:?}, got: {err}"
            );
        }
    }
}

#[test]
fn decode_audio_to_f32le_and_loudnorm_try_plan_preserve_rss_and_canonical_option_order() {
    let dir = TempDir(
        std::env::temp_dir().join(format!("fvid-batch-373-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();

    let wav_src = dir.0.join("src.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    let mp4_src = source("small.m4a");
    let mix_dst = dir.0.join("mix.wav");
    let merge_dst = dir.0.join("merge.wav");

    // 1. Mixing, merging, or concatenating G.711 RIFF/WAVE inputs (1 byte per channel frame, so a
    // stereo G.711 frame is 2 bytes, whereas its decoded float32 WAVE spool frame is 8 bytes) with
    // max_packet_bytes = 2 (and either default or bounded max_controlled_bytes) must succeed,
    // because decode_source_to_f32le_with_options must not apply the source's encoded packet byte
    // limit (2 bytes) to the internal float32 WAVE spool read!
    let mut g711_wav = Vec::new();
    let g711_data = [0xffu8; 64]; // 32 stereo mu-law frames
    g711_wav.extend_from_slice(b"RIFF");
    g711_wav.extend_from_slice(&(38u32 + g711_data.len() as u32).to_le_bytes());
    g711_wav.extend_from_slice(b"WAVEfmt ");
    g711_wav.extend_from_slice(&18u32.to_le_bytes());
    g711_wav.extend_from_slice(&7u16.to_le_bytes()); // mu-law
    g711_wav.extend_from_slice(&2u16.to_le_bytes()); // 2 channels
    g711_wav.extend_from_slice(&8000u32.to_le_bytes()); // 8000 Hz
    g711_wav.extend_from_slice(&16000u32.to_le_bytes()); // byte rate
    g711_wav.extend_from_slice(&2u16.to_le_bytes()); // block align = 2 bytes
    g711_wav.extend_from_slice(&8u16.to_le_bytes()); // 8 bits per sample
    g711_wav.extend_from_slice(&0u16.to_le_bytes()); // cbSize = 0
    g711_wav.extend_from_slice(b"data");
    g711_wav.extend_from_slice(&(g711_data.len() as u32).to_le_bytes());
    g711_wav.extend_from_slice(&g711_data);
    let g711_src = dir.0.join("g711-stereo.wav");
    std::fs::write(&g711_src, &g711_wav).unwrap();

    let mut g711_mix_opts = CopyOptions::default();
    g711_mix_opts.max_packet_bytes = 2; // 1 stereo G.711 frame (2 bytes) < 1 stereo f32 frame (8 bytes)
    let mix_stats = fvid::media::mix_audio_with_controls(
        &[g711_src.clone(), g711_src.clone()],
        &mix_dst,
        &fvid::media::MixAudioOptions::default(),
        &g711_mix_opts,
    )
    .expect("mix_audio_with_controls on stereo G.711 WAVE with max_packet_bytes=2 should succeed");
    assert_eq!(mix_stats.sample_frames, 32);
    let _ = std::fs::remove_file(&mix_dst);

    let merge_stats = fvid::media::merge_audio_with_controls(
        &[g711_src.clone(), g711_src.clone()],
        &merge_dst,
        &g711_mix_opts,
    )
    .expect("merge_audio_with_controls on stereo G.711 WAVE with max_packet_bytes=2 should succeed");
    assert_eq!(merge_stats.sample_frames, 32);
    let _ = std::fs::remove_file(&merge_dst);

    let concat_dst = dir.0.join("concat.wav");
    let concat_stats = fvid::native_audio_mix::concat_audio_with_options(
        &[g711_src.clone(), g711_src.clone()],
        &concat_dst,
        &g711_mix_opts,
    )
    .expect("concat_audio_with_options on stereo G.711 WAVE with max_packet_bytes=2 should succeed");
    assert_eq!(concat_stats.sample_frames, 64);
    let _ = std::fs::remove_file(&concat_dst);

    // 3. fvid_media::mix_audio_with_controls, plan_mix_audio_with_controls,
    // merge_audio_with_controls, and plan_merge_audio_with_controls must enforce canonical
    // CopyOptions order:
    // cancel -> max_packet_bytes == 0 -> max_packets == Some(0) -> max_controlled_bytes == Some(0) -> RSS.
    let cancel = fvid_control::CancelFlag::new();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        max_packet_bytes: 0,
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_pkt_opts = CopyOptions {
        max_packet_bytes: 0,
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_cnt_opts = CopyOptions {
        max_packets: Some(0),
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    let zero_mem_opts = CopyOptions {
        max_controlled_bytes: Some(0),
        max_rss_bytes: Some(0),
        ..Default::default()
    };
    for (opts, expected) in [
        (&cancelled_opts, "media operation cancelled"),
        (&zero_pkt_opts, "packet byte limit must be positive"),
        (
            &zero_cnt_opts,
            "packet work limit requires at least one packet",
        ),
        (&zero_mem_opts, "controlled memory limit must be positive"),
    ] {
        for err in [
            fvid_media::mix_audio_with_controls(
                &[wav_src.clone(), wav_src.clone()],
                &mix_dst,
                &fvid_media::MixAudioOptions::default(),
                opts,
            )
            .unwrap_err(),
            fvid_media::plan_mix_audio_with_controls(
                &[wav_src.clone(), wav_src.clone()],
                &fvid_media::MixAudioOptions::default(),
                opts,
            )
            .unwrap_err(),
            fvid_media::merge_audio_with_controls(
                &[wav_src.clone(), wav_src.clone()],
                &merge_dst,
                opts,
            )
            .unwrap_err(),
            fvid_media::plan_merge_audio_with_controls(&[wav_src.clone(), wav_src.clone()], opts)
                .unwrap_err(),
            fvid::media::mix_audio_with_controls(
                &[wav_src.clone(), wav_src.clone()],
                &mix_dst,
                &fvid::media::MixAudioOptions::default(),
                opts,
            )
            .unwrap_err(),
            fvid::media::plan_mix_audio_with_controls(
                &[wav_src.clone(), wav_src.clone()],
                &fvid::media::MixAudioOptions::default(),
                opts,
            )
            .unwrap_err(),
            fvid::media::merge_audio_with_controls(
                &[wav_src.clone(), wav_src.clone()],
                &merge_dst,
                opts,
            )
            .unwrap_err(),
            fvid::media::plan_merge_audio_with_controls(&[wav_src.clone(), wav_src.clone()], opts)
                .unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` for {opts:?}, got: {err}"
            );
        }
    }
    let _ = mp4_src;
}

#[test]
fn mp4_and_matroska_audio_export_supports_and_admission_preserve_budget_and_packet_policies() {
    let mp4_aac = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-native-edit.m4a");
    let mp4_pcm = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/pcm-screen.mov");
    let mp4_changing = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-export-transitions/changes.mp4");
    let mka_aac = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-stereo.mka");
    let mka_alac = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-resample-window.mka");
    let mkv_pcm = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/pcm-int.mkv");

    // 1. When `max_controlled_bytes` is large enough for `geometry_for_decode` (open + descriptor
    // with `target = None`, where `output_channels` defaults to `first.channels = 2`) but smaller
    // than `admit_audio_reader_with_interval` with `target = Some((96_000, 8, mask))` (where an
    // 8-channel, 96 kHz transform increases normalization ring/buffer memory), `plan_decode_audio`
    // and `decode_audio_transformed` must both enforce the normalization budget.
    let probe_min = CopyOptions {
        max_controlled_bytes: Some(64 * 1024),
        ..Default::default()
    };
    let min_changing_err = fvid_media::plan_decode_audio(
        &mp4_changing,
        &fvid_media_info::AudioDecodeTransform::default(),
        &probe_min,
    )
    .unwrap_err();
    let changing_base_need: usize = min_changing_err
        .split("need ")
        .nth(1)
        .and_then(|s| s.split(" bytes").next())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("expected exact `need <N> bytes` in controlled memory budget error, got: {min_changing_err}"));
    let tight_for_normalized = CopyOptions {
        max_controlled_bytes: Some(changing_base_need + 1024),
        ..Default::default()
    };
    fvid_media::plan_decode_audio(
        &mp4_changing,
        &fvid_media_info::AudioDecodeTransform::default(),
        &tight_for_normalized,
    )
    .expect("default changes.mp4 decode plan should fit within tight_for_normalized");

    let normalized_8ch_transform = fvid_media_info::AudioDecodeTransform {
        sample_rate: Some(96_000),
        channels: Some(8),
        ..Default::default()
    };
    let plan_err = fvid_media::plan_decode_audio(
        &mp4_changing,
        &normalized_8ch_transform,
        &tight_for_normalized,
    )
    .unwrap_err();
    assert!(
        plan_err.contains("controlled memory budget exceeded"),
        "expected plan_decode_audio to enforce normalization memory budget, got: {plan_err}"
    );
    let dir = TempDir::new("mp4-matroska-supports-budget");
    let out_wav = dir.0.join("out.wav");
    let err = fvid_media::decode_audio_transformed(
        &mp4_changing,
        &out_wav,
        normalized_8ch_transform,
        &tight_for_normalized,
    )
    .unwrap_err();
    assert!(
        err.contains("controlled memory budget exceeded"),
        "expected controlled memory budget error, got: {err}"
    );

    // 2. When `max_controlled_bytes` is `None` and `max_packet_bytes` is 0 or smaller than an active
    // MP4 audio packet, `decode_mp4_audio_pcm` and `decode_mp4_pcm_f64` must enforce
    // `max_packet_bytes` and canonical `CopyOptions` checks at admission time.
    let zero_pkt = CopyOptions {
        max_packet_bytes: 0,
        max_controlled_bytes: None,
        ..Default::default()
    };
    let err = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        std::io::BufReader::new(std::fs::File::open(&mp4_aac).unwrap()),
        &mut std::io::sink(),
        None,
        &zero_pkt,
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("packet byte limit must be positive"),
        "expected packet byte limit must be positive, got: {err}"
    );
    let err = fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
        std::io::BufReader::new(std::fs::File::open(&mp4_pcm).unwrap()),
        &mut std::io::sink(),
        None,
        &zero_pkt,
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("packet byte limit must be positive"),
        "expected packet byte limit must be positive for f64 PCM, got: {err}"
    );

    for mka in [&mka_aac, &mka_alac, &mkv_pcm] {
        let tiny_mem = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        let err = fvid_media::decode_audio_transformed(
            mka,
            &out_wav,
            fvid_media_info::AudioDecodeTransform::default(),
            &tiny_mem,
        )
        .unwrap_err();
        assert!(
            err.contains("controlled memory budget exceeded"),
            "expected controlled memory budget exceeded for {}, got: {err}",
            mka.display()
        );
    }
}

#[test]
fn raw_flac_and_mp3_plan_decode_and_standalone_loudness_admission_enforce_pcm_and_copy_policies() {
    let flac_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-index-growth/frames-129.flac");
    let mp3_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mp3/tone.mp3");

    // 1. Raw FLAC: when `max_controlled_bytes` is large enough for `Flac::parse_owned_controlled`
    // (`file_bytes + DECODE_ADMISSION_BYTES + index_bytes`) but smaller than the decoded PCM buffer
    // budget (`+ max_pcm_samples * 4`), `plan_decode_audio` must reject with
    // `controlled memory budget exceeded` rather than succeeding at plan time and failing only in
    // `decode_raw_flac_pcm`.
    let flac_bytes = std::fs::read(&flac_src).unwrap();
    let flac_parsed =
        fvid_media::owned_flac::Flac::parse(&flac_bytes, &Default::default()).unwrap();
    let flac_index_bytes = flac_parsed.frames.capacity()
        * std::mem::size_of::<fvid_media::owned_flac::Frame>();
    let flac_tight_budget = flac_bytes.len() + 32 * 1024 * 1024 + flac_index_bytes;
    let flac_opts = CopyOptions {
        max_controlled_bytes: Some(flac_tight_budget),
        ..Default::default()
    };
    let flac_plan_err = fvid_media::plan_decode_audio(
        &flac_src,
        &fvid_media_info::AudioDecodeTransform::default(),
        &flac_opts,
    )
    .unwrap_err();
    assert!(
        flac_plan_err.contains("controlled memory budget exceeded"),
        "expected plan_decode_audio on raw FLAC to enforce decoded PCM budget, got: {flac_plan_err}"
    );

    // 2. Raw MP3: when `max_controlled_bytes` is large enough for `Mp3::parse_owned_controlled`
    // (`file_bytes + decode_admission_bytes + parse_index_bytes`) but smaller than the decoded PCM
    // frame budget (`+ max_pcm_samples * 4`), `plan_decode_audio` must reject with
    // `controlled memory budget exceeded`.
    let mp3_bytes = std::fs::read(&mp3_src).unwrap();
    let mp3_parsed =
        fvid_media::owned_mpeg_audio::Mp3::parse_owned(mp3_bytes.clone(), &Default::default())
            .unwrap();
    let mp3_index_bytes =
        mp3_parsed.frames.capacity() * std::mem::size_of::<fvid_media::owned_mpeg_audio::Frame>();
    let mp3_pcm_bytes = mp3_parsed
        .frames
        .iter()
        .map(|f| (f.samples as usize) * usize::from(mp3_parsed.channels) * 4)
        .max()
        .unwrap_or(0);
    let mp3_total_needed = mp3_bytes.len()
        + fvid_codecs::mp3::Decoder::decode_admission_bytes()
        + mp3_index_bytes
        + mp3_pcm_bytes;
    let mp3_tight_opts = CopyOptions {
        max_controlled_bytes: Some(mp3_total_needed - 4),
        ..Default::default()
    };
    let mp3_plan_err = fvid_media::plan_decode_audio(
        &mp3_src,
        &fvid_media_info::AudioDecodeTransform::default(),
        &mp3_tight_opts,
    )
    .unwrap_err();
    assert!(
        mp3_plan_err.contains("controlled memory budget exceeded"),
        "expected plan_decode_audio on raw MP3 to enforce decoded PCM budget, got: {mp3_plan_err}"
    );

    // 3. `recognizes_standalone_audio_with_options` must enforce canonical `CopyOptions` checks
    // (`cancel`, zero limits, `max_rss_bytes`) before inspecting non-G.711 standalone files.
    let cancel = fvid_control::CancelFlag::new();
    cancel.cancel();
    for (opts, expected) in [
        (
            CopyOptions {
                cancel: Some(cancel),
                ..Default::default()
            },
            "media operation cancelled",
        ),
        (
            CopyOptions {
                max_packet_bytes: 0,
                ..Default::default()
            },
            "packet byte limit must be positive",
        ),
        (
            CopyOptions {
                max_packets: Some(0),
                ..Default::default()
            },
            "packet work limit requires at least one packet",
        ),
        (
            CopyOptions {
                max_controlled_bytes: Some(0),
                ..Default::default()
            },
            "controlled memory limit must be positive",
        ),
        (
            CopyOptions {
                max_rss_bytes: Some(1),
                ..Default::default()
            },
            "rss budget exceeded",
        ),
    ] {
        let err = fvid_media::owned_container_loudness::recognizes_standalone_audio_with_options(
            &flac_src, &opts,
        )
        .unwrap_err();
        assert!(
            err.contains(expected),
            "expected `{expected}` for {opts:?}, got: {err}"
        );
    }
}

#[test]
fn matroska_pcm_f64_codec_check_and_stream_selection_enforce_kind_and_cardinality() {
    let aac_mka = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-stereo.mka");
    let alac_mka = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-resample-window.mka");

    // 1. `decode_matroska_pcm_f64` must validate that the selected Matroska audio track is a PCM
    // codec (`A_PCM/INT/LIT`, `A_PCM/INT/BIG`, `A_PCM/FLOAT/IEEE`) before running
    // `admit_audio_reader_with_interval`, matching `decode_matroska_pcm` / `decode_matroska_audio_pcm`.
    let tiny_budget = CopyOptions {
        max_controlled_bytes: Some(4096),
        ..Default::default()
    };
    for mka in [&aac_mka, &alac_mka] {
        let mut sink = std::io::sink();
        let err = fvid_media::owned_matroska_pcm::decode_matroska_pcm_f64(
            std::io::BufReader::new(std::fs::File::open(mka).unwrap()),
            &mut sink,
            None,
            &tiny_budget,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("selected Matroska audio stream has a different codec"),
            "expected codec mismatch error before decoder budget admission for {}, got: {err}",
            mka.display()
        );
    }

    // 2. `matroska_descriptor` / `source_geometry` must reject `options.streams` with >1 stream
    // (`[0, 1]`) instead of silently taking `options.streams.first().copied()`.
    let multi_streams = CopyOptions {
        streams: vec![0, 1],
        ..Default::default()
    };
    let err =
        fvid_media::owned_audio_plan::source_geometry(&aac_mka, &multi_streams).unwrap_err();
    assert!(
        err.contains("select exactly one audio stream"),
        "expected `select exactly one audio stream` for streams=[0, 1], got: {err}"
    );
}

#[test]
fn ogg_opus_plan_decode_and_supports_enforce_wave_metadata_budget_and_nonempty_interval_prefix() {
    let ogg_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-g711-loudnorm-budget/ogg-opus-mono-4byte-pkt.ogg");
    let flac_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/raw-flac-mp3-prefix-budget/prefix-budget.flac");
    let mp3_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/raw-flac-mp3-prefix-budget/prefix-budget.mp3");

    // 1. When `transform.interval` starts at or after the end of an Ogg Opus, Raw FLAC, or Raw MP3
    // program (`Some((10_000_000, 20_000_000))`), `plan_decode_audio` and `supports` must reject
    // with `"no decoded audio samples"` instead of returning `Ok(MediaPlan)` and failing only
    // during `decode_audio_transformed`!
    let past_end_interval_transform = fvid_media_info::AudioDecodeTransform {
        interval: Some((10_000_000, 20_000_000)),
        ..Default::default()
    };
    for (label, src) in [
        ("Ogg Opus", &ogg_src),
        ("Raw FLAC", &flac_src),
        ("Raw MP3", &mp3_src),
    ] {
        let plan_err = fvid_media::plan_decode_audio(
            src,
            &past_end_interval_transform,
            &CopyOptions::default(),
        )
        .unwrap_err();
        assert!(
            plan_err.contains("no decoded audio samples"),
            "expected plan_decode_audio on {label} with interval past end (10s..20s) to reject with `no decoded audio samples`, got: {plan_err}"
        );
    }
}

#[test]
fn mp4_matroska_and_adts_plan_decode_and_supports_enforce_source_metadata_budget_and_nonempty_interval_prefix() {
    let mp4_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-resample-window.m4a");
    let mka_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-resample-window.mka");
    let adts_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-stereo.aac");

    // 1. When `transform.interval` starts at or after the end of an MP4, Matroska, or ADTS
    // audio stream (`Some((10_000_000, 20_000_000))`), `plan_decode_audio` must reject with
    // `"no decoded audio samples"` instead of returning `Ok(MediaPlan)` and failing only during
    // `decode_audio_transformed`.
    let past_end_interval_transform = fvid_media_info::AudioDecodeTransform {
        interval: Some((10_000_000, 20_000_000)),
        ..Default::default()
    };
    for (label, src) in [
        ("MP4 ALAC", &mp4_src),
        ("Matroska ALAC", &mka_src),
        ("ADTS AAC", &adts_src),
    ] {
        let plan_err = fvid_media::plan_decode_audio(
            src,
            &past_end_interval_transform,
            &CopyOptions::default(),
        )
        .unwrap_err();
        assert!(
            plan_err.contains("no decoded audio samples"),
            "expected plan_decode_audio on {label} with interval past end (10s..20s) to reject with `no decoded audio samples`, got: {plan_err}"
        );
    }

    // 2. When `streams: vec![0, 1]` is passed to `mp4_source_info` or `matroska_source_info`
    // (or `source_geometry`), it must reject with `select exactly one audio stream` instead of
    // silently taking `streams.first().copied()`.
    let multi_stream_opts = CopyOptions {
        streams: vec![0, 1],
        ..Default::default()
    };
    for (label, src) in [("MP4", &mp4_src), ("Matroska", &mka_src)] {
        let geom_err =
            fvid_media::owned_audio_plan::source_geometry(src, &multi_stream_opts).unwrap_err();
        assert!(
            geom_err.contains("select exactly one audio stream"),
            "expected source_geometry on {label} with streams=[0, 1] to reject with `select exactly one audio stream`, got: {geom_err}"
        );
    }
}

#[test]
fn canonical_file_tags_are_charged_in_matroska_metadata_working_set_across_lossless_remux_and_subtitle_routes() {
    let dir = TempDir::new("batch379-canonical-file-tags");
    let mkv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/multitrack-lossless-budget-parity/multitrack-ffv1-aac.mkv");
    let mp4 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/multitrack-lossless-budget-parity/multitrack-avc-aac.mp4");
    let adts = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-scaled-clock/warm-third.aac");

    let canonical_tags = [
        ("description", "x".repeat(400)),
        ("genre", "y".repeat(400)),
        ("publisher", "z".repeat(400)),
        ("copyright", "c".repeat(400)),
        ("album_artist", "a".repeat(400)),
        ("track", "1/12".to_string()),
        ("disc", "1/2".to_string()),
        ("rating", "5.0".to_string()),
        ("date", "2026-10-26".to_string()),
    ];

    // 1. Lossless transcode (`plan_transcode_lossless` & `transcode_lossless`) under 4096-byte budget:
    let mut lossless_opts = CopyOptions::default();
    lossless_opts.max_controlled_bytes = Some(4096);
    for (k, v) in &canonical_tags {
        lossless_opts
            .metadata_set
            .push(((*k).to_string(), v.clone()));
    }
    let lossless_plan_err = fvid::media::plan_transcode_lossless(
        &mkv,
        &fvid::media::LosslessTransform::default(),
        &lossless_opts,
        None,
    )
    .expect_err("plan_transcode_lossless must charge all 13 canonical FileTags fields");
    assert_eq!(
        lossless_plan_err,
        "Matroska metadata working set exceeds controlled memory budget"
    );
    let out_lossless = dir.0.join("out-lossless.mkv");
    let lossless_exec_err = fvid::media::transcode_lossless(
        &mkv,
        &out_lossless,
        fvid::media::LosslessTransform::default(),
        &lossless_opts,
    )
    .expect_err("transcode_lossless must charge all 13 canonical FileTags fields");
    assert_eq!(lossless_exec_err, lossless_plan_err);
    assert!(!out_lossless.exists());

    // 2. MP4 -> Matroska remux (`plan_remux` & `remux`) under 20_000-byte budget:
    let mut remux_opts = CopyOptions::default();
    remux_opts.max_controlled_bytes = Some(20_000);
    for (k, v) in &canonical_tags {
        remux_opts.metadata_set.push(((*k).to_string(), v.clone()));
    }
    let out_mp4_mkv = dir.0.join("out-mp4-remux.mkv");
    let mp4_plan_err = fvid::media::plan_remux(&mp4, &remux_opts)
        .expect_err("MP4->MKV plan_remux must charge all 13 canonical FileTags fields")
        .to_string();
    assert!(
        mp4_plan_err.contains("controlled memory budget exceeded"),
        "unexpected mp4 plan_remux err: {mp4_plan_err}"
    );
    let mp4_exec_err = fvid::media::remux(&mp4, &out_mp4_mkv, &remux_opts)
        .expect_err("MP4->MKV remux must charge all 13 canonical FileTags fields")
        .to_string();
    assert!(
        mp4_exec_err.contains("controlled memory budget exceeded"),
        "unexpected mp4 remux err: {mp4_exec_err}"
    );
    assert!(!out_mp4_mkv.exists());

    // 3. ADTS -> Matroska remux (`plan_remux` & `remux`) under 20_000-byte budget:
    let out_adts_mka = dir.0.join("out-adts.mka");
    let adts_plan_err = fvid::media::plan_remux(&adts, &remux_opts)
        .expect_err("ADTS->MKA plan_remux must charge all 13 canonical FileTags fields")
        .to_string();
    assert!(
        adts_plan_err.contains("controlled memory budget exceeded"),
        "unexpected adts plan_remux err: {adts_plan_err}"
    );
    let adts_exec_err = fvid::media::remux(&adts, &out_adts_mka, &remux_opts)
        .expect_err("ADTS->MKA remux must charge all 13 canonical FileTags fields")
        .to_string();
    assert!(
        adts_exec_err.contains("controlled memory budget exceeded"),
        "unexpected adts remux err: {adts_exec_err}"
    );
    assert!(!out_adts_mka.exists());

    // 4. SRT -> Matroska subtitle conversion (`plan_convert_subtitles_for_destination` & `convert_subtitles_with_controls`)
    //    under 20_000-byte budget:
    let srt_path = dir.0.join("sample.srt");
    std::fs::write(
        &srt_path,
        b"1\n00:00:00,000 --> 00:00:01,000\nHello world\n",
    )
    .unwrap();
    let out_sub_mkv = dir.0.join("out-sub.mkv");
    let sub_opts = fvid::media::SubtitleConvertOptions::default();
    let sub_plan_err = fvid::media::plan_convert_subtitles_for_destination(
        &srt_path,
        Some(&out_sub_mkv),
        &sub_opts,
        &remux_opts,
    )
    .expect_err("subtitle plan must charge all 13 canonical FileTags fields")
    .to_string();
    assert!(
        sub_plan_err.contains("controlled memory budget exceeded"),
        "unexpected sub plan err: {sub_plan_err}"
    );
    let sub_exec_err = fvid::media::convert_subtitles_with_controls(
        &srt_path,
        &out_sub_mkv,
        &sub_opts,
        &remux_opts,
    )
    .expect_err("subtitle convert must charge all 13 canonical FileTags fields")
    .to_string();
    assert!(
        sub_exec_err.contains("controlled memory budget exceeded"),
        "unexpected sub exec err: {sub_exec_err}"
    );
    assert!(!out_sub_mkv.exists());
}

#[test]
fn matroska_probe_copy_and_subtitle_conversion_enforce_estimated_index_payload_budget() {
    let dir = TempDir::new("batch380-matroska-index-payload-budget");
    let adts = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-scaled-clock/warm-third.aac");

    // 1. Create a valid Matroska audio file with 3 small ADTS packets and 6 container tags,
    // so `reader.packets.len()` is only 3 (`3 * 56 = 168` bytes <= 4096), while
    // `reader.estimated_index_payload_bytes()` is > 28 KiB (> 4096 bytes).
    let mut seed_opts = CopyOptions::default();
    for i in 0..6 {
        seed_opts
            .metadata_set
            .push((format!("CUSTOM_TAG_{i}"), "v".repeat(500)));
    }
    let mka_tagged = dir.0.join("tagged.mka");
    fvid::media::remux(&adts, &mka_tagged, &seed_opts).unwrap();

    // Under `max_controlled_bytes = Some(4096)`:
    // - `4096 / size_of::<Packet>()` is `73 >= 3`, so packet count limit alone does not trigger.
    // - `estimated_index_payload_bytes()` is > 28 KiB > 4096, so `probe_with_options`,
    //   `probe_as_with_options`, `probe_webm_with_options`, and `try_probe_webm_with_packet_and_memory_limit`
    //   must reject with `"controlled memory budget exceeded"`.
    let mem_4096 = CopyOptions {
        max_controlled_bytes: Some(4096),
        ..Default::default()
    };
    for probe_err in [
        fvid::media::probe_with_options(&mka_tagged, &mem_4096).unwrap_err(),
        fvid::media::probe_as_with_options(&mka_tagged, Some("matroska"), &mem_4096).unwrap_err(),
        fvid_media::owned_webm_probe::probe_webm_with_options(&mka_tagged, &mem_4096).unwrap_err(),
        fvid_media::owned_webm_probe::try_probe_webm_with_packet_and_memory_limit(
            &mka_tagged,
            64 << 20,
            Some(4096),
        )
        .unwrap_err(),
    ] {
        assert!(
            probe_err.contains("controlled memory budget exceeded"),
            "expected Matroska probe to enforce estimated_index_payload_bytes under 4096-byte budget, got: {probe_err}"
        );
    }

    // 2. Under `max_controlled_bytes = Some(70_000)` (> 64 KiB + 3 * 16 = 65_584 bytes, so old
    // `64 KiB + packets.len() * 16` check in `owned_matroska_copy` passed!), when a Matroska file
    // carries 6 custom container tags (`estimated_index_payload_bytes()` ~ 37 KiB, so
    // `37 KiB + 64 KiB > 70_000`), `plan_remux` and `remux` (and `owned_matroska_copy`) must reject
    // with `"controlled memory budget exceeded"`.
    let mem_70k = CopyOptions {
        max_controlled_bytes: Some(70_000),
        ..Default::default()
    };
    let out_copy = dir.0.join("out-copy.mka");
    let copy_plan_err = fvid::media::plan_remux(&mka_tagged, &mem_70k)
        .expect_err("Matroska plan_remux must charge estimated_index_payload_bytes in copy budget")
        .to_string();
    assert!(
        copy_plan_err.contains("controlled memory budget exceeded"),
        "unexpected copy plan_remux err: {copy_plan_err}"
    );
    let copy_exec_err = fvid::media::remux(&mka_tagged, &out_copy, &mem_70k)
        .expect_err("Matroska remux must charge estimated_index_payload_bytes in copy budget")
        .to_string();
    assert!(
        copy_exec_err.contains("controlled memory budget exceeded"),
        "unexpected copy remux err: {copy_exec_err}"
    );
    assert!(!out_copy.exists());

    // 3. Create a Matroska subtitle (.mkv) file with 1 cue and 6 custom tags:
    // Under `max_controlled_bytes = Some(20_000)` (`1 * 64 + 16_384 = 16_448 <= 20_000`, so old
    // `packets.len() * 64 + 16384` check in `owned_subtitle_impl` passed!), when we delete the tags
    // via `metadata_delete` in the conversion options (so output `meta_working_bytes` is 0, while
    // source `reader.estimated_index_payload_bytes()` is > 28 KiB > 20_000), both
    // `plan_convert_subtitles_for_destination` and `convert_subtitles_with_controls` must reject
    // with `"controlled memory budget exceeded"`.
    let srt_path = dir.0.join("one.srt");
    std::fs::write(
        &srt_path,
        b"1\n00:00:00,000 --> 00:00:01,000\nSubtitle line\n",
    )
    .unwrap();
    let mkv_sub_tagged = dir.0.join("sub-tagged.mkv");
    let sub_opts = fvid::media::SubtitleConvertOptions::default();
    fvid::media::convert_subtitles_with_controls(
        &srt_path,
        &mkv_sub_tagged,
        &sub_opts,
        &seed_opts,
    )
    .unwrap();

    let mut sub_budget_opts = CopyOptions {
        max_controlled_bytes: Some(20_000),
        ..Default::default()
    };
    for i in 0..6 {
        sub_budget_opts
            .metadata_delete
            .push(format!("CUSTOM_TAG_{i}"));
    }
    let out_sub_mkv = dir.0.join("out-sub-from-mkv.mkv");
    let sub_mkv_plan_err = fvid::media::plan_convert_subtitles_for_destination(
        &mkv_sub_tagged,
        Some(&out_sub_mkv),
        &sub_opts,
        &sub_budget_opts,
    )
    .expect_err("Matroska subtitle plan must enforce estimated_index_payload_bytes")
    .to_string();
    assert!(
        sub_mkv_plan_err.contains("controlled memory budget exceeded"),
        "unexpected sub mkv plan err: {sub_mkv_plan_err}"
    );
    let sub_mkv_exec_err = fvid::media::convert_subtitles_with_controls(
        &mkv_sub_tagged,
        &out_sub_mkv,
        &sub_opts,
        &sub_budget_opts,
    )
    .expect_err("Matroska subtitle convert must enforce estimated_index_payload_bytes")
    .to_string();
    assert!(
        sub_mkv_exec_err.contains("controlled memory budget exceeded"),
        "unexpected sub mkv exec err: {sub_mkv_exec_err}"
    );
    assert!(!out_sub_mkv.exists());
}

#[test]
fn matroska_lossless_multitrack_video_decode_and_subtitle_burn_enforce_estimated_index_payload_budget() {
    let dir = TempDir::new("matroska-lossless-multitrack-burn-index-payload-budget");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors");

    let mut seed_opts = CopyOptions::default();
    for i in 0..8 {
        seed_opts
            .metadata_set
            .push((format!("CUSTOM_TAG_{i}"), "v".repeat(500)));
    }

    // 1. Single-track FFV1 MKV with 8 custom container tags (`estimated_index_payload_bytes()` > 44 KiB).
    let y4m_src = root.join("subtitle-burn.y4m");
    let srt_src = root.join("subtitle-burn.srt");
    let single_tagged = dir.0.join("single-tagged.mkv");
    fvid_media::transcode_lossless(
        &y4m_src,
        &single_tagged,
        fvid_media::LosslessTransform::default(),
        &seed_opts,
    )
    .unwrap();

    // 2. Multitrack FFV1+AAC MKV with 8 custom container tags (`estimated_index_payload_bytes()` > 48 KiB).
    let multi_src = root.join("multitrack-lossless-budget-parity/multitrack-ffv1-aac.mkv");
    let multi_tagged = dir.0.join("multi-tagged.mkv");
    fvid_media::transcode_lossless(
        &multi_src,
        &multi_tagged,
        fvid_media::LosslessTransform::default(),
        &seed_opts,
    )
    .unwrap();

    // Delete all 8 custom tags in the export options so that output `estimate_matroska_metadata_working_bytes`
    // is near zero, while the retained source `WebmReader` index (`estimated_index_payload_bytes()`)
    // is > 44 KiB (> 40_000 bytes). A 40_000-byte budget is large enough for 96x64 FFV1 decoding/encoding
    // and SRT mask working sets (~25 KiB), so only `estimated_index_payload_bytes()` exceeds 40_000.
    let mut budget_opts = CopyOptions {
        max_controlled_bytes: Some(40_000),
        ..Default::default()
    };
    for i in 0..8 {
        budget_opts
            .metadata_delete
            .push(format!("CUSTOM_TAG_{i}"));
    }

    let out_single = dir.0.join("out-single.mkv");
    let single_plan_err = fvid_media::plan_transcode_lossless(
        &single_tagged,
        &fvid_media::LosslessTransform::default(),
        &budget_opts,
        None,
    )
    .expect_err("single-track Matroska plan_transcode_lossless must enforce estimated_index_payload_bytes");
    assert!(
        single_plan_err.contains("controlled memory budget exceeded"),
        "unexpected single_plan_err: {single_plan_err}"
    );
    let single_exec_err = fvid_media::transcode_lossless(
        &single_tagged,
        &out_single,
        fvid_media::LosslessTransform::default(),
        &budget_opts,
    )
    .expect_err("single-track Matroska transcode_lossless must enforce estimated_index_payload_bytes");
    assert!(
        single_exec_err.contains("controlled memory budget exceeded"),
        "unexpected single_exec_err: {single_exec_err}"
    );
    assert!(!out_single.exists());

    let out_multi = dir.0.join("out-multi.mkv");
    let multi_plan_err = fvid_media::plan_transcode_lossless(
        &multi_tagged,
        &fvid_media::LosslessTransform::default(),
        &budget_opts,
        None,
    )
    .expect_err("multitrack Matroska plan_transcode_lossless must enforce estimated_index_payload_bytes");
    assert!(
        multi_plan_err.contains("controlled memory budget exceeded"),
        "unexpected multi_plan_err: {multi_plan_err}"
    );
    let multi_exec_err = fvid_media::transcode_lossless(
        &multi_tagged,
        &out_multi,
        fvid_media::LosslessTransform::default(),
        &budget_opts,
    )
    .expect_err("multitrack Matroska transcode_lossless must enforce estimated_index_payload_bytes");
    assert!(
        multi_exec_err.contains("controlled memory budget exceeded"),
        "unexpected multi_exec_err: {multi_exec_err}"
    );
    assert!(!out_multi.exists());

    let out_burn = dir.0.join("out-burn.mkv");
    let burn_plan_err = fvid_media::plan_burn_subtitles(&single_tagged, &srt_src, &budget_opts)
        .expect_err("Matroska plan_burn_subtitles must enforce estimated_index_payload_bytes");
    assert!(
        burn_plan_err.contains("controlled memory budget exceeded"),
        "unexpected burn_plan_err: {burn_plan_err}"
    );
    let burn_exec_err =
        fvid_media::burn_subtitles(&single_tagged, &out_burn, &srt_src, &budget_opts)
            .expect_err("Matroska burn_subtitles must enforce estimated_index_payload_bytes");
    assert!(
        burn_exec_err.contains("controlled memory budget exceeded"),
        "unexpected burn_exec_err: {burn_exec_err}"
    );
    assert!(!out_burn.exists());
}

#[test]
fn mp4_probe_remux_concat_video_decode_and_native_source_info_enforce_estimated_index_payload_budget() {
    let dir = TempDir::new("batch-382-mp4-index-payload");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let small_m4a = source("small.m4a");
    let multi_mp4 = root.join("multitrack-lossless-budget-parity/multitrack-avc-aac.mp4");
    let avc_mp4 = root.join("avc-field-b-implicit-10bit-top-first-bi-cabac-init0-filter0.mp4");

    // 1. `small.m4a` has a 508-byte `moov` payload, while `Mp4Reader::estimated_index_payload_bytes()`
    // is > 600 bytes (`Track` + `SampleDescription` + `Sample` + `configuration`).
    // Under `max_controlled_bytes = Some(550)`, `moov` fits in `metadata_bytes`, and all packets fit,
    // so `probe_with_options`, `owned_probe::probe_with_options`, `is_aac_source_with_metadata_limit`,
    // `aac_source_info_selected_with_options`, and `audio_source_info_selected_with_options` must
    // enforce `reader.estimated_index_payload_bytes() <= limit`.
    let small_opts = CopyOptions {
        max_controlled_bytes: Some(550),
        ..Default::default()
    };
    let probe_err = fvid::media::probe_with_options(&small_m4a, &small_opts)
        .expect_err("fvid::media::probe_with_options on small.m4a must enforce estimated_index_payload_bytes");
    assert!(
        probe_err.to_string().contains("controlled memory budget exceeded"),
        "unexpected probe_err: {probe_err}"
    );
    let owned_probe_err = fvid_media::owned_probe::probe_with_options(&small_m4a, &small_opts)
        .expect_err("owned_probe::probe_with_options on small.m4a must enforce estimated_index_payload_bytes");
    assert!(
        owned_probe_err.contains("controlled memory budget exceeded"),
        "unexpected owned_probe_err: {owned_probe_err}"
    );
    let is_aac_err =
        fvid::native_media::is_aac_source_with_metadata_limit(&small_m4a, Some(550))
            .expect_err("is_aac_source_with_metadata_limit on small.m4a must enforce estimated_index_payload_bytes");
    assert!(
        is_aac_err.to_string().contains("controlled memory budget exceeded"),
        "unexpected is_aac_err: {is_aac_err}"
    );
    let aac_info_err = fvid::native_media::aac_source_info_selected_with_options(
        &small_m4a,
        None,
        None,
        &small_opts,
    )
    .expect_err("aac_source_info_selected_with_options on small.m4a must enforce estimated_index_payload_bytes");
    assert!(
        aac_info_err.to_string().contains("controlled memory budget exceeded"),
        "unexpected aac_info_err: {aac_info_err}"
    );
    let Err(audio_info_err) = fvid::native_media::audio_source_info_selected_with_options(
        &small_m4a,
        None,
        &small_opts,
    ) else {
        panic!("audio_source_info_selected_with_options on small.m4a must enforce estimated_index_payload_bytes");
    };
    assert!(
        audio_info_err.to_string().contains("controlled memory budget exceeded"),
        "unexpected audio_info_err: {audio_info_err}"
    );

    // 2. `multitrack-avc-aac.mp4` has a 1096-byte `moov` payload, while `Mp4Reader::estimated_index_payload_bytes()`
    // across its two tracks (`vide` + `soun`) is > 1600 bytes.
    // Under `max_controlled_bytes = Some(1300)` (for multitrack lossless export) and `Some(17_500)`
    // (for MP4->Matroska remux where `16_384 + packet_plan_bytes` < `17_500` < `16_384 + estimated_index_payload_bytes()`),
    // and `Some(34_000)` (for 2-segment MP4 concat where `16_384 + 2*8_192 + packet_plan_bytes` < `34_000`
    // < `16_384 + 2*8_192 + 2*estimated_index_payload_bytes()`), each route must enforce `estimated_index_payload_bytes()`.
    let multi_lossless_opts = CopyOptions {
        max_controlled_bytes: Some(1300),
        ..Default::default()
    };
    let out_multi = dir.0.join("out-multi.mkv");
    let multi_plan_err = fvid_media::plan_transcode_lossless(
        &multi_mp4,
        &fvid_media::LosslessTransform::default(),
        &multi_lossless_opts,
        None,
    )
    .expect_err("multitrack MP4 plan_transcode_lossless must enforce estimated_index_payload_bytes");
    assert!(
        multi_plan_err.contains("controlled memory budget exceeded"),
        "unexpected multi_plan_err: {multi_plan_err}"
    );
    let multi_exec_err = fvid_media::transcode_lossless(
        &multi_mp4,
        &out_multi,
        fvid_media::LosslessTransform::default(),
        &multi_lossless_opts,
    )
    .expect_err("multitrack MP4 transcode_lossless must enforce estimated_index_payload_bytes");
    assert!(
        multi_exec_err.contains("controlled memory budget exceeded"),
        "unexpected multi_exec_err: {multi_exec_err}"
    );
    assert!(!out_multi.exists());

    let remux_opts = CopyOptions {
        max_controlled_bytes: Some(17_500),
        ..Default::default()
    };
    let out_remux = dir.0.join("out-remux.mkv");
    let remux_plan_err = fvid_media::plan_remux(&multi_mp4, &remux_opts)
        .expect_err("MP4 plan_remux must charge estimated_index_payload_bytes");
    assert!(
        remux_plan_err.contains("controlled memory budget exceeded"),
        "unexpected remux_plan_err: {remux_plan_err}"
    );
    let remux_exec_err = fvid_media::remux(&multi_mp4, &out_remux, &remux_opts)
        .expect_err("MP4 remux must charge estimated_index_payload_bytes");
    assert!(
        remux_exec_err.contains("controlled memory budget exceeded"),
        "unexpected remux_exec_err: {remux_exec_err}"
    );
    assert!(!out_remux.exists());

    let concat_opts = CopyOptions {
        max_controlled_bytes: Some(34_000),
        ..Default::default()
    };
    let concat_inputs = vec![multi_mp4.clone(), multi_mp4.clone()];
    let Err(concat_err) = fvid::container::mp4_concat::open_with_options(
        &concat_inputs,
        &concat_opts,
    ) else {
        panic!("mp4_concat::open_with_options must charge segment.reader.estimated_index_payload_bytes()");
    };
    assert!(
        concat_err.to_string().contains("controlled memory budget exceeded"),
        "unexpected concat_err: {concat_err}"
    );

    // 3. `avc-field-b-implicit-10bit-top-first-bi-cabac-init0-filter0.mp4` has a 700-byte `moov` payload
    // while `Mp4Reader::estimated_index_payload_bytes()` is > 900 bytes.
    // Under `max_controlled_bytes = Some(800)`, single-track MP4 video lossless export (`owned_mp4_video_bridge`
    // and `owned_mp4_video_decode`) must enforce `estimated_index_payload_bytes()`.
    let avc_opts = CopyOptions {
        max_controlled_bytes: Some(800),
        ..Default::default()
    };
    let out_avc = dir.0.join("out-avc.mkv");
    let avc_plan_err = fvid_media::plan_transcode_lossless(
        &avc_mp4,
        &fvid_media::LosslessTransform::default(),
        &avc_opts,
        None,
    )
    .expect_err("single-track MP4 plan_transcode_lossless must enforce estimated_index_payload_bytes");
    assert!(
        avc_plan_err.contains("controlled memory budget exceeded"),
        "unexpected avc_plan_err: {avc_plan_err}"
    );
    let avc_exec_err = fvid_media::transcode_lossless(
        &avc_mp4,
        &out_avc,
        fvid_media::LosslessTransform::default(),
        &avc_opts,
    )
    .expect_err("single-track MP4 transcode_lossless must enforce estimated_index_payload_bytes");
    assert!(
        avc_exec_err.contains("controlled memory budget exceeded"),
        "unexpected avc_exec_err: {avc_exec_err}"
    );
    assert!(!out_avc.exists());
}

#[test]
fn audio_mix_simultaneous_pcm_buffers_and_native_plan_export_index_payload_budget_are_enforced() {
    let dir = TempDir(std::env::temp_dir().join(format!(
        "fvid-audio-mix-and-native-index-budget-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();

    // 1. Two stereo float32 WAVE inputs of 960 frames each (7,680 PCM bytes per file).
    // Each input individually satisfies `size = 7_680 <= 16_000` (and `estimate_float_wave_export_bytes`
    // is `7_680 <= 16_000`), and `out_bytes` alone is `7_680 <= 16_000` for mix and `15_360 <= 16_000`
    // for merge. However, `owned_audio_mix` holds all decoded input `Vec<u8>` buffers simultaneously
    // with the output buffer (`7_680 + 7_680 + 7_680 = 23_040 > 16_000` for mix; and `7_680 + 7_680
    // + 7_680 + 7_680 + 15_360 = 46_080 > 16_000` for merge).
    let wav_a = dir.0.join("a.wav");
    let wav_b = dir.0.join("b.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_a, 48_000, 2, &[0.1; 1920]).unwrap();
    fvid_media::owned_wav_file::write_wav_f32le(&wav_b, 48_000, 2, &[0.2; 1920]).unwrap();
    let wav_sources = vec![wav_a.clone(), wav_b.clone()];
    let mix_opts = CopyOptions {
        max_controlled_bytes: Some(16_000),
        ..Default::default()
    };
    let out_mix = dir.0.join("mixed.wav");
    let out_merge = dir.0.join("merged.wav");

    let plan_mix_err = fvid_media::plan_mix_audio_with_controls(
        &wav_sources,
        &fvid_media::MixAudioOptions::default(),
        &mix_opts,
    )
    .expect_err("plan_mix_audio_with_controls must charge simultaneous input PCM buffers + output buffer");
    assert!(
        plan_mix_err.contains("controlled memory budget exceeded"),
        "unexpected plan_mix_err: {plan_mix_err}"
    );
    let exec_mix_err = fvid_media::mix_audio_with_controls(
        &wav_sources,
        &out_mix,
        &fvid_media::MixAudioOptions::default(),
        &mix_opts,
    )
    .expect_err("mix_audio_with_controls must charge simultaneous input PCM buffers + output buffer");
    assert!(
        exec_mix_err.contains("controlled memory budget exceeded"),
        "unexpected exec_mix_err: {exec_mix_err}"
    );
    assert!(!out_mix.exists());

    let plan_merge_err = fvid_media::plan_merge_audio_with_controls(&wav_sources, &mix_opts)
        .expect_err("plan_merge_audio_with_controls must charge simultaneous input PCM + channel-vector + output buffers");
    assert!(
        plan_merge_err.contains("controlled memory budget exceeded"),
        "unexpected plan_merge_err: {plan_merge_err}"
    );
    let exec_merge_err = fvid_media::merge_audio_with_controls(&wav_sources, &out_merge, &mix_opts)
        .expect_err("merge_audio_with_controls must charge simultaneous input PCM + channel-vector + output buffers");
    assert!(
        exec_merge_err.contains("controlled memory budget exceeded"),
        "unexpected exec_merge_err: {exec_merge_err}"
    );
    assert!(!out_merge.exists());

    // Sufficient budget (`48_000` bytes) succeeds for both mix and merge planning and execution.
    let ok_mix_opts = CopyOptions {
        max_controlled_bytes: Some(48_000),
        ..Default::default()
    };
    fvid_media::plan_mix_audio_with_controls(
        &wav_sources,
        &fvid_media::MixAudioOptions::default(),
        &ok_mix_opts,
    )
    .expect("plan_mix_audio_with_controls must succeed when simultaneous working set fits");
    fvid_media::plan_merge_audio_with_controls(&wav_sources, &ok_mix_opts)
        .expect("plan_merge_audio_with_controls must succeed when simultaneous working set fits");

    // 2. `native_plan::remux_with_options`, `native_export::pcm_matroska::extract_source_pcm_metadata_with_interval_packet_and_memory_limit`
    // (via `native_plan::normalize_loudness_with_options`), and `native_audio_mix::validate_concat_segments_with_options`
    // (with `max_packets` set so `segment_packet_count` runs) on `small.m4a` under `max_controlled_bytes = Some(550)`.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let small_m4a = source("small.m4a");
    let small_opts = CopyOptions {
        max_controlled_bytes: Some(550),
        ..Default::default()
    };
    let native_remux_err = fvid::native_plan::remux_with_options(&small_m4a, &small_opts)
        .expect_err("native_plan::remux_with_options on small.m4a must enforce estimated_index_payload_bytes");
    assert!(
        native_remux_err.contains("controlled memory budget exceeded"),
        "unexpected native_remux_err: {native_remux_err}"
    );

    let concat_packet_opts = CopyOptions {
        max_controlled_bytes: Some(550),
        max_packets: Some(2),
        ..Default::default()
    };
    let concat_seg_err = fvid::native_audio_mix::validate_concat_segments_with_options(
        &[small_m4a.clone(), small_m4a.clone()],
        &concat_packet_opts,
    )
    .expect_err("validate_concat_segments_with_options must enforce estimated_index_payload_bytes in segment_packet_count");
    assert!(
        concat_seg_err.to_string().contains("controlled memory budget exceeded"),
        "unexpected concat_seg_err: {concat_seg_err}"
    );

    // 3. `native_plan::export_y4m_with_options`, `native_plan::trim_y4m_with_options`,
    // `native_plan::concat_y4m_with_options`, and `native_export::export_y4m_pipeline_with_options`
    // on `avc-field-b-implicit-10bit-top-first-bi-cabac-init0-filter0.mp4` under `max_controlled_bytes = Some(800)`
    // (where `moov` is 700 bytes and `estimated_index_payload_bytes()` is > 900 bytes).
    let avc_mp4 = root.join(
        "tests/fixtures/playback-errors/avc-field-b-implicit-10bit-top-first-bi-cabac-init0-filter0.mp4",
    );
    let avc_opts = CopyOptions {
        max_controlled_bytes: Some(800),
        ..Default::default()
    };
    let y4m_plan_err = fvid::native_plan::export_y4m_with_options(
        &avc_mp4,
        None,
        &Default::default(),
        &Default::default(),
        &avc_opts,
    )
    .expect_err("native_plan::export_y4m_with_options must enforce estimated_index_payload_bytes");
    assert!(
        y4m_plan_err.contains("controlled memory budget exceeded"),
        "unexpected y4m_plan_err: {y4m_plan_err}"
    );
    let y4m_trim_err = fvid::native_plan::trim_y4m_with_options(&avc_mp4, 0, 100_000, &avc_opts)
        .expect_err("native_plan::trim_y4m_with_options must enforce estimated_index_payload_bytes");
    assert!(
        y4m_trim_err.contains("controlled memory budget exceeded"),
        "unexpected y4m_trim_err: {y4m_trim_err}"
    );
    let y4m_concat_err = fvid::native_plan::concat_y4m_with_options(
        &[avc_mp4.clone(), avc_mp4.clone()],
        &avc_opts,
    )
    .expect_err("native_plan::concat_y4m_with_options must enforce estimated_index_payload_bytes");
    assert!(
        y4m_concat_err.contains("controlled memory budget exceeded"),
        "unexpected y4m_concat_err: {y4m_concat_err}"
    );
    let out_y4m = dir.0.join("out.y4m");
    let y4m_exec_err = fvid::native_export::export_y4m_pipeline_with_options(
        &avc_mp4,
        &out_y4m,
        None,
        &Default::default(),
        &Default::default(),
        &avc_opts,
    )
    .expect_err("native_export::export_y4m_pipeline_with_options must enforce estimated_index_payload_bytes");
    assert!(
        y4m_exec_err.to_string().contains("controlled memory budget exceeded"),
        "unexpected y4m_exec_err: {y4m_exec_err}"
    );
    assert!(!out_y4m.exists());
}

#[test]
fn pcm_matroska_all_canonical_file_tags_and_mp4_relocate_index_payload_budget_are_enforced() {
    let dir = TempDir::new("pcm-matroska-canonical-tags-and-mp4-relocate-index-budget");
    let wav_a = dir.0.join("a.wav");
    let wav_b = dir.0.join("b.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_a, 48_000, 2, &[0.1; 1920]).unwrap();
    fvid_media::owned_wav_file::write_wav_f32le(&wav_b, 48_000, 2, &[0.2; 1920]).unwrap();

    // 1. Canonical FileTags fields beyond the first 8 (`description`, `publisher`, `album_artist`, `disc`, `rating`)
    // must be charged in `pcm_matroska::check_output_memory_budget` and `pcm_matroska::Sink::new_with_source_metadata`.
    // For stereo (`channels == 2`), `sink_block_bytes = 8192`, `stage_working_bytes = 65536` (in `native_audio_mix::plan_with_controls`),
    // and without `description` (`tag_bytes = 67`), `mka_meta_bytes = 8 * (67 + 12) = 632`, so `needed = 65536 + 8192 + 632 + 16384 = 90744`.
    // With a 900-byte `description` tag, `tag_bytes` increases by `932` (`mka_meta_bytes` increases by `8 * 932 = 7456` to `8088`),
    // so `needed = 98200`, which exceeds `max_controlled_bytes = Some(92_000)`.
    let mix_opts = fvid::native_audio_mix::MixAudioOptions::default();
    let untagged_mix_opts = CopyOptions {
        max_controlled_bytes: Some(92_000),
        ..Default::default()
    };
    let untagged_mix_plan = fvid::native_audio_mix::plan_with_controls(
        &[wav_a.clone(), wav_b.clone()],
        &mix_opts,
        &untagged_mix_opts,
    )
    .expect("untagged mix plan to .mka should fit in 92_000 bytes");
    assert_eq!(untagged_mix_plan.command, "mix-audio");

    for canonical_key in ["description", "publisher", "album_artist", "disc", "rating"] {
        let mut tagged_opts = CopyOptions {
            max_controlled_bytes: Some(92_000),
            ..Default::default()
        };
        tagged_opts
            .metadata_set
            .push((canonical_key.to_string(), "x".repeat(900)));

        let mix_plan_err = fvid::native_audio_mix::plan_with_controls(
            &[wav_a.clone(), wav_b.clone()],
            &mix_opts,
            &tagged_opts,
        )
        .expect_err("mix plan to .mka must charge all 13 canonical FileTags fields");
        assert!(
            mix_plan_err.to_string().contains("controlled memory budget exceeded"),
            "unexpected mix_plan_err for {canonical_key}: {mix_plan_err}"
        );

        let out_mka = dir.0.join(format!("mix-{canonical_key}.mka"));
        let mix_exec_err = fvid::native_audio_mix::mix_audio_with_controls(
            &[wav_a.clone(), wav_b.clone()],
            &out_mka,
            &mix_opts,
            &tagged_opts,
        )
        .expect_err("mix_audio_with_controls to .mka must charge all 13 canonical FileTags fields");
        assert!(
            mix_exec_err.to_string().contains("controlled memory budget exceeded"),
            "unexpected mix_exec_err for {canonical_key}: {mix_exec_err}"
        );
        assert!(!out_mka.exists());
    }

    // 2. `fvid::media::remux` (MP4 fast-start relocation) and `fvid::native_export::is_native_mp4_matroska_with_memory_limit`
    // on `small.m4a` under `max_controlled_bytes = Some(550)` (where `moov` is 508 bytes and `estimated_index_payload_bytes()` > 600 bytes).
    let small_m4a = source("small.m4a");
    let small_opts = CopyOptions {
        max_controlled_bytes: Some(550),
        ..Default::default()
    };
    let is_native_err =
        fvid::native_export::is_native_mp4_matroska_with_memory_limit(&small_m4a, Some(550))
            .expect_err("is_native_mp4_matroska_with_memory_limit must enforce estimated_index_payload_bytes");
    assert!(
        is_native_err.to_string().contains("controlled memory budget exceeded"),
        "unexpected is_native_err: {is_native_err}"
    );

    let out_m4a = dir.0.join("relocated.m4a");
    let remux_m4a_err = fvid::media::remux(&small_m4a, &out_m4a, &small_opts)
        .expect_err("fvid::media::remux to .m4a must enforce estimated_index_payload_bytes in validate_mp4_packet_budget");
    assert!(
        remux_m4a_err.to_string().contains("controlled memory budget exceeded"),
        "unexpected remux_m4a_err: {remux_m4a_err}"
    );
    assert!(!out_m4a.exists());
}

#[test]
fn container_wave_metadata_working_set_and_matroska_source_geometry_errors_are_enforced() {
    let dir = TempDir::new("batch-385");

    // 1. `admit_audio_reader_with_interval` (MP4 and Matroska), `mp4_source_info`, and
    // `matroska_source_info_with_interval` must charge the 8x WAVE INFO working set
    // (`8 * (info.len() + assignments + 12)`) on top of the container decode/index working set!
    // On `small.m4a` (`pcm_sle`), `admit_audio_reader_with_interval` without metadata needs 37_473 bytes,
    // and `check_wave_metadata_budget` alone only charges 32_560 bytes (`8 * (2010 + 12) + 16384 = 32_560`)
    // for a 2000-byte `title` assignment.
    // Under `max_controlled_bytes = Some(40_000)`, untagged `plan_decode_audio` on `small.m4a`
    // succeeds (`37_473 <= 40_000`), and `check_wave_metadata_budget` alone would also succeed
    // (`32_560 <= 40_000`), whereas the combined MP4 decode (`37_473`) + WAVE metadata (`16_176`)
    // working set (`53_649`) exceeds 40_000!
    let small_m4a = source("small.m4a");
    let untagged_mp4_opts = CopyOptions {
        max_controlled_bytes: Some(40_000),
        ..Default::default()
    };
    fvid::media::plan_decode_audio(
        &small_m4a,
        &AudioDecodeTransform::default(),
        &untagged_mp4_opts,
    )
    .expect("untagged plan_decode_audio on small.m4a should fit in 40_000 bytes");

    let mut mp4_wave_meta_opts = untagged_mp4_opts.clone();
    mp4_wave_meta_opts
        .metadata_set
        .push(("title".into(), "t".repeat(2000)));

    let mp4_plan_err = fvid::media::plan_decode_audio(
        &small_m4a,
        &AudioDecodeTransform::default(),
        &mp4_wave_meta_opts,
    )
    .expect_err("plan_decode_audio on MP4 with WAVE metadata must charge combined MP4 + WAVE metadata working set");
    assert!(
        mp4_plan_err.contains("controlled memory budget exceeded"),
        "unexpected mp4_plan_err: {mp4_plan_err}"
    );

    // Likewise on `pcm-int.mkv` (where untagged decode needs 106_838 bytes), untagged `plan_decode_audio`
    // fits in `max_controlled_bytes = Some(110_000)`, whereas adding a 7000-byte `title` assignment
    // (`wave_meta_bytes = 56_176`, whereas `check_wave_metadata_budget` alone is 72_560 <= 110_000)
    // pushes the combined Matroska (`106_838`) + WAVE metadata (`56_176`) working set (`163_014`) above 110_000!
    let mkv_pcm_src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/pcm-int.mkv");
    let untagged_mkv_opts = CopyOptions {
        max_controlled_bytes: Some(110_000),
        ..Default::default()
    };
    fvid::media::plan_decode_audio(
        &mkv_pcm_src,
        &AudioDecodeTransform::default(),
        &untagged_mkv_opts,
    )
    .expect("untagged plan_decode_audio on pcm-int.mkv should fit in 110_000 bytes");

    let mut mkv_wave_meta_opts = untagged_mkv_opts.clone();
    mkv_wave_meta_opts
        .metadata_set
        .push(("title".into(), "t".repeat(7000)));
    let mkv_plan_err = fvid::media::plan_decode_audio(
        &mkv_pcm_src,
        &AudioDecodeTransform::default(),
        &mkv_wave_meta_opts,
    )
    .expect_err("plan_decode_audio on Matroska with WAVE metadata must charge combined Matroska + WAVE metadata working set");
    assert!(
        mkv_plan_err.contains("controlled memory budget exceeded"),
        "unexpected mkv_plan_err: {mkv_plan_err}"
    );

    // 2. When `fvid::native_loudnorm::try_apply` outputs to `.mka` under `max_controlled_bytes`
    // and `source_geometry` fails (e.g. a WAVE source with `max_packet_bytes: 1`),
    // `native_loudnorm::try_apply` must propagate the `source_geometry` error immediately!
    let wav_src = dir.0.join("src.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_src, 48_000, 2, &[0.1; 960]).unwrap();
    let bad_packet_mka_opts = CopyOptions {
        max_packet_bytes: 1,
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let out_mka = dir.0.join("out.mka");
    let loudnorm_mka_err = fvid::native_loudnorm::try_apply(
        &wav_src,
        &out_mka,
        None,
        false,
        &bad_packet_mka_opts,
    )
    .expect_err("native_loudnorm::try_apply to .mka must propagate source_geometry failure");
    assert!(
        loudnorm_mka_err
            .to_string()
            .contains("PCM packet limit cannot hold one sample frame"),
        "unexpected loudnorm_mka_err: {loudnorm_mka_err}"
    );
    assert!(!out_mka.exists());
}

#[test]
fn adts_wave_metadata_budget_and_container_loudnorm_admission_preserve_metadata_working_set() {
    let dir = TempDir::new("batch-386");

    // 1. `check_adts_decode_admission` must charge the 8x WAVE INFO working set
    // (`wave_metadata_working_bytes(0, options)`) on top of `decode_config_admission_bytes`.
    // On `aac-stereo.aac` (2-channel LC with unknown SBR signalling), `decode_config_admission_bytes`
    // is `20 * 332 * 1024 + 256 * 1024 + 2 * 2 * 1024 * 1024 = 11_255_808` bytes.
    // Under `max_controlled_bytes = Some(11_270_000)`:
    // - Untagged `plan_decode_audio` on `aac-stereo.aac` succeeds (`11_255_808 <= 11_270_000`).
    // - Adding a 2_000-byte `title` assignment (`wave_meta_bytes = 8 * (2010 + 12) = 16_176`)
    //   passes `check_wave_metadata_budget` alone (`32_560 <= 11_270_000`),
    //   but the combined ADTS decoder (`11_255_808`) + WAVE metadata (`16_176`) working set
    //   (`11_271_984`) exceeds `11_270_000`!
    let adts_src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-stereo.aac");
    let untagged_adts_opts = CopyOptions {
        max_controlled_bytes: Some(11_270_000),
        ..Default::default()
    };
    fvid::media::plan_decode_audio(
        &adts_src,
        &AudioDecodeTransform::default(),
        &untagged_adts_opts,
    )
    .expect("untagged plan_decode_audio on aac-stereo.aac should fit in 11_270_000 bytes");

    let mut tagged_adts_opts = untagged_adts_opts.clone();
    tagged_adts_opts
        .metadata_set
        .push(("title".into(), "t".repeat(2000)));
    let adts_plan_err = fvid::media::plan_decode_audio(
        &adts_src,
        &AudioDecodeTransform::default(),
        &tagged_adts_opts,
    )
    .expect_err("plan_decode_audio on ADTS with WAVE metadata must charge combined ADTS + WAVE metadata working set");
    assert!(
        adts_plan_err.contains("controlled memory budget exceeded"),
        "unexpected adts_plan_err: {adts_plan_err}"
    );
    let out_adts_wav = dir.0.join("adts-tagged.wav");
    let adts_exec_err = fvid::media::decode_audio_transformed(
        &adts_src,
        &out_adts_wav,
        AudioDecodeTransform::default(),
        &tagged_adts_opts,
    )
    .expect_err("decode_audio_transformed on ADTS with WAVE metadata must charge combined ADTS + WAVE metadata working set");
    assert!(
        adts_exec_err.contains("controlled memory budget exceeded"),
        "unexpected adts_exec_err: {adts_exec_err}"
    );
    assert!(!out_adts_wav.exists());

    // 2. `owned_adts_loudnorm::{plan_loudnorm, apply}` must validate packet byte limits via
    // `validate_configuration` during `apply` (before `decode_to_wave`) AND charge the
    // combined ADTS + WAVE metadata working set during `plan_loudnorm` and `apply`!
    let adts_loudnorm_plan_err =
        fvid_media::owned_adts_loudnorm::plan_loudnorm(&adts_src, None, false, &tagged_adts_opts)
            .expect_err("owned_adts_loudnorm::plan_loudnorm must charge combined ADTS + WAVE metadata working set");
    assert!(
        adts_loudnorm_plan_err.contains("controlled memory budget exceeded"),
        "unexpected adts_loudnorm_plan_err: {adts_loudnorm_plan_err}"
    );
    let out_adts_ln = dir.0.join("adts-ln.wav");
    let adts_loudnorm_exec_err = fvid_media::owned_adts_loudnorm::apply(
        &adts_src,
        &out_adts_ln,
        None,
        false,
        &tagged_adts_opts,
    )
    .expect_err("owned_adts_loudnorm::apply must charge combined ADTS + WAVE metadata working set");
    assert!(
        adts_loudnorm_exec_err.contains("controlled memory budget exceeded"),
        "unexpected adts_loudnorm_exec_err: {adts_loudnorm_exec_err}"
    );
    assert!(!out_adts_ln.exists());

    // 3. `owned_container_loudnorm::{plan_loudnorm, apply}` must charge the combined container
    // decode + WAVE metadata working set (`plan_decode_audio(source, &Default::default(), options)`)
    // rather than stripping `metadata_set` / `metadata_delete` before preflight!
    let small_m4a = source("small.m4a");
    let untagged_mp4_opts = CopyOptions {
        max_controlled_bytes: Some(40_000),
        ..Default::default()
    };
    fvid_media::owned_container_loudnorm::plan_loudnorm(
        &small_m4a,
        None,
        false,
        &untagged_mp4_opts,
    )
    .expect("untagged owned_container_loudnorm::plan_loudnorm on small.m4a should fit in 40_000 bytes");

    let mut tagged_mp4_opts = untagged_mp4_opts.clone();
    tagged_mp4_opts
        .metadata_set
        .push(("title".into(), "t".repeat(2000)));
    let mp4_ln_plan_err = fvid_media::owned_container_loudnorm::plan_loudnorm(
        &small_m4a,
        None,
        false,
        &tagged_mp4_opts,
    )
    .expect_err("owned_container_loudnorm::plan_loudnorm on MP4 must charge combined MP4 + WAVE metadata working set");
    assert!(
        mp4_ln_plan_err.contains("controlled memory budget exceeded"),
        "unexpected mp4_ln_plan_err: {mp4_ln_plan_err}"
    );
    let out_mp4_ln = dir.0.join("mp4-ln.wav");
    let mp4_ln_exec_err = fvid_media::owned_container_loudnorm::apply(
        &small_m4a,
        &out_mp4_ln,
        None,
        false,
        &tagged_mp4_opts,
    )
    .expect_err("owned_container_loudnorm::apply on MP4 must charge combined MP4 + WAVE metadata working set");
    assert!(
        mp4_ln_exec_err.contains("controlled memory budget exceeded"),
        "unexpected mp4_ln_exec_err: {mp4_ln_exec_err}"
    );
    assert!(!out_mp4_ln.exists());
}

#[test]
fn mp4_and_adts_matroska_remux_and_concat_charge_simultaneous_plan_cue_and_packet_working_sets() {
    let dir = TempDir(std::env::temp_dir().join(format!(
        "fvid-mp4-adts-remux-packet-coexistence-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();

    // 1. In `owned_mp4_remux::validate_selected_prefix_packet_budget`, `plan_bytes`
    // (16384 + metadata + estimated_index_payload_bytes + PacketTime plans) and the
    // reusable `payload: Vec<u8>` buffer (`sample.size` bytes) plus 32 bytes per cue
    // point are live simultaneously during `write_selected_with_text_metadata_overrides`.
    // Verify that when `plan_bytes` fits inside `max_controlled_bytes`, a packet whose
    // `plan_bytes + sample.size + 32 * packets > max_controlled_bytes` is rejected by
    // both `plan_remux` and `remux` (even when `sample.size + 16384 <= max_controlled_bytes`).
    let mp4_src = dir.0.join("large-pkt.m4a");
    {
        fn mp4_box(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut out = Vec::with_capacity(8 + body.len());
            out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
            out.extend_from_slice(fourcc);
            out.extend_from_slice(body);
            out
        }
        // Build a synthetic 1-packet AAC MP4 whose single packet is 6_000 bytes.
        // `plan_bytes` is ~16_500 bytes, and `sample.size + 16384` is 22_384 bytes,
        // so with `max_controlled_bytes = Some(23_000)`:
        // - `plan_bytes <= 23_000` (passes initial plan_bytes check)
        // - `sample.size + 16384 = 22_384 <= 23_000` (passed the old isolated packet check!)
        // - `plan_bytes + sample.size + 32 > 22_500` (wait: let's set max_controlled_bytes = Some(22_400)
        //   so `sample.size + 16384 = 22_384 <= 22_400`, `plan_bytes < 17_000 <= 22_400`,
        //   while `plan_bytes + sample.size + 32 > 22_500 > 22_400`!)
        let aac_pkt = vec![0x21u8; 6_000];
        let mdat_payload = aac_pkt.clone();
        let stsd_esds = fvid_media::owned_aac::adts::esds_for(&[0x11, 0x90]).unwrap();
        let mut mp4a = Vec::new();
        mp4a.extend_from_slice(&[0u8; 6]);
        mp4a.extend_from_slice(&1u16.to_be_bytes());
        mp4a.extend_from_slice(&[0u8; 8]);
        mp4a.extend_from_slice(&2u16.to_be_bytes());
        mp4a.extend_from_slice(&16u16.to_be_bytes());
        mp4a.extend_from_slice(&[0u8; 4]);
        mp4a.extend_from_slice(&(48000u32 << 16).to_be_bytes());
        mp4a.extend_from_slice(&mp4_box(b"esds", &stsd_esds));
        let mut stsd_data = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd_data.extend_from_slice(&mp4_box(b"mp4a", &mp4a));
        let stsd = mp4_box(b"stsd", &stsd_data);
        let stts = mp4_box(
            b"stts",
            &[
                0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 4, 0,
            ],
        );
        let stsc = mp4_box(
            b"stsc",
            &[
                0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1,
            ],
        );
        let mut stsz_data = vec![0, 0, 0, 0];
        stsz_data.extend_from_slice(&(aac_pkt.len() as u32).to_be_bytes());
        stsz_data.extend_from_slice(&1u32.to_be_bytes());
        let stsz = mp4_box(b"stsz", &stsz_data);
        // Temporary stco with 0, then patch after computing moov size
        let stco_placeholder = mp4_box(b"stco", &[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
        let build_moov = |chunk_offset: u32| {
            let mut stco_bytes = vec![0, 0, 0, 0, 0, 0, 0, 1];
            stco_bytes.extend_from_slice(&chunk_offset.to_be_bytes());
            let stco = mp4_box(b"stco", &stco_bytes);
            let mut stbl_body = Vec::new();
            stbl_body.extend_from_slice(&stsd);
            stbl_body.extend_from_slice(&stts);
            stbl_body.extend_from_slice(&stsc);
            stbl_body.extend_from_slice(&stsz);
            stbl_body.extend_from_slice(&stco);
            let stbl = mp4_box(b"stbl", &stbl_body);
            let smhd = mp4_box(b"smhd", &[0; 8]);
            let dref = mp4_box(
                b"dref",
                &[
                    0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 12, b'u', b'r', b'l', b' ', 0, 0, 0, 1,
                ],
            );
            let dinf = mp4_box(b"dinf", &dref);
            let mut minf_body = Vec::new();
            minf_body.extend_from_slice(&smhd);
            minf_body.extend_from_slice(&dinf);
            minf_body.extend_from_slice(&stbl);
            let minf = mp4_box(b"minf", &minf_body);
            let mut mdhd_data = vec![0; 4];
            mdhd_data.extend_from_slice(&[0; 8]);
            mdhd_data.extend_from_slice(&48000u32.to_be_bytes());
            mdhd_data.extend_from_slice(&1024u32.to_be_bytes());
            mdhd_data.extend_from_slice(&[0x55, 0xc4, 0, 0]);
            let mdhd = mp4_box(b"mdhd", &mdhd_data);
            let hdlr = mp4_box(
                b"hdlr",
                &[
                    0, 0, 0, 0, 0, 0, 0, 0, b's', b'o', b'u', b'n', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0,
                ],
            );
            let mut mdia_body = Vec::new();
            mdia_body.extend_from_slice(&mdhd);
            mdia_body.extend_from_slice(&hdlr);
            mdia_body.extend_from_slice(&minf);
            let mdia = mp4_box(b"mdia", &mdia_body);
            let mut tkhd_data = vec![0, 0, 0, 7];
            tkhd_data.extend_from_slice(&[0; 8]);
            tkhd_data.extend_from_slice(&1u32.to_be_bytes());
            tkhd_data.extend_from_slice(&[0; 4]);
            tkhd_data.extend_from_slice(&1024u32.to_be_bytes());
            tkhd_data.extend_from_slice(&[0; 8]);
            tkhd_data.extend_from_slice(&[0; 4]);
            tkhd_data.extend_from_slice(&0x0100u16.to_be_bytes());
            tkhd_data.extend_from_slice(&[0; 2]);
            tkhd_data.extend_from_slice(&[
                0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0x40, 0, 0, 0,
            ]);
            tkhd_data.extend_from_slice(&[0; 8]);
            let tkhd = mp4_box(b"tkhd", &tkhd_data);
            let mut trak_body = Vec::new();
            trak_body.extend_from_slice(&tkhd);
            trak_body.extend_from_slice(&mdia);
            let trak = mp4_box(b"trak", &trak_body);
            let mut mvhd_data = vec![0; 4];
            mvhd_data.extend_from_slice(&[0; 8]);
            mvhd_data.extend_from_slice(&48000u32.to_be_bytes());
            mvhd_data.extend_from_slice(&1024u32.to_be_bytes());
            mvhd_data.extend_from_slice(&0x00010000u32.to_be_bytes());
            mvhd_data.extend_from_slice(&0x0100u16.to_be_bytes());
            mvhd_data.extend_from_slice(&[0; 10]);
            mvhd_data.extend_from_slice(&[
                0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0x40, 0, 0, 0,
            ]);
            mvhd_data.extend_from_slice(&[0; 24]);
            mvhd_data.extend_from_slice(&2u32.to_be_bytes());
            let mvhd = mp4_box(b"mvhd", &mvhd_data);
            let mut moov_body = Vec::new();
            moov_body.extend_from_slice(&mvhd);
            moov_body.extend_from_slice(&trak);
            mp4_box(b"moov", &moov_body)
        };
        let _ = stco_placeholder;
        let ftyp = mp4_box(
            b"ftyp",
            b"M4A \x00\x00\x02\x00M4A isommp42",
        );
        let dummy_moov = build_moov(0);
        let chunk_offset = (ftyp.len() + dummy_moov.len() + 8) as u32;
        let moov = build_moov(chunk_offset);
        let mdat = mp4_box(b"mdat", &mdat_payload);
        let mut file_bytes = Vec::new();
        file_bytes.extend_from_slice(&ftyp);
        file_bytes.extend_from_slice(&moov);
        file_bytes.extend_from_slice(&mdat);
        std::fs::write(&mp4_src, &file_bytes).unwrap();
    }

    // With `max_controlled_bytes = Some(22_450)` and a metadata tag of ~100 bytes
    // (`raw_meta_bytes * 8` ~ 1056 bytes, so `plan_bytes` ~ 17_500 bytes):
    // - `plan_bytes` (~17_500) <= 22_450
    // - `sample.size + 16384` (6_000 + 16_384 = 22_384) <= 22_450
    // - `plan_bytes + sample.size + 32` (~23_532) > 22_450!
    let mp4_opts = CopyOptions {
        max_controlled_bytes: Some(22_450),
        metadata_set: vec![("title".into(), "x".repeat(100))],
        ..Default::default()
    };
    let mp4_plan_err = fvid_media::owned_mp4_remux::plan_remux(&mp4_src, &mp4_opts)
        .expect_err("owned_mp4_remux::plan_remux must charge plan_bytes + sample.size + cue working set");
    assert!(
        mp4_plan_err.contains("controlled memory budget exceeded"),
        "unexpected mp4_plan_err: {mp4_plan_err}"
    );
    let mp4_out = dir.0.join("mp4-remux.mka");
    let mp4_exec_err = fvid_media::owned_mp4_remux::remux(&mp4_src, &mp4_out, &mp4_opts)
        .expect_err("owned_mp4_remux::remux must charge plan_bytes + sample.size + cue working set");
    assert!(
        mp4_exec_err.contains("controlled memory budget exceeded"),
        "unexpected mp4_exec_err: {mp4_exec_err}"
    );
    assert!(!mp4_out.exists());

    // 2. In `owned_adts_remux::{plan_remux, remux}` and `owned_concat::{plan_concat, concat}`,
    // each ADTS packet allocates a `Vec<u8>` of `packet.len()` bytes while `base_bytes`,
    // `meta_bytes`, and `(packets * 32)` cue entries remain live.
    let adts_a = dir.0.join("a.aac");
    let adts_b = dir.0.join("b.aac");
    {
        // Build a valid 1-frame ADTS file with a 1_000-byte raw AAC payload (ID_END = 7 << 5 = 0xE0).
        let payload_len = 1_000usize;
        let frame_len = 7 + payload_len;
        let mut frame = vec![0u8; frame_len];
        frame[0] = 0xFF;
        frame[1] = 0xF1; // MPEG-4, Layer 0, no CRC
        // profile = AAC-LC (1 -> 01), freq_idx = 3 (48kHz -> 0011), chan_cfg = 2 (010)
        frame[2] = (1 << 6) | (3 << 2);
        frame[3] = (2 << 6) | ((frame_len >> 11) as u8 & 0x03);
        frame[4] = ((frame_len >> 3) & 0xFF) as u8;
        frame[5] = (((frame_len & 0x07) as u8) << 5) | 0x1F;
        frame[6] = 0xFC; // raw_data_blocks = 0 (1 block)
        frame[7] = 0xE0; // ID_END (7 << 5)
        std::fs::write(&adts_a, &frame).unwrap();
        std::fs::write(&adts_b, &frame).unwrap();
    }

    // For `owned_adts_remux`, `base_bytes = 16_384`, `meta_bytes = 0`, 1 packet cue = 32,
    // `packet.len() = 1_000`. Total simultaneous working set = 16_384 + 32 + 1_000 = 17_416.
    // With `max_controlled_bytes = Some(17_000)`:
    // - `16_384 + 32 = 16_416 <= 17_000` (passed old check that ignored `packet.len()`)
    // - `16_384 + 32 + 1_000 = 17_416 > 17_000` (must be rejected!)
    let adts_remux_opts = CopyOptions {
        max_controlled_bytes: Some(17_000),
        ..Default::default()
    };
    let adts_plan_err = fvid_media::owned_adts_remux::plan_remux(&adts_a, &adts_remux_opts)
        .expect_err("owned_adts_remux::plan_remux must charge packet.len() on top of base_bytes + cues + meta_bytes");
    assert!(
        adts_plan_err.contains("controlled memory budget exceeded"),
        "unexpected adts_plan_err: {adts_plan_err}"
    );
    let adts_out = dir.0.join("adts-remux.mka");
    let adts_exec_err = fvid_media::owned_adts_remux::remux(&adts_a, &adts_out, &adts_remux_opts)
        .expect_err("owned_adts_remux::remux must charge packet.len() on top of base_bytes + cues + meta_bytes");
    assert!(
        adts_exec_err.contains("controlled memory budget exceeded"),
        "unexpected adts_exec_err: {adts_exec_err}"
    );
    assert!(!adts_out.exists());

    // For `owned_concat` with 2 inputs, `base_bytes = 2 * 8_192 + 16_384 = 32_768`,
    // `meta_bytes = 0`, 2 packet cues = 64, `packet.len() = 1_000`.
    // Total simultaneous working set = 32_768 + 64 + 1_000 = 33_832.
    // With `max_controlled_bytes = Some(33_200)`:
    // - `32_768 + 64 = 32_832 <= 33_200` (passed old check that ignored `packet.len()`)
    // - `32_768 + 32 + 1_000 = 33_800 > 33_200` (must be rejected!)
    let concat_opts = CopyOptions {
        max_controlled_bytes: Some(33_200),
        ..Default::default()
    };
    let sources = vec![adts_a.clone(), adts_b.clone()];
    let concat_plan_err = fvid_media::owned_concat::plan_concat(&sources, &concat_opts)
        .expect_err("owned_concat::plan_concat must charge packet.len() on top of base_bytes + cues + meta_bytes");
    assert!(
        concat_plan_err.contains("controlled memory budget exceeded"),
        "unexpected concat_plan_err: {concat_plan_err}"
    );
    let concat_out = dir.0.join("adts-concat.mka");
    let concat_exec_err = fvid_media::owned_concat::concat(&sources, &concat_out, &concat_opts)
        .expect_err("owned_concat::concat must charge packet.len() on top of base_bytes + cues + meta_bytes");
    assert!(
        concat_exec_err.contains("controlled memory budget exceeded"),
        "unexpected concat_exec_err: {concat_exec_err}"
    );
    assert!(!concat_out.exists());
}

