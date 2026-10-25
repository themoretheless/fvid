//! Preserve AAC PCE positions across the private WAVE/DSP export boundary.
use crate::owned_stereo_router::StereoRouter;
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn router(
    source: &Path,
    rate: u32,
    channels: u16,
    options: &CopyOptions,
) -> Result<Option<StereoRouter>> {
    let mut prefix = [0; 8];
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let n = file.read(&mut prefix).map_err(|e| e.to_string())?;
    let selected = match options.streams.as_slice() {
        [] => None,
        [i] => Some(*i),
        _ => return Err("select exactly one audio stream".into()),
    };
    let asc = if n >= 8 && crate::owned_mp4::recognizes_prefix(&prefix) {
        let mut reader = crate::owned_mp4::Mp4Reader::open_indexed(
            BufReader::new(File::open(source).map_err(|e| e.to_string())?),
            crate::owned_mp4::Limits::default(),
        )
        .map_err(|e| e.to_string())?;
        reader.restrict_packet_bytes(options.max_packet_bytes);
        let index = crate::owned_mp4_audio::mp4_audio_index(&reader, selected)
            .map_err(|e| e.to_string())?;
        if reader.tracks()[index].description_for_sample(0).is_none_or(|s|s.codec!=*b"mp4a") {return Ok(None);}
        crate::owned_mp4_audio::admit_audio_reader(&reader,index,options).map_err(|e|e.to_string())?;
        if let Some((normalized_rate,normalized_channels,mask))=crate::owned_mp4_audio::mp4_normalized_geometry(&mut reader,index,options.max_packets).map_err(|e|e.to_string())? {
            if rate!=normalized_rate||channels!=normalized_channels {return Err("MP4 normalized spool geometry mismatch".into());}
            return StereoRouter::new(channels,mask).map(Some);
        }
        let track = &reader.tracks()[index];
        let active_samples = if track.edits.is_empty() {
            options
                .max_packets
                .map(|m| usize::try_from(m).unwrap_or(usize::MAX))
                .unwrap_or(track.samples.len())
                .min(track.samples.len())
        } else {
            track.samples.len()
        };
        let first = track.samples.get(0).ok_or("MP4 audio has no samples")?;
        let setup = track.decoder_setup(first.description_index).map_err(|e|e.to_string())?;
        if setup.codec != *b"mp4a" { return Ok(None); }
        let mut router: Option<StereoRouter> = None;
        for (i, description) in track.sample_descriptions.iter().enumerate() {
            if !(0..active_samples).any(|n| track.samples.get(n).is_some_and(|s| s.description_index as usize == i+1)) { continue; }
            if description.codec != *b"mp4a" { return Err("mixed-codec MP4 speaker routing requires per-configuration routing".into()); }
            let asc = crate::owned_codec_config::aac_specific_config(&description.configuration).map_err(|e|e.to_string())?;
            let parsed = crate::owned_aac::config::AudioSpecificConfig::parse(asc).map_err(|e|e.to_string())?;
            let next = crate::owned_aac::speaker_routing::stereo_router(asc, rate, channels, channels == 2 && parsed.core.channels == 1)?;
            if router.as_ref().is_some_and(|previous| *previous != next) {
                return Err("MP4 AAC speaker routing changes require per-configuration routing".into());
            }
            router = Some(next);
        }
        return Ok(router);
    } else if n >= 4 && prefix[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        let mut reader = crate::owned_matroska_audio::open_aac_reader(
            BufReader::new(File::open(source).map_err(|e| e.to_string())?),
            options,
            options.max_packet_bytes,
        )
        .map_err(|e| e.to_string())?;
        let index = crate::owned_matroska_audio::matroska_audio_index(&reader, selected)
            .map_err(|e| e.to_string())?;
        if reader.tracks[index].codec != "A_AAC" {
            return Ok(None);
        }
        crate::owned_matroska_audio::admit_audio_reader(&mut reader, index, options)
            .map_err(|e| e.to_string())?;
        reader.tracks[index].codec_private.clone()
    } else if crate::owned_aac::adts::header(&prefix[..n]).is_some() {
        if selected.is_some_and(|i| i != 0) {
            return Err("ADTS has only stream 0".into());
        }
        let reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
            BufReader::new(File::open(source).map_err(|e| e.to_string())?),
            options.max_packet_bytes,
        )
        .map_err(|e| e.to_string())?;
        crate::owned_aac::stream::check_adts_decode_admission(
            reader.audio_specific_config(),
            options,
        )
        .map_err(|e| e.to_string())?;
        reader.audio_specific_config().to_vec()
    } else {
        return Ok(None);
    };
    let parsed =
        crate::owned_aac::config::AudioSpecificConfig::parse(&asc).map_err(|e| e.to_string())?;
    let ps = channels == 2 && parsed.core.channels == 1;
    crate::owned_aac::speaker_routing::stereo_router(&asc, rate, channels, ps).map(Some)
}
fn check(options: &CopyOptions) -> Result<()> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)
}
pub(crate) fn export(
    source: &Path,
    mut spool: crate::owned_adts_export::DecodedSpool,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    let fold = transform
        .channels
        .is_some_and(|output| output != i32::from(spool.stats.channels) && matches!(output, 1 | 2));
    if fold
        && let Some(router) = router(
            source,
            spool.stats.sample_rate,
            spool.stats.channels,
            options,
        )?
    {
        check(options)?;
        let mut input = File::open(&spool.wave).map_err(|e| e.to_string())?;
        let info = crate::owned_wave_inspect::inspect(&mut input, options.cancel.as_ref())
            .map_err(|e| e.to_string())?;
        if !info.float || info.bits_per_sample != 32 || info.channels != router.input_channels() {
            return Err("AAC routing spool geometry mismatch".into());
        }
        let source_info = crate::owned_wave_inspect::info_chunks(
            &mut input,
            &info,
            options.cancel.as_ref(),
        )
        .map_err(|e| e.to_string())?;
        let chunk_frames = 256usize;
        let bytes = chunk_frames * usize::from(info.channels) * 4;
        let estimate =
            bytes + chunk_frames * 2 * 4 * 2 + 4096 + std::mem::size_of::<StereoRouter>();
        if options
            .max_controlled_bytes
            .is_some_and(|max| estimate > max)
        {
            return Err("controlled memory budget exceeded: AAC spatial routing".into());
        }
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(bytes)
            .map_err(|_| "AAC routing allocation failed")?;
        buffer.resize(bytes, 0);
        let mut packed = Vec::new();
        packed
            .try_reserve_exact(chunk_frames * 8)
            .map_err(|_| "AAC routing allocation failed")?;
        let routed = spool.wave.with_file_name("routed.wav");
        let mut output = File::create_new(&routed).map_err(|e| e.to_string())?;
        output
            .write_all(&crate::owned_wav::float_wav_header_with_mask(
                info.sample_rate,
                2,
                info.sample_frames,
                3,
            )?)
            .map_err(|e| e.to_string())?;
        input
            .seek(SeekFrom::Start(info.data_offset))
            .map_err(|e| e.to_string())?;
        let mut remaining = info.sample_frames;
        while remaining > 0 {
            check(options)?;
            let frames = remaining.min(chunk_frames as u64) as usize;
            let size = frames * usize::from(info.channels) * 4;
            input
                .read_exact(&mut buffer[..size])
                .map_err(|e| e.to_string())?;
            let samples = router.route(&buffer[..size], chunk_frames * 8)?;
            packed.clear();
            packed.extend(samples.into_iter().flat_map(f32::to_le_bytes));
            output.write_all(&packed).map_err(|e| e.to_string())?;
            remaining -= frames as u64;
        }
        output.flush().map_err(|e| e.to_string())?;
        drop(output);
        crate::owned_ogg_opus_export::append_info(&routed, &source_info)?;
        check(options)?;
        spool.wave = routed;
    }
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_only_spatial_admission_accepts_pce_and_emits_no_progress() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let events = Arc::new(AtomicUsize::new(0));
        let captured = events.clone();
        let options = CopyOptions {
            progress: Some(fvid_control::ProgressHook::new(move |_| {
                captured.fetch_add(1, Ordering::Relaxed);
            })),
            ..Default::default()
        };
        let transform = AudioDecodeTransform {
            channels: Some(2),
            ..Default::default()
        };
        assert!(crate::owned_adts_export::supports(
            &root.join("aac-height-mono-bottom.aac"),
            transform,
            &options
        ));
        assert!(crate::owned_mp4_audio_export::supports(
            &root.join("aac-device-layout/normal-ten.mp4"),
            transform,
            &options
        ));
        assert!(crate::owned_matroska_aac_export::supports(
            &root.join("aac-device-layout/layered-front.mkv"),
            transform,
            &options
        ));
        assert_eq!(events.load(Ordering::Relaxed), 0);
    }
}
