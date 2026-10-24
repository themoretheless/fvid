//! Controlled Ogg Vorbis extraction through owned presentation and WAVE DSP.
use crate::owned_ogg_opus::{AudioPacket, AudioStep};
use crate::owned_ogg_opus_export::{CheckedRead, admit, check, preflight};
use crate::owned_ogg_vorbis::Program;
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read, Write},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn open(source: &Path, options: &CopyOptions) -> Result<(Program, Vec<u8>)> {
    check(options)?;
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let (limits, index) = preflight(&mut file, options)?;
    let mut bytes = Vec::new();
    CheckedRead {
        input: BufReader::new(file),
        options,
    }
    .read_to_end(&mut bytes)
    .map_err(|e| e.to_string())?;
    let ogg = crate::owned_ogg::Ogg::parse(&bytes, &limits).map_err(|e| e.to_string())?;
    drop(bytes);
    let selection = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err("select exactly one Vorbis audio stream".into()),
    };
    let selected = crate::owned_ogg_vorbis::select_source_chains(&ogg, selection)
        .map_err(|e| e.to_string())?;
    let first = selected[0];
    let output_rate =
        fvid_codecs::vorbis::Identification::parse(&ogg.chains()[first].packets[0].data)
            .map_err(|e| e.to_string())?
            .sample_rate;
    let mut reserve = 0usize;
    for index in selected {
        let chain = &ogg.chains()[index];
        let setup = chain.packets.get(2).ok_or("missing Vorbis setup")?;
        let id = fvid_codecs::vorbis::Identification::parse(&chain.packets[0].data)
            .map_err(|e| e.to_string())?;
        let admission =
            fvid_codecs::vorbis_admission::inspect(&setup.data).map_err(|e| e.to_string())?;
        let n = 1usize << id.block_exponents[1];
        let ch = usize::from(id.channels);
        let dimension = admission.max_dimension;
        // The sinc engine keeps 255-wide history frames, independently of
        // active channels. Reserve its exact rate-dependent history before
        // constructing decoders or admitting any converted PCM packet.
        let radius = (32u128 * u128::from(id.sample_rate)).div_ceil(u128::from(output_rate)) + 1;
        let step = u128::from(id.sample_rate).div_ceil(u128::from(output_rate));
        let history = (2 * radius + step + 3) * 255 * 4;
        let output_frames =
            (n as u128 * u128::from(output_rate)).div_ceil(u128::from(id.sample_rate)) + 2;
        let converted = output_frames * 255 * 4;
        let conversion = usize::try_from(history + converted + 64 * 1024)
            .map_err(|_| "Vorbis resample reserve overflow")?;
        let workspace = conversion
            + ch * n * 32
            + n * 40
            + 64 * 1024
            + 16 * (ch * (n / 2 + dimension) + dimension + ch * n / 2);
        reserve = reserve.max(
            admission
                .bytes
                .checked_add(workspace)
                .ok_or("Vorbis decoder reserve overflow")?,
        );
    }
    admit(
        index
            .checked_add(
                reserve
                    .checked_mul(2)
                    .ok_or("Vorbis state reserve overflow")?,
            )
            .and_then(|n| n.checked_add(512 * 1024))
            .ok_or("Vorbis export reserve overflow")?,
        options,
    )?;
    let comment = &ogg.chains()[first].packets[1].data;
    if !comment.starts_with(b"\x03vorbis") {
        return Err("missing Vorbis comment header".into());
    }
    let mut tagged = b"OpusTags".to_vec();
    tagged.extend_from_slice(&comment[7..]);
    let tags = fvid_opus::OpusTags::parse(&tagged).map_err(|e| e.to_string())?;
    let info = crate::owned_ogg_opus_export::info_for_tags(&tags)?;
    drop(ogg);
    let program = Program::open_source_track(
        CheckedRead {
            input: BufReader::new(File::open(source).map_err(|e| e.to_string())?),
            options,
        },
        limits,
        selection,
    )
    .map_err(|e| e.to_string())?;
    check(options)?;
    Ok((program, info))
}
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    match open(source, options) {
        Ok((p, _)) => {
            let target = transform.channels.unwrap_or(i32::from(p.channels()));
            target == i32::from(p.channels())
                || (((p.channels() <= 8 && matches!(target, 1 | 2))
                    || (p.channels() <= 2 && (1..=8).contains(&target)))
                    && ((p.channels() <= 2 && p.output_channel_mask() == 0)
                        || (p.output_channel_mask() != 0
                            && crate::owned_pcm_channels::rematrix_layout_supported(
                                p.channels(),
                                p.output_channel_mask(),
                            ))))
        }
        Err(e) => e.starts_with("controlled memory budget exceeded:"),
    }
}
fn decode(
    program: &mut Program,
    writer: &mut impl Write,
    until: Option<u64>,
    options: &CopyOptions,
) -> Result<crate::owned_matroska_audio::AudioDecodeStats> {
    check(options)?;
    let mut decoder = program.native_decoder().map_err(|e| e.to_string())?;
    let mut stats = crate::owned_matroska_audio::AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: program.sample_rate(),
        channels: program.channels(),
    };
    let mut event = ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    let channels = usize::from(program.channels());
    let width = channels * 4;
    let permutation = program.wave_channel_order().to_vec();
    let silence = vec![0u8; 1024 * width];
    loop {
        check(options)?;
        if options.max_packets.is_some_and(|n| event.packets >= n) {
            break;
        }
        if until.is_some_and(|end| stats.sample_frames >= end) {
            break;
        }
        if let Some(end) = until {
            if !program.has_rate_changes()
                && program
                    .next_presentation_start()
                    .is_some_and(|p| p >= 0 && p as u64 >= end)
            {
                while stats.sample_frames < end {
                    check(options)?;
                    let frames = (end - stats.sample_frames).min(1024) as usize;
                    writer
                        .write_all(&silence[..frames * width])
                        .map_err(|e| e.to_string())?;
                    stats.sample_frames += frames as u64;
                }
                break;
            }
        }
        if program
            .next_packet_size()
            .is_some_and(|size| size > options.max_packet_bytes)
        {
            return Err("Vorbis compressed packet exceeds byte limit".into());
        }
        let Some(step) = program.next_step().map_err(|e| e.to_string())? else {
            break;
        };
        match step {
            AudioStep::ResetDecoder => {
                decoder = program.native_decoder().map_err(|e| e.to_string())?
            }
            AudioStep::Encoded(packet) => {
                let samples = decoder.decode(&packet.data).map_err(|e| e.to_string())?;
                stats.decoded_frames += 1;
                event.packets = stats.decoded_frames;
                event.payload_bytes += packet.data.len() as u64;
                if let Some(hook) = &options.progress {
                    hook.emit(event);
                }
                check(options)?;
                if samples.is_empty() {
                    continue;
                }
                let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                if let Some(pcm) = program
                    .present_decoded(
                        AudioPacket {
                            data,
                            pts: 0,
                            timebase_num: 1,
                            timebase_den: stats.sample_rate,
                        },
                        packet.pts,
                    )
                    .map_err(|e| e.to_string())?
                {
                    write_pcm(
                        pcm,
                        writer,
                        until,
                        &permutation,
                        &silence,
                        &mut stats,
                        options,
                    )?;
                }
            }
        }
    }
    if !until.is_some_and(|end| stats.sample_frames >= end) {
        if let Some(pcm) = program.finish_pcm_prefix().map_err(|e| e.to_string())? {
            write_pcm(
                pcm,
                writer,
                until,
                &permutation,
                &silence,
                &mut stats,
                options,
            )?;
        }
    }
    Ok(stats)
}
fn write_pcm(
    mut pcm: AudioPacket,
    writer: &mut impl Write,
    until: Option<u64>,
    permutation: &[usize],
    silence: &[u8],
    stats: &mut crate::owned_matroska_audio::AudioDecodeStats,
    options: &CopyOptions,
) -> Result<()> {
    check(options)?;
    let width = usize::from(stats.channels) * 4;
    let gap_end = until.map_or(pcm.pts, |end| end.min(pcm.pts));
    while stats.sample_frames < gap_end {
        check(options)?;
        let frames = (gap_end - stats.sample_frames).min(1024) as usize;
        writer
            .write_all(&silence[..frames * width])
            .map_err(|e| e.to_string())?;
        stats.sample_frames += frames as u64;
    }
    let frames = (pcm.data.len() / width)
        .min(until.map_or(usize::MAX, |end| end.saturating_sub(pcm.pts) as usize));
    pcm.data.truncate(frames * width);
    let mut frame = [0u8; 255 * 4];
    for bytes in pcm.data.chunks_exact_mut(width) {
        frame[..width].copy_from_slice(bytes);
        for (to, &from) in permutation.iter().enumerate() {
            bytes[to * 4..to * 4 + 4].copy_from_slice(&frame[from * 4..from * 4 + 4]);
        }
    }
    writer.write_all(&pcm.data).map_err(|e| e.to_string())?;
    stats.sample_frames += frames as u64;
    Ok(())
}
pub(crate) fn decode_to_wave(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    check(options)?;
    let (mut program, _) = open(source, options)?;
    let rate = program.sample_rate();
    let channels = program.channels();
    let mask = program.output_channel_mask();
    crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        move |writer, options| decode(&mut program, writer, None, options),
    )
}
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if destination.symlink_metadata().is_ok() {
        return Err("destination already exists".into());
    }
    check(options)?;
    let (mut program, info) = open(source, options)?;
    let rate = program.sample_rate();
    let channels = program.channels();
    let until = crate::owned_adts_export::decoded_prefix(transform, rate)
        .map(|(_, end)| {
            u64::try_from((end.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000))
                .map_err(|_| "Vorbis prefix overflow")
        })
        .transpose()?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        program.output_channel_mask(),
        options,
        move |writer, options| decode(&mut program, writer, until, options),
    )?;
    check(options)?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
    drop(info);
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}
