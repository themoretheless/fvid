//! Ogg Opus file extraction through the shared native program and WAVE DSP.
use crate::owned_ogg_opus::{AudioPacket, AudioStep, Program};
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn check(options: &CopyOptions) -> Result<()> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)
}
pub(crate) fn admit(need: usize, options: &CopyOptions) -> Result<()> {
    if options
        .max_controlled_bytes
        .is_some_and(|limit| need > limit)
    {
        return Err(format!(
            "controlled memory budget exceeded: need {need} bytes, limit {}",
            options.max_controlled_bytes.unwrap()
        ));
    }
    Ok(())
}
pub(crate) fn recognizes(source: &Path) -> Result<bool> {
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut signature = [0; 4];
    Ok(file.read_exact(&mut signature).is_ok() && signature == *b"OggS")
}
// Count index geometry without retaining payload. The reserve covers input and
// parsed payload growth, lossy-UTF8/INFO metadata copies, vector growth, routes,
// and fixed I/O. Decoder/output storage is charged after header qualification.
pub(crate) fn preflight(
    file: &mut File,
    options: &CopyOptions,
) -> Result<(crate::owned_ogg::Limits, usize)> {
    check(options)?;
    admit(256 * 1024, options)?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let mut at = 0u64;
    let mut packets = 0usize;
    let mut streams = 0usize;
    let mut pages = 0usize;
    while at < length {
        if pages % 256 == 0 {
            check(options)?;
        }
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        let mut header = [0; 27];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        if &header[..4] != b"OggS" || header[4] != 0 {
            return Err("invalid Ogg page during index admission".into());
        }
        let count = usize::from(header[26]);
        let mut table = [0; 255];
        file.read_exact(&mut table[..count])
            .map_err(|e| e.to_string())?;
        let payload: usize = table[..count].iter().map(|&n| usize::from(n)).sum();
        packets = packets
            .checked_add(table[..count].iter().filter(|&&n| n < 255).count())
            .ok_or("Ogg packet count overflow")?;
        streams = streams
            .checked_add(usize::from(header[5] & 2 != 0))
            .ok_or("Ogg stream count overflow")?;
        at = at
            .checked_add(27 + count as u64 + payload as u64)
            .filter(|&end| end <= length)
            .ok_or("Ogg page exceeds input")?;
        file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        pages += 1;
    }
    let length = usize::try_from(length).map_err(|_| "Ogg file size overflow")?;
    let estimate = length
        .checked_mul(16)
        .and_then(|n| packets.checked_mul(384).and_then(|p| n.checked_add(p)))
        .and_then(|n| streams.checked_mul(8192).and_then(|p| n.checked_add(p)))
        .and_then(|n| n.checked_add(256 * 1024))
        .ok_or("Ogg index admission overflow")?;
    admit(estimate, options)?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    Ok((
        crate::owned_ogg::Limits {
            file_bytes: length,
            packets,
        },
        estimate,
    ))
}
pub(crate) struct CheckedRead<'a> {
    pub(crate) input: BufReader<File>,
    pub(crate) options: &'a CopyOptions,
}
impl Read for CheckedRead<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        check(self.options).map_err(std::io::Error::other)?;
        self.input.read(bytes)
    }
}
pub(crate) fn open(source: &Path, options: &CopyOptions) -> Result<Program> {
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err("select exactly one audio stream".into()),
    };
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let (limits, index) = preflight(&mut file, options)?;
    let program = Program::open_source_track(
        CheckedRead {
            input: BufReader::new(file),
            options,
        },
        limits,
        selected,
    )
    .map_err(|e| e.to_string())?;
    let decoder = program
        .decoder_admission_bytes()
        .map_err(|e| e.to_string())?;
    let estimate = decoder
        .checked_mul(2)
        .and_then(|n| n.checked_add(index))
        .and_then(|n| n.checked_add(usize::from(program.channels()) * 5760 * 8 + 64 * 1024))
        .ok_or("Ogg decoder admission overflow")?;
    admit(estimate, options)?;
    check(options)?;
    Ok(program)
}
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    match open(source, options) {
        Ok(program) => {
            let target = transform.channels.unwrap_or(i32::from(program.channels()));
            target == i32::from(program.channels())
                || (crate::owned_pcm_channels::rematrix_layout_supported(
                    program.channels(),
                    program.output_channel_mask().unwrap_or(0),
                ) && ((program.channels() <= 8 && matches!(target, 1 | 2))
                    || (program.channels() <= 2 && (1..=8).contains(&target))))
        }
        Err(e) => e.starts_with("controlled memory budget exceeded:"),
    }
}
fn decode(
    program: &mut Program,
    output: &mut impl Write,
    until: Option<u64>,
    options: &CopyOptions,
) -> Result<crate::owned_matroska_audio::AudioDecodeStats> {
    check(options)?;
    let mut decoder = program.native_decoder().map_err(|e| e.to_string())?;
    let mut event = ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    let mut stats = crate::owned_matroska_audio::AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: 48000,
        channels: program.channels(),
    };
    loop {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        if event.packets % 256 == 0 {
            check(options)?;
        }
        if options
            .max_packets
            .is_some_and(|limit| event.packets >= limit)
        {
            break;
        }
        if until.is_some_and(|end| {
            program
                .next_presentation_start()
                .is_some_and(|p| p >= 0 && p as u64 >= end)
        }) {
            break;
        }
        if program
            .next_packet_size()
            .is_some_and(|size| size > options.max_packet_bytes)
        {
            return Err("Opus compressed packet exceeds byte limit".into());
        }
        let Some(step) = program.next_step().map_err(|e| e.to_string())? else {
            break;
        };
        match step {
            AudioStep::ResetDecoder => {
                decoder = program.native_decoder().map_err(|e| e.to_string())?
            }
            AudioStep::Encoded(packet) => {
                let data = decoder.decode_bytes(&packet.data)?;
                stats.decoded_frames = stats
                    .decoded_frames
                    .checked_add(1)
                    .ok_or("Opus frame count overflow")?;
                event.packets = stats.decoded_frames;
                event.payload_bytes = event
                    .payload_bytes
                    .checked_add(packet.data.len() as u64)
                    .ok_or("Opus payload count overflow")?;
                if let Some(hook) = &options.progress {
                    hook.emit(event);
                }
                if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                    return Err("media operation cancelled".into());
                }
                if let Some(mut pcm) = program
                    .present_decoded(
                        AudioPacket {
                            data,
                            pts: 0,
                            timebase_num: 1,
                            timebase_den: 48000,
                        },
                        packet.pts,
                    )
                    .map_err(|e| e.to_string())?
                {
                    let width = usize::from(program.channels()) * 4;
                    if let Some(end) = until {
                        let frames = usize::try_from(end.saturating_sub(pcm.pts))
                            .unwrap_or(usize::MAX)
                            .min(pcm.data.len() / width);
                        pcm.data.truncate(frames * width);
                    }
                    output.write_all(&pcm.data).map_err(|e| e.to_string())?;
                    stats.sample_frames = stats
                        .sample_frames
                        .checked_add((pcm.data.len() / width) as u64)
                        .ok_or("Opus sample count overflow")?;
                }
            }
        }
    }
    Ok(stats)
}
fn source_info(program: &Program) -> Result<Vec<u8>> {
    let tags = program.source_tags().map_err(|e| e.to_string())?;
    info_for_tags(&tags)
}
pub(crate) fn info_for_tags(tags: &fvid_opus::OpusTags) -> Result<Vec<u8>> {
    let mut entries = Vec::new();
    for (source, target) in [
        ("TITLE", "title"),
        ("ARTIST", "artist"),
        ("COMMENT", "comment"),
        ("DATE", "date"),
        ("ALBUM", "album"),
        ("GENRE", "genre"),
        ("COPYRIGHT", "copyright"),
        ("ENCODER", "encoder"),
        ("TRACKNUMBER", "track"),
        ("LANGUAGE", "ILNG"),
    ] {
        if let Some(value) = tags.get(source) {
            if !value.contains('\0') {
                entries.push((target.to_string(), value.to_string()));
            }
        }
    }
    crate::owned_wave_metadata::edit_info_chunks(&[], &[], &entries)
}
pub(crate) fn append_info(path: &Path, info: &[u8]) -> Result<()> {
    if info.is_empty() {
        return Ok(());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let total = file
        .metadata()
        .map_err(|e| e.to_string())?
        .len()
        .checked_add(info.len() as u64)
        .ok_or("WAVE metadata size overflow")?;
    let riff = u32::try_from(total.checked_sub(8).ok_or("invalid private WAVE size")?)
        .map_err(|_| "WAVE metadata exceeds RIFF size")?;
    file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    file.write_all(info).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(4)).map_err(|e| e.to_string())?;
    file.write_all(&riff.to_le_bytes())
        .map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())
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
    check(options)?;
    let mut program = open(source, options)?;
    let channels = program.channels();
    let mask = program.output_channel_mask().unwrap_or(0);
    let info = source_info(&program)?;
    let until = crate::owned_adts_export::decoded_prefix(transform, 48000)
        .map(|(_, end)| {
            u64::try_from((end.as_nanos() * 48000).div_ceil(1_000_000_000))
                .map_err(|_| "Opus prefix duration overflow")
        })
        .transpose()?;
    let spool = crate::owned_adts_export::spool_decoded(
        48000,
        channels,
        mask,
        options,
        move |writer, options| decode(&mut program, writer, until, options),
    )?;
    check(options)?;
    append_info(&spool.wave, &info)?;
    drop(info);
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}
