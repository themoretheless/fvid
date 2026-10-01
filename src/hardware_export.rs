//! Streaming hardware video encode into the native Matroska muxer.
use crate::container::matroska_write::{Encoding, PacketWriter, TrackSpec};
pub use crate::container::matroska_write::{PacketOptions, TrackSpec as AdditionalTrack};
pub use crate::container::matroska_write::{TrackOptions, VideoMetadata};
use crate::{Error, Result, invalid};
pub use fvid_vt::{EncodedTime, EncoderCodec, Surface};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

struct StagedOutput(PathBuf);
impl Drop for StagedOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn publish_export<T>(
    destination: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<T>,
) -> Result<T> {
    match destination.symlink_metadata() {
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "export destination exists",
            )
            .into());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut staged = None;
    for index in 0..1000 {
        let path = parent.join(format!(".fvid-hardware-{}-{index}.tmp", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                staged = Some((StagedOutput(path), file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    let (temporary, mut file) =
        staged.ok_or_else(|| invalid("cannot create staged hardware export"))?;
    let result = write(&mut file)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    // Same-directory hard link publishes a complete file and atomically refuses
    // an existing name, including one created while encoding was in progress.
    // Do not fall back to rename: POSIX rename can overwrite that competing file.
    std::fs::hard_link(&temporary.0, destination)?;
    Ok(result)
}

/// Publish a completed shader export without replacing an existing path.
/// Failed input/render/encode/mux operations leave no destination file.
#[cfg(feature = "player")]
pub fn write_shader_file(
    destination: &Path,
    frames: impl IntoIterator<Item = Result<ShaderFrame>>,
    renderer: &mut crate::player_gpu::MetalEncoderRenderer,
    device: &eframe::egui_wgpu::wgpu::Device,
    queue: &eframe::egui_wgpu::wgpu::Queue,
    output_size: [usize; 2],
    codec: EncoderCodec,
    bits_per_second: u32,
    options: TrackOptions,
) -> Result<ExportStats> {
    publish_export(destination, |output| {
        write_shader_video(
            output,
            frames,
            renderer,
            device,
            queue,
            output_size,
            codec,
            bits_per_second,
            options,
        )
    })
}

pub struct HardwareFrame {
    pub surface: Surface,
    pub pts: EncodedTime,
    pub duration: EncodedTime,
    pub force_keyframe: bool,
}
/// Native shader input with its source signal and display orientation.
#[cfg(feature = "player")]
pub struct ShaderFrame {
    pub frame: HardwareFrame,
    pub colour: crate::playback_native::AvcColour,
    pub grade: Option<std::sync::Arc<crate::color::Grade>>,
    pub rotation: u16,
}

/// Stream native frames through Metal grading/shaders, hardware encoding and
/// Matroska muxing. Timing and forced keyframes survive rendering unchanged.
/// `options` describe the resulting shader signal, not the source signal.
/// Only compressed bytes cross to CPU; source/output pixel planes stay native.
#[cfg(feature = "player")]
pub fn write_shader_video<W: Write + Seek>(
    output: &mut W,
    frames: impl IntoIterator<Item = Result<ShaderFrame>>,
    renderer: &mut crate::player_gpu::MetalEncoderRenderer,
    device: &eframe::egui_wgpu::wgpu::Device,
    queue: &eframe::egui_wgpu::wgpu::Queue,
    output_size: [usize; 2],
    codec: EncoderCodec,
    bits_per_second: u32,
    options: TrackOptions,
) -> Result<ExportStats> {
    let rendered = frames.into_iter().enumerate().map(|(serial, input)| {
        let input = input?;
        let surface = renderer.render_surface_sized(
            device,
            queue,
            &input.frame.surface,
            input.colour,
            u64::try_from(serial).map_err(|_| invalid("shader frame serial overflow"))?,
            input.grade.as_ref(),
            input.rotation,
            output_size,
        )?;
        Ok(HardwareFrame {
            surface,
            pts: input.frame.pts,
            duration: input.frame.duration,
            force_keyframe: input.frame.force_keyframe,
        })
    });
    write_video_with_options(output, rendered, codec, bits_per_second, options)
}
#[derive(Debug, Default)]
pub struct ExportStats {
    pub frames: u64,
    pub compressed_bytes: u64,
}
/// An encoded packet on an additional track's already mapped output timeline.
pub struct AdditionalPacket {
    /// Zero-based index into `additional_tracks` (video is inserted separately).
    pub track: usize,
    pub pts_ns: u64,
    pub duration_ns: u64,
    pub key_frame: bool,
    pub data: Vec<u8>,
    pub options: PacketOptions,
}

/// Video track streaming API; the caller supplies all timestamps and handles
/// destination publication. Audio/subtitles are separate tracks, not implicitly
/// copied. Pixel surfaces remain native; only compressed packets reach the CPU.
pub fn write_video<W: Write + Seek>(
    output: &mut W,
    frames: impl IntoIterator<Item = Result<HardwareFrame>>,
    codec: EncoderCodec,
    bits_per_second: u32,
) -> Result<ExportStats> {
    write_video_with_options(
        output,
        frames,
        codec,
        bits_per_second,
        TrackOptions::default(),
    )
}
/// Encode native frames and publish a complete Matroska file without overwrite.
pub fn write_video_file(
    destination: &Path,
    frames: impl IntoIterator<Item = Result<HardwareFrame>>,
    codec: EncoderCodec,
    bits_per_second: u32,
    options: TrackOptions,
) -> Result<ExportStats> {
    publish_export(destination, |output| {
        write_video_with_options(output, frames, codec, bits_per_second, options)
    })
}
/// Publish hardware video and pre-encoded additional tracks without overwrite.
pub fn write_video_file_with_tracks(
    destination: &Path,
    frames: impl IntoIterator<Item = Result<HardwareFrame>>,
    codec: EncoderCodec,
    bits_per_second: u32,
    options: TrackOptions,
    additional_tracks: Vec<AdditionalTrack<'_>>,
    additional_options: Vec<TrackOptions>,
    additional_packets: impl IntoIterator<Item = Result<AdditionalPacket>>,
) -> Result<ExportStats> {
    publish_export(destination, |output| {
        write_video_with_tracks(
            output,
            frames,
            codec,
            bits_per_second,
            options,
            additional_tracks,
            additional_options,
            additional_packets,
        )
    })
}
/// Copy all AAC tracks from an eligible MP4, applying the native edit/gapless plan.
/// Video frames must already use the same edited presentation timeline.
pub fn write_video_file_with_mp4_audio<R: Read + Seek>(
    destination: &Path,
    frames: impl IntoIterator<Item = Result<HardwareFrame>>,
    codec: EncoderCodec,
    bits_per_second: u32,
    options: TrackOptions,
    mut input: crate::container::mp4::Mp4Reader<R>,
) -> Result<ExportStats> {
    use crate::container::mp4_matroska;
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    if !mp4_matroska::eligible(&input) {
        return Err(invalid(
            "audio copy requires AVC/HEVC/AAC MP4 with contiguous media edits",
        ));
    }
    let selected: Vec<_> = input
        .tracks()
        .iter()
        .enumerate()
        .filter(|(_, t)| t.handler == *b"soun")
        .map(|(i, t)| (i, t.clone()))
        .collect();
    let plans: Vec<_> = selected
        .iter()
        .map(|(_, t)| mp4_matroska::plan(t, input.movie_timescale(), None))
        .collect::<Result<_>>()?;
    let specs = selected
        .iter()
        .map(|(_, t)| mp4_matroska::spec(t))
        .collect::<Result<Vec<_>>>()?;
    let track_options = plans.iter().map(|p| p.options.clone()).collect();
    let mut heap = BinaryHeap::new();
    for (track, plan) in plans.iter().enumerate() {
        if let Some(first) = plan.packets.first() {
            heap.push(Reverse((first.pts, track, 0usize)));
        }
    }
    let packets = std::iter::from_fn(|| {
        let Reverse((pts_ns, track, sample)) = heap.pop()?;
        let time = &plans[track].packets[sample];
        if let Some(next) = plans[track].packets.get(sample + 1) {
            heap.push(Reverse((next.pts, track, sample + 1)));
        }
        let mut data = Vec::new();
        Some(
            input
                .read_packet(selected[track].0, sample, &mut data)
                .map(|()| AdditionalPacket {
                    track,
                    pts_ns,
                    duration_ns: time.duration,
                    key_frame: true,
                    data,
                    options: time.options,
                }),
        )
    });
    write_video_file_with_tracks(
        destination,
        frames,
        codec,
        bits_per_second,
        options,
        specs,
        track_options,
        packets,
    )
}

/// Describe the output signal, HDR data and display geometry explicitly.
/// These options belong to the encoded result, including any applied shader.
pub fn write_video_with_options<W: Write + Seek>(
    output: &mut W,
    frames: impl IntoIterator<Item = Result<HardwareFrame>>,
    codec: EncoderCodec,
    bits_per_second: u32,
    options: TrackOptions,
) -> Result<ExportStats> {
    write_video_with_tracks(
        output,
        frames,
        codec,
        bits_per_second,
        options,
        Vec::new(),
        Vec::new(),
        std::iter::empty(),
    )
}

/// Merge pre-encoded audio/subtitle packets with hardware video in PTS order.
/// Additional packets must be globally nondecreasing; only one is held ahead.
/// Callers map edits, delay, padding and duration to the output timeline.
pub fn write_video_with_tracks<W: Write + Seek>(
    output: &mut W,
    frames: impl IntoIterator<Item = Result<HardwareFrame>>,
    codec: EncoderCodec,
    bits_per_second: u32,
    options: TrackOptions,
    additional_tracks: Vec<AdditionalTrack<'_>>,
    mut additional_options: Vec<TrackOptions>,
    additional_packets: impl IntoIterator<Item = Result<AdditionalPacket>>,
) -> Result<ExportStats> {
    if additional_tracks.len() != additional_options.len() {
        return Err(invalid("additional track options mismatch"));
    }
    let additional_count = additional_tracks.len();
    let mut additional = additional_packets.into_iter().peekable();
    let mut previous_additional = None;
    if options.codec_delay_ns != 0 {
        return Err(invalid(
            "video hardware export does not take audio codec delay",
        ));
    }
    let mut frames = frames.into_iter();
    let hdr_nal = if matches!(codec, EncoderCodec::Hevc) {
        crate::codec::hevc_sei::output_hdr_nal(
            &options
                .video
                .as_ref()
                .map_or_else(Default::default, |video| video.hdr),
        )?
    } else {
        Vec::new()
    };
    let first = frames
        .next()
        .ok_or_else(|| invalid("hardware export requires a frame"))??;
    let width =
        u32::try_from(first.surface.width()).map_err(|_| invalid("export width overflow"))?;
    let height =
        u32::try_from(first.surface.height()).map_err(|_| invalid("export height overflow"))?;
    let full_range = first.surface.full_range();
    let mut encoder = fvid_vt::Encoder::new_with_bitrate(
        codec,
        width as usize,
        height as usize,
        first.surface.depth(),
        bits_per_second,
    )
    .map_err(gpu)?;
    if let Some(colour) = options.video.as_ref().and_then(|video| video.colour) {
        encoder
            .set_colour(colour.primaries, colour.transfer, colour.matrix)
            .map_err(gpu)?;
    }
    let first = encode(&mut encoder, first)?;
    let configuration = &first.decoder_configuration;
    let encoding = match codec {
        EncoderCodec::H264 => Encoding::Avc {
            configuration,
            width,
            height,
        },
        EncoderCodec::Hevc => Encoding::Hevc {
            configuration,
            width,
            height,
        },
    };
    let mut tracks = vec![TrackSpec {
        encoding,
        name: "hardware video",
        language: "und",
    }];
    tracks.extend(additional_tracks.iter().map(|track| TrackSpec {
        encoding: track.encoding,
        name: track.name,
        language: track.language,
    }));
    additional_options.insert(0, options);
    let mut writer = PacketWriter::new_with_options(output, &tracks, &additional_options)?;
    let mut stats = ExportStats::default();
    let mut previous = None;
    let mut write = |frame: &fvid_vt::EncodedFrame| -> Result<()> {
        if &frame.decoder_configuration != configuration {
            return Err(invalid(
                "hardware encoder configuration changed during export",
            ));
        }
        let pts = u64::try_from(frame.pts.nanoseconds().map_err(gpu)?).map_err(|_| {
            invalid("export needs a nonnegative timeline; offset negative PTS explicitly")
        })?;
        let duration = u64::try_from(frame.duration.nanoseconds().map_err(gpu)?)
            .map_err(|_| invalid("negative encoded duration"))?;
        if duration == 0 || previous.is_some_and(|value| pts <= value) {
            return Err(invalid(
                "export requires increasing PTS and positive nanosecond duration",
            ));
        }
        let mut packet = Vec::new();
        let bytes = if frame.key_frame && !hdr_nal.is_empty() {
            let prefix = frame.nal_length_size as usize;
            let length =
                u32::try_from(hdr_nal.len()).map_err(|_| invalid("HDR SEI size overflow"))?;
            if ![1, 2, 4].contains(&prefix) || (prefix < 4 && length >= 1u32 << (prefix * 8)) {
                return Err(invalid("HDR SEI does not fit NAL length prefix"));
            }
            packet.extend_from_slice(&length.to_be_bytes()[4 - prefix..]);
            packet.extend_from_slice(&hdr_nal);
            packet.extend_from_slice(&frame.data);
            packet.as_slice()
        } else {
            frame.data.as_slice()
        };
        while additional
            .peek()
            .is_some_and(|next| next.as_ref().map_or(true, |packet| packet.pts_ns <= pts))
        {
            write_additional(
                &mut writer,
                additional.next().unwrap()?,
                additional_count,
                &mut previous_additional,
            )?;
        }
        writer.write_packet(0, pts, duration, frame.key_frame, bytes)?;
        previous = Some(pts);
        stats.frames += 1;
        stats.compressed_bytes = stats
            .compressed_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| invalid("export byte count overflow"))?;
        Ok(())
    };
    write(&first)?;
    for frame in frames {
        let frame = frame?;
        if frame.surface.full_range() != full_range {
            return Err(invalid("hardware surface range changed during export"));
        }
        write(&encode(&mut encoder, frame)?)?;
    }
    drop(write);
    for packet in additional {
        write_additional(
            &mut writer,
            packet?,
            additional_count,
            &mut previous_additional,
        )?;
    }
    writer.finish()?;
    Ok(stats)
}
fn write_additional<W: Write + Seek>(
    writer: &mut PacketWriter<'_, W>,
    packet: AdditionalPacket,
    count: usize,
    previous: &mut Option<u64>,
) -> Result<()> {
    if packet.track >= count || previous.is_some_and(|pts| packet.pts_ns < pts) {
        return Err(invalid("invalid additional track or decreasing packet PTS"));
    }
    writer.write_packet_with_options(
        packet.track + 1,
        packet.pts_ns,
        packet.duration_ns,
        packet.key_frame,
        &packet.data,
        packet.options,
    )?;
    *previous = Some(packet.pts_ns);
    Ok(())
}
fn gpu(error: fvid_vt::Error) -> Error {
    Error::Gpu(error.to_string())
}
fn encode(encoder: &mut fvid_vt::Encoder, frame: HardwareFrame) -> Result<fvid_vt::EncodedFrame> {
    let scale = i32::try_from(frame.pts.timescale)
        .map_err(|_| invalid("encoder timescale exceeds signed 32-bit range"))?;
    if scale == 0 || frame.duration.timescale == 0 {
        return Err(invalid("zero hardware frame timescale"));
    }
    let scaled = i128::from(frame.duration.value) * i128::from(scale);
    if scaled % i128::from(frame.duration.timescale) != 0 {
        return Err(invalid("duration cannot be represented in PTS timescale"));
    }
    let duration = i64::try_from(scaled / i128::from(frame.duration.timescale))
        .map_err(|_| invalid("hardware duration overflow"))?;
    encoder
        .encode_with_keyframe(
            &frame.surface,
            frame.pts.value,
            duration,
            scale,
            frame.force_keyframe,
        )
        .map_err(gpu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_cleans_failures_and_preserves_competing_destinations() {
        let root = std::env::temp_dir().join(format!(
            "fvid-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let destination = root.join("result.mkv");
        let failed: Result<()> = publish_export(&destination, |file| {
            file.write_all(b"incomplete mux")?;
            Err(invalid("input failure"))
        });
        assert!(failed.is_err());
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);

        let raced: Result<()> = publish_export(&destination, |file| {
            file.write_all(b"complete mux")?;
            std::fs::write(&destination, b"other writer")?;
            Ok(())
        });
        assert!(raced.is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"other writer");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        assert!(
            publish_export(&destination, |_| -> Result<()> {
                panic!("must reject before consuming input")
            })
            .is_err()
        );
        std::fs::remove_file(&destination).unwrap();

        let value = publish_export(&destination, |file| {
            file.write_all(b"complete mux")?;
            Ok(42)
        })
        .unwrap();
        assert_eq!(value, 42);
        assert_eq!(std::fs::read(&destination).unwrap(), b"complete mux");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    }
}
