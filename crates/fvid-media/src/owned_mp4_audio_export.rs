//! Owned MP4 AAC/ALAC/PCM file export through presentation decoding and WAVE DSP.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
fn open(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_mp4::Mp4Reader<BufReader<File>>> {
    crate::owned_mp4::Mp4Reader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        crate::owned_mp4::Limits {
            packet_bytes: options.max_packet_bytes,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn recognizes(source: &Path, options: &CopyOptions) -> Result<bool> {
    let mut prefix = [0; 8];
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    if file.read_exact(&mut prefix).is_err() || !crate::owned_mp4::recognizes_prefix(&prefix) {
        return Ok(false);
    }
    let reader = open(source, options)?;
    Ok(reader.tracks().iter().enumerate().any(|(index, t)| {
        (options.streams.is_empty() || options.streams.contains(&index))
            && t.handler == *b"soun"
            && matches!(
                &t.codec,
                b"mp4a"
                    | b"alac"
                    | b"raw "
                    | b"sowt"
                    | b"twos"
                    | b"in24"
                    | b"in32"
                    | b"fl32"
                    | b"fl64"
                    | b"ima4"
                    | b"ms\x00\x11"
            )
    }))
}
pub(crate) fn descriptor(
    source: &Path,
    options: &CopyOptions,
) -> Result<(usize, u32, u16, u32, String)> {
    let reader = open(source, options)?;
    if !reader.refused().is_empty() {
        return Err("MP4 stream selection requires every track to be represented".into());
    }
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err("select exactly one audio stream".into()),
    };
    let index =
        crate::owned_mp4_audio::mp4_audio_index(&reader, selected).map_err(|e| e.to_string())?;
    crate::owned_mp4_audio::admit_audio_reader(&reader, index, options)
        .map_err(|e| e.to_string())?;
    let track = &reader.tracks()[index];
    let decoder =
        crate::owned_mp4_audio::Mp4TimelineDecoder::new(track).map_err(|e| e.to_string())?;
    if track.timescale == 0
        || track.sample_rate != decoder.sample_rate()
        || track.channels != decoder.channels()
    {
        return Err("MP4 audio export requires valid clock and matching audio geometry".into());
    }
    Ok((
        index,
        decoder.sample_rate(),
        decoder.channels(),
        decoder.channel_mask(),
        match &track.codec {
            b"mp4a" => "aac",
            b"alac" => "alac",
            b"ima4" => "adpcm_ima_qt",
            b"ms\x00\x11" => "adpcm_ima_wav",
            b"raw " => "pcm_u8",
            b"sowt" => "pcm_sle",
            b"twos" => "pcm_sbe",
            b"in24" | b"in32" => {
                if track.configuration.first() == Some(&1) {
                    "pcm_sle"
                } else {
                    "pcm_sbe"
                }
            }
            b"fl32" | b"fl64" => "pcm_float",
            _ => return Err("selected MP4 audio codec is not owned".into()),
        }
        .into(),
    ))
}
fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32, bool)> {
    let (_, rate, channels, mask, codec) = descriptor(source, options)?;
    Ok((rate, channels, mask, codec.starts_with("pcm_")))
}
use crate::owned_adts_export::decoded_prefix as prefix;
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    let (rate, channels, mask, precise) = match geometry(source, options) {
        Ok(value) => value,
        Err(error) => return error.starts_with("controlled memory budget exceeded:"),
    };
    let output = transform.channels.unwrap_or(i32::from(channels));
    if output != i32::from(channels)
        && (!((channels <= 2 && (1..=8).contains(&output))
            || (channels <= 8 && matches!(output, 1 | 2)))
            || !crate::owned_pcm_channels::standard_mask(channels)
                .is_some_and(|standard| standard == u64::from(mask)))
    {
        return false;
    }
    let Ok(file) = File::open(source) else {
        return false;
    };
    let mut probe = options.clone();
    probe.progress = None;
    probe.metadata_set.clear();
    probe.metadata_delete.clear();
    probe.stream_metadata_set.clear();
    probe.stream_metadata_delete.clear();
    let result = if precise {
        crate::owned_mp4_audio::decode_mp4_pcm_f64(
            BufReader::new(file),
            &mut std::io::sink(),
            prefix(transform, rate),
            &probe,
        )
    } else {
        crate::owned_mp4_audio::decode_mp4_audio_pcm(
            BufReader::new(file),
            &mut std::io::sink(),
            prefix(transform, rate),
            &probe,
        )
    };
    result.is_ok()
}
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let (rate, channels, mask, precise) = geometry(source, options)?;
    let spool = crate::owned_adts_export::spool_decoded_with_precision(
        rate,
        channels,
        mask,
        if precise { 64 } else { 32 },
        options,
        |writer, options| {
            let file = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
            let result = if precise {
                crate::owned_mp4_audio::decode_mp4_pcm_f64(
                    file,
                    writer,
                    prefix(transform, rate),
                    options,
                )
            } else {
                crate::owned_mp4_audio::decode_mp4_audio_pcm(
                    file,
                    writer,
                    prefix(transform, rate),
                    options,
                )
            };
            result.map_err(|e| e.to_string())
        },
    )?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn mp4_aac_export_loudness_and_normalization_keep_owned_budget() {
        check_budget("../../tests/fixtures/audio/aac-native-edit.m4a");
    }
    #[test]
    fn mp4_alac_export_loudness_and_normalization_keep_owned_budget() {
        check_budget("../../tests/fixtures/playback-errors/alac-resample-window.m4a");
    }
    #[test]
    fn mp4_pcm_export_loudness_and_normalization_keep_owned_budget() {
        check_budget("../../tests/fixtures/audio/pcm-screen.mov");
    }
    #[test]
    fn quicktime_pcm_precision_export_preserves_silence_and_repeated_edits() {
        let doubles = [
            0.12345678901234567,
            -0.9876543210987654,
            0.5000000000000001,
            -0.5000000000000001,
            1e-100,
            -1e-100,
        ];
        let integers = [2147483647, -2147483647, 16777217, -16777217, 1, -1];
        let normalized: Vec<f64> = integers
            .iter()
            .map(|&n| f64::from(n) / 2147483648.0)
            .collect();
        for (name, values) in [
            ("pcm64-precision-edits.mov", doubles.as_slice()),
            ("pcm32-precision-edits.mov", normalized.as_slice()),
        ] {
            assert!(values.iter().any(|&x| f64::from(x as f32) != x));
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/playback-errors")
                .join(name);
            let options = CopyOptions {
                max_controlled_bytes: Some(32 * 1024 * 1024),
                ..Default::default()
            };
            let plan = crate::plan_decode_audio(&source, &Default::default(), &options).unwrap();
            assert!(plan.steps.iter().any(|s| s.detail.contains("float64")));
            let output = std::env::temp_dir()
                .join(format!("fvid-precision-{name}-{}.wav", std::process::id()));
            let _ = std::fs::remove_file(&output);
            let stats = crate::decode_audio(&source, &output, &options).unwrap();
            assert_eq!(stats.sample_frames, 12);
            let bytes = std::fs::read(&output).unwrap();
            std::fs::remove_file(&output).unwrap();
            let info = crate::owned_wave_inspect::inspect(&mut std::io::Cursor::new(&bytes), None)
                .unwrap();
            assert_eq!(info.bits_per_sample, 64);
            let expected: Vec<u8> = [0f64, 0.0]
                .iter()
                .chain(values)
                .chain(&values[2..])
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(
                &bytes[info.data_offset as usize
                    ..info.data_offset as usize + info.data_bytes as usize],
                expected
            );
        }
    }
    #[test]
    fn ima4_export_keeps_owned_backend_and_budget_policy() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/ima4-ramp-edits.mov");
        let tiny = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        assert!(supports(&source, Default::default(), &tiny));
        let output = std::env::temp_dir().join(format!(
            "fvid-ima4-owned-dispatch-{}.wav",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        assert!(
            crate::decode_audio(&source, &output, &tiny)
                .unwrap_err()
                .contains("controlled memory budget exceeded")
        );
        assert!(!output.exists());
        let stats = crate::decode_audio(
            &source,
            &output,
            &CopyOptions {
                max_controlled_bytes: Some(32 * 1024 * 1024),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 128);
        let bytes = std::fs::read(&output).unwrap();
        std::fs::remove_file(&output).unwrap();
        let info =
            crate::owned_wave_inspect::inspect(&mut std::io::Cursor::new(&bytes), None).unwrap();
        let ramp: Vec<f32> = (1..=64).map(|n| n as f32 / 32768.0).collect();
        let expected: Vec<u8> = [0f32, 0.0]
            .iter()
            .chain(&ramp)
            .chain(&ramp[2..])
            .flat_map(|x| x.to_le_bytes())
            .collect();
        assert_eq!(
            &bytes[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize],
            expected
        );
    }
    #[test]
    fn ima_wav_export_keeps_owned_backend_and_budget_policy() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/ima-wav-stereo-edits.mov");
        let tiny = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        assert!(supports(&source, Default::default(), &tiny));
        let output =
            std::env::temp_dir().join(format!("fvid-ima-wav-dispatch-{}.wav", std::process::id()));
        let _ = std::fs::remove_file(&output);
        assert!(
            crate::decode_audio(&source, &output, &tiny)
                .unwrap_err()
                .contains("controlled memory budget exceeded")
        );
        assert!(!output.exists());
        let stats = crate::decode_audio(
            &source,
            &output,
            &CopyOptions {
                max_controlled_bytes: Some(32 * 1024 * 1024),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((stats.sample_frames, stats.channels), (18, 2));
        std::fs::remove_file(&output).unwrap();
    }
    fn check_budget(fixture: &str) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture);
        let tiny = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        let admitted = CopyOptions {
            max_controlled_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        };
        assert!(supports(&source, Default::default(), &tiny));
        assert!(crate::owned_audio_plan::supports(
            &source,
            &Default::default(),
            &tiny
        ));
        assert!(crate::owned_container_loudness::supports(&source, &tiny));
        assert!(
            crate::plan_decode_audio(&source, &Default::default(), &tiny)
                .unwrap_err()
                .contains("controlled memory budget exceeded")
        );
        crate::plan_decode_audio(&source, &Default::default(), &admitted).unwrap();
        assert!(
            crate::measure_loudness(&source, &tiny)
                .unwrap_err()
                .contains("controlled memory budget exceeded")
        );
        let expected = crate::measure_loudness(&source, &Default::default()).unwrap();
        let actual = crate::measure_loudness(&source, &admitted).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        for mode in 0..3 {
            let output = std::env::temp_dir().join(format!(
                "fvid-mp4-audio-budget-{}-{mode}-{}.wav",
                source.file_stem().unwrap().to_string_lossy(),
                std::process::id()
            ));
            let _ = std::fs::remove_file(&output);
            let apply = |options: &CopyOptions| -> Result<()> {
                match mode {
                    0 => crate::decode_audio(&source, &output, options).map(|_| ()),
                    1 => crate::apply_loudnorm(&source, &output, None, options).map(|_| ()),
                    _ => crate::apply_loudnorm_dual(&source, &output, None, options).map(|_| ()),
                }
            };
            assert!(
                apply(&tiny)
                    .unwrap_err()
                    .contains("controlled memory budget exceeded")
            );
            assert!(!output.exists());
            apply(&admitted).unwrap();
            let bytes = std::fs::read(&output).unwrap();
            std::fs::remove_file(&output).unwrap();
            apply(&CopyOptions::default()).unwrap();
            assert_eq!(std::fs::read(&output).unwrap(), bytes);
            std::fs::remove_file(&output).unwrap();
        }
    }
}
