//! Owned MP4 AAC/ALAC/PCM/ADPCM/MACE file export through presentation decoding and WAVE DSP.
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
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let metadata_bytes = crate::owned_mp4::Limits::default()
        .metadata_bytes
        .min(options.max_controlled_bytes.unwrap_or(usize::MAX));
    let mut reader = crate::owned_mp4::Mp4Reader::open_indexed(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        crate::owned_mp4::Limits {
            metadata_bytes,
            ..Default::default()
        },
    )
    .map_err(|error| {
        if metadata_bytes < crate::owned_mp4::Limits::default().metadata_bytes
            && error.to_string().contains("MP4 metadata exceeds budget")
        {
            "controlled memory budget exceeded: MP4 metadata exceeds budget".to_owned()
        } else {
            error.to_string()
        }
    })?;
    reader.restrict_packet_bytes(options.max_packet_bytes);
    Ok(reader)
}
pub(crate) fn recognizes(source: &Path, options: &CopyOptions) -> Result<bool> {
    let mut prefix = [0; 8];
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    if file.read_exact(&mut prefix).is_err() || !crate::owned_mp4::recognizes_prefix(&prefix) {
        return Ok(false);
    }
    let reader = open(source, options)?;
    let automatic = reader.automatic_track_indices(&[*b"soun"]);
    Ok(reader.tracks().iter().enumerate().any(|(index, t)| {
        ((options.streams.is_empty() && automatic.contains(&index)) || options.streams.contains(&index))
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
                    | b"MAC3"
                    | b"MAC6"
                    | b"ima4"
                    | b"ms\x00\x11"
                    | b"ms\x00\x02"
            )
    }))
}
pub(crate) fn descriptor(
    source: &Path,
    options: &CopyOptions,
) -> Result<(usize, u32, u16, u32, String)> {
    descriptor_for_decode(source, None, options)
}
pub(crate) fn descriptor_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(usize, u32, u16, u32, String)> {
    let mut reader = open(source, options)?;
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
    let initial_rate = reader.tracks()[index]
        .description_for_sample(0)
        .map_or(reader.tracks()[index].sample_rate, |d| d.sample_rate);
    let interval = transform.and_then(|t| prefix(t, initial_rate));
    crate::owned_mp4_audio::admit_audio_reader_with_interval(&reader, index, interval, options, None)
        .map_err(|e| e.to_string())?;
    if options.max_packet_bytes < crate::owned_mp4::Limits::default().packet_bytes {
        let track = &reader.tracks()[index];
        let max_samples = crate::owned_mp4_audio::mp4_active_samples(track, interval, options.max_packets);
        let exceeds = match &track.samples {
            crate::owned_mp4::SampleIndex::Expanded(samples) => samples
                .iter()
                .take(max_samples)
                .any(|s| (s.size as usize) > options.max_packet_bytes),
            crate::owned_mp4::SampleIndex::Uniform(uniform) => {
                uniform.count > 0
                    && max_samples > 0
                    && (uniform.bytes_per_frame as usize) > options.max_packet_bytes
            }
        };
        if exceeds {
            return Err("MP4 packet exceeds budget".into());
        }
    }
    let in_band_ps = crate::owned_mp4_audio::negotiate_mp4_ps_with_interval(
        &mut reader,
        index,
        interval,
        options.max_packets,
        || {
            if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err(crate::owned_matroska_audio::invalid("media operation cancelled"));
            }
            crate::owned_budget::check_rss_budget(options)
                .map_err(|e| crate::owned_matroska_audio::invalid(&e))
        },
    )
    .map_err(|e| e.to_string())?;
    let source_track = &reader.tracks()[index];
    let first = source_track.samples.get(0).ok_or("MP4 audio has no samples")?;
    let setup = source_track.decoder_setup(first.description_index).map_err(|e| e.to_string())?;
    let track = &setup;
    let decoder = if in_band_ps {crate::owned_mp4_audio::Mp4TimelineDecoder::with_in_band_ps(track)} else {crate::owned_mp4_audio::Mp4TimelineDecoder::new(track)}.map_err(|e|e.to_string())?;
    if track.timescale == 0
        || track.sample_rate != decoder.sample_rate()
        || (!in_band_ps && track.channels != decoder.channels())
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
            b"MAC3" => "mace3",
            b"MAC6" => "mace6",
            b"ima4" => "adpcm_ima_qt",
            b"ms\x00\x11" => "adpcm_ima_wav",
            b"ms\x00\x02" => "adpcm_ms",
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
fn geometry_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, bool)> {
    let (_, rate, channels, mask, codec) = descriptor_for_decode(source, transform, options)?;
    let reader = open(source, options)?;
    let index = crate::owned_mp4_audio::mp4_audio_index(&reader, options.streams.first().copied()).map_err(|e|e.to_string())?;
    let track = &reader.tracks()[index];
    let interval = transform.and_then(|t| prefix(t, rate));
    let active_samples = crate::owned_mp4_audio::mp4_active_samples(track, interval, options.max_packets);
    let all_pcm = (0..active_samples).all(|n| track.description_for_sample(n).is_some_and(|d| matches!(&d.codec,b"raw "|b"sowt"|b"twos"|b"in24"|b"in32"|b"fl32"|b"fl64")));
    Ok((rate, channels, mask, codec.starts_with("pcm_") && all_pcm))
}
pub(crate) fn normalization_target(source:&Path,transform:AudioDecodeTransform,options:&CopyOptions,rate:u32,channels:u16,mask:u32)->Result<Option<(u32,u16,u32)>> {
    let requested=transform.channels.is_some()||transform.sample_rate.is_some();
    if !requested&&mask!=0 {return Ok(None);}
    let mut reader=open(source,options)?;
    let index=crate::owned_mp4_audio::mp4_audio_index(&reader,options.streams.first().copied()).map_err(|e|e.to_string())?;
    let interval = prefix(transform, rate);
    let active_samples = crate::owned_mp4_audio::mp4_active_samples(&reader.tracks()[index], interval, options.max_packets);
    let first_desc = reader.tracks()[index].description_for_sample(0).cloned();
    let changing = (0..active_samples).any(|i| reader.tracks()[index].description_for_sample(i) != first_desc.as_ref());
    if !changing {
        let projection=transform.channels.is_some_and(|c|c!=i32::from(channels)&&(mask==0||(channels>2&&c>2)));
        if !projection {return Ok(None);}
    }else if mask==0&&transform.channels.is_none() {
        let mut check = || -> std::result::Result<(), crate::owned_matroska_audio::Error> {
            if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err(crate::owned_matroska_audio::invalid("media operation cancelled"));
            }
            crate::owned_budget::check_rss_budget(options).map_err(|e| crate::owned_matroska_audio::invalid(&e))
        };
        if crate::owned_mp4_audio::mp4_normalized_geometry_controlled_with_interval(&mut reader,index,interval,options.max_packets,&mut check).map_err(|e|e.to_string())?.is_some() {
            return Err("MP4 normalization requires an explicit destination speaker layout".into());
        }
        return Ok(None);
    }
    if !requested {return Ok(None);}
    let rate=transform.sample_rate.map(u32::try_from).transpose().map_err(|_|"invalid output rate")?.unwrap_or(rate);
    let output=transform.channels.map(u16::try_from).transpose().map_err(|_|"invalid output channels")?.unwrap_or(channels);
    let mask=if transform.channels.is_some() {crate::owned_pcm_channels::standard_mask(output).ok_or("unsupported output speaker layout")? as u32}else{mask};
    crate::owned_mp4_audio::admit_audio_reader_with_interval(&reader, index, interval, options, Some((rate, output, mask))).map_err(|e| e.to_string())?;
    Ok(Some((rate,output,mask)))
}
use crate::owned_adts_export::decoded_prefix as prefix;
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    let (rate, channels, mask, precise) = match geometry_for_decode(source, Some(transform), options) {
        Ok(value) => value,
        Err(error) => return error.starts_with("controlled memory budget exceeded:"),
    };
    let target=match normalization_target(source,transform,options,rate,channels,mask) {Ok(t)=>t,Err(e)=>return e.starts_with("controlled memory budget exceeded:")};
    let output = transform.channels.unwrap_or(i32::from(channels));
    let aac_fold=if target.is_none() && output!=i32::from(channels)&&matches!(output,1|2){match crate::owned_aac_export_routing::router_for_decode(source,rate,channels,Some(transform),options){Ok(router)=>router.is_some(),Err(error)=>return error.starts_with("controlled memory budget exceeded:")}}else{false};
    if target.is_none() && output != i32::from(channels) && !aac_fold
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
        crate::owned_mp4_audio::decode_mp4_pcm_f64_targeted(
            BufReader::new(file),
            &mut std::io::sink(),
            prefix(transform, target.map_or(rate,|t|t.0)),
            &probe,
            target,
        )
    } else {
        crate::owned_mp4_audio::decode_mp4_audio_pcm_targeted(
            BufReader::new(file),
            &mut std::io::sink(),
            prefix(transform, target.map_or(rate,|t|t.0)),
            &probe,
            target,
        )
    };
    result.is_ok()
        || result
            .err()
            .is_some_and(|e| e.to_string().starts_with("controlled memory budget exceeded:"))
}
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    let (rate, channels, mask, precise) = geometry_for_decode(source, Some(transform), options)?;
    let target=normalization_target(source,transform,options,rate,channels,mask)?;
    let (rate,channels,mask)=target.unwrap_or((rate,channels,mask));
    let info = crate::owned_wave_metadata::mp4_source_info(source, options)?;
    let spool = crate::owned_adts_export::spool_decoded_with_precision(
        rate,
        channels,
        mask,
        if precise { 64 } else { 32 },
        options,
        |writer, options| {
            let file = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
            let result = if precise {
                crate::owned_mp4_audio::decode_mp4_pcm_f64_targeted(
                    file,
                    writer,
                    prefix(transform, rate),
                    options,
                    target,
                )
            } else {
                crate::owned_mp4_audio::decode_mp4_audio_pcm_targeted(
                    file,
                    writer,
                    prefix(transform, rate),
                    options,
                    target,
                )
            };
            result.map_err(|e| e.to_string())
        },
    )?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
    crate::owned_aac_export_routing::export(source,spool,destination,transform,options)
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
    #[test]
    fn ms_adpcm_export_keeps_owned_backend_and_budget_policy() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/ms-adpcm-stereo-edits.mov");
        let tiny = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        assert!(supports(&source, Default::default(), &tiny));
        let output =
            std::env::temp_dir().join(format!("fvid-ms-adpcm-dispatch-{}.wav", std::process::id()));
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
        assert_eq!((stats.sample_frames, stats.channels), (12, 2));
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
