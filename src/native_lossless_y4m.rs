//! Owned planar Y4M and AVC/HEVC/VP9/AV1/FFV1 Matroska to FFV1 with audio retention.
use crate::container::matroska_write::{
    Encoding, PacketWriter, TrackOptions, TrackSpec, VideoMetadata,
};
use crate::{
    Result, invalid,
    native_geometry::VideoGeometry,
    native_pixels::PixelFilters,
    playback_native::{NativeReader, RawFrame},
};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::{
    fs::File,
    io::{BufReader, Read, Seek, Write},
    path::Path,
};
pub fn is_source(source: &Path) -> Result<bool> {
    let mut file = File::open(source)?;
    let mut magic = [0; 9];
    match file.read_exact(&mut magic) {
        Ok(()) => Ok(&magic == b"YUV4MPEG2"),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}
/// Admission preserves the legacy path for Y4M profiles not owned yet.
pub fn eligible(source: &Path) -> Result<bool> {
    if !is_source(source)? {
        if !crate::native_export::is_matroska_source(source)? {
            return Ok(false);
        }
        let mut input = crate::container::webm::WebmReader::open(
            BufReader::new(File::open(source)?),
            Default::default(),
        )?;
        input.scan_all()?;
        let videos: Vec<_> = input.tracks.iter().filter(|t| t.kind == 1).collect();
        return Ok(videos.len() == 1
            && matches!(
                videos[0].codec.as_str(),
                "V_VP9" | "V_AV1" | "V_MPEG4/ISO/AVC" | "V_MPEGH/ISO/HEVC" | "V_FFV1"
            )
            && input.tracks.iter().all(|t| {
                t.kind == 1
                    || (t.kind == 2
                        && ((t.codec == "A_AAC"
                            && crate::codec::config::AacConfig::parse(&t.codec_private).is_ok())
                            || (t.codec == "A_OPUS"
                                && crate::container::opus_packet::header_channels(
                                    &t.codec_private,
                                )
                                .is_ok_and(|c| u64::from(c) == t.channels)
                                && crate::container::opus_packet::pre_skip_ns(&t.codec_private)
                                    .is_ok_and(|delay| delay == t.codec_delay_ns)
                                && u32::from_le_bytes(
                                    t.codec_private[12..16].try_into().unwrap(),
                                ) != 0)))
            })
            && input.packets.iter().all(|p| p.pts_ns >= 0));
    }
    let mut input = BufReader::new(File::open(source)?);
    let mut line = Vec::new();
    crate::line(&mut input, &mut line)?;
    let Ok(header) = crate::Header::parse(&line) else {
        return Ok(false);
    };
    Ok(header.tokens.iter().any(|t| t.starts_with('F'))
        && !header
            .tokens
            .iter()
            .any(|t| matches!(t.as_str(), "C420mpeg2" | "C420paldv")))
}
fn check(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(CancelFlag::is_cancelled) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}
pub fn write<W: Write + Seek>(
    source: &Path,
    output: &mut W,
    geometry: &VideoGeometry,
    filters: &PixelFilters,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
 ) -> Result<(crate::media_info::LosslessStats, ProgressEvent)> {
    write_processed(source,output,geometry,filters,cancel,progress,None)
}

/// Process display-oriented sample planes before owned FFV1 encoding.
/// Retain source timing, metadata and supported AAC/Opus companion tracks.
pub fn write_processed<W: Write + Seek>(
    source:&Path,output:&mut W,geometry:&VideoGeometry,filters:&PixelFilters,
    cancel:Option<&CancelFlag>,progress:Option<&ProgressHook>,
    mut processor:Option<&mut dyn FnMut(&mut crate::native_geometry::GeometryFrame,u8,u64)->Result<()>>,
)->Result<(crate::media_info::LosslessStats,ProgressEvent)> {
    check(cancel)?;
    if !eligible(source)? {
        return Err(invalid("unsupported owned planar lossless source"));
    }
    let mut webm = if is_source(source)? {
        None
    } else {
        Some(crate::container::webm::WebmReader::open(
            BufReader::new(File::open(source)?),
            Default::default(),
        )?)
    };
    if let Some(input) = webm.as_mut() {
        input.scan_all()?;
    }
    let video_index = webm
        .as_ref()
        .and_then(|r| r.tracks.iter().position(|t| t.kind == 1))
        .unwrap_or(0);
    let video_origin = webm
        .as_ref()
        .and_then(|r| {
            r.packets
                .iter()
                .filter(|p| p.track == r.tracks[video_index].number && !p.invisible)
                .map(|p| p.pts_ns)
                .min()
        })
        .unwrap_or(0) as u64;
    let mut reader = NativeReader::software(BufReader::new(File::open(source)?), usize::MAX)?;
    let first = reader
        .read_frame_raw()?
        .ok_or_else(|| invalid("input has no video frames"))?;
    let [w, h] = reader.dimensions();
    let rotation = reader.rotation();
    let source_colour = reader.colour();
    let source_full_range = source_colour.full_range;
    let processed = processor.is_some();
    let bake_rotation = processor.is_some() || !geometry.is_identity() || !filters.is_empty();
    let (coded_w, coded_h) = if matches!(rotation, 90 | 270) {
        (h, w)
    } else {
        (w, h)
    };

    let crop = webm
        .as_ref()
        .map(|r| r.tracks[video_index].crop)
        .unwrap_or([0; 4]);
    let crop = crop
        .map(usize::try_from)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| invalid("stored crop overflow"))?;
    let crop: [usize; 4] = crop.try_into().unwrap();
    let visible_w = coded_w
        .checked_sub(crop[0])
        .and_then(|n| n.checked_sub(crop[2]))
        .filter(|&n| n > 0)
        .ok_or_else(|| invalid("stored crop removes picture width"))?;
    let visible_h = coded_h
        .checked_sub(crop[1])
        .and_then(|n| n.checked_sub(crop[3]))
        .filter(|&n| n > 0)
        .ok_or_else(|| invalid("stored crop removes picture height"))?;
    let (display_w, display_h) = if matches!(rotation, 90 | 270) {
        (visible_h, visible_w)
    } else {
        (visible_w, visible_h)
    };
    let depth_of = |frame: &RawFrame| -> Result<u8> {
        match frame {
            RawFrame::Planar(p) => Ok(p.depth),
            RawFrame::Avc { picture, .. } => Ok(picture.bit_depth),
            RawFrame::Planar8(_) | RawFrame::Yuv { .. } => Ok(8),
            _ => Err(invalid("expected planar sample planes")),
        }
    };
    let depth = depth_of(&first)?;
    let prepare = |frame: &RawFrame,n:u64,t:Option<f64>| -> Result<_> {
        if depth_of(frame)? != depth {
            return Err(invalid("frame sample depth changed"));
        }
        let mut samples = if bake_rotation && crop != [0; 4] {
            let clipped = VideoGeometry {
                crop: Some([crop[0], crop[1], visible_w, visible_h]),
                ..Default::default()
            }
            .apply_media(frame, coded_w, coded_h)?;
            let colour = match frame {
                RawFrame::Avc { colour, .. } => *colour,
                RawFrame::Planar(p) => p.colour,
                RawFrame::Planar8(p) => p.colour,
                _ => Default::default(),
            };
            let clipped = RawFrame::Planar(std::sync::Arc::new(
                crate::playback_native::PackedPlanar::new(clipped, depth, colour)?,
            ));
            geometry.apply_display_media(&clipped, display_w, display_h, rotation)?
        } else if bake_rotation {
            geometry.apply_display_media(frame, w, h, rotation)?
        } else {
            geometry.apply(frame, coded_w, coded_h)?
        };
        if !processed {filters.apply_colour_at(&mut samples, depth, source_full_range, source_colour.matrix,n,t)?;}
        Ok(samples)
    };
    let mut colour = reader.colour();
    if let RawFrame::Planar8(ref p) = first {
        colour.full_range = p.colour.full;
    }
    let first = prepare(&first,0,reader.frame_interval().map(|(start,_,scale)|start as f64/scale as f64))?;
    let shape = (first.width, first.height, first.subsampling);
    let encoding = Encoding::Ffv1V1 {
        width: u32::try_from(first.width).map_err(|_| invalid("FFV1 width overflow"))?,
        height: u32::try_from(first.height).map_err(|_| invalid("FFV1 height overflow"))?,
    };
    let specs = if let Some(input) = webm.as_ref() {
        input
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| -> Result<_> {
                Ok(TrackSpec {
                    encoding: if index == video_index {
                        encoding
                    } else if track.codec == "A_OPUS" {
                        Encoding::Opus {
                            configuration: &track.codec_private,
                        }
                    } else {
                        Encoding::Aac {
                            configuration: &track.codec_private,
                            sample_rate: u32::try_from(track.sample_rate)
                                .map_err(|_| invalid("AAC rate overflow"))?,
                            channels: u16::try_from(track.channels)
                                .map_err(|_| invalid("AAC channels overflow"))?,
                        }
                    },
                    name: &track.name,
                    language: &track.language,
                })
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![TrackSpec {
            encoding,
            name: "",
            language: "",
        }]
    };
    let default_duration_ns = match webm
        .as_ref()
        .map(|r| r.tracks[video_index].default_duration_ns)
    {
        Some(duration) if duration != 0 => duration,
        _ => {
            let (start, end, scale) = reader
                .frame_interval()
                .ok_or_else(|| invalid("missing planar frame clock"))?;
            if scale == 0 {
                return Err(invalid("zero planar frame clock"));
            }
            u64::try_from(
                end.checked_sub(start)
                    .and_then(|n| n.checked_mul(1_000_000_000))
                    .ok_or_else(|| invalid("planar frame duration overflow"))?
                    / u128::from(scale),
            )
            .map_err(|_| invalid("planar frame duration overflow"))?
        }
    };
    let video_options = TrackOptions {
        rotation: if bake_rotation { 0 } else { rotation },
        default_duration_ns,
        video: Some(VideoMetadata {
            crop: if bake_rotation {
                [0; 4]
            } else {
                crop.map(u32::try_from)
                    .into_iter()
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|_| invalid("stored crop metadata overflow"))?
                    .try_into()
                    .unwrap()
            },
            pixel_aspect: if bake_rotation {
                crate::native_export::transformed_aspect(
                    reader.pixel_aspect(),
                    display_w,
                    display_h,
                    geometry,
                )?
            } else {
                let aspect = reader.pixel_aspect();
                if matches!(rotation, 90 | 270) {
                    (aspect.1, aspect.0)
                } else {
                    aspect
                }
            },
            colour: Some(colour),
            hdr: reader.hdr(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let options = if let Some(input) = webm.as_ref() {
        input
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| {
                if index == video_index {
                    video_options
                } else {
                    TrackOptions {
                        codec_delay_ns: track.codec_delay_ns,
                        default_duration_ns: track.default_duration_ns,
                        ..Default::default()
                    }
                }
            })
            .collect::<Vec<_>>()
    } else {
        vec![video_options]
    };
    let metadata = webm
        .as_ref()
        .map(|r| crate::container::matroska_write::FileMetadata {
            tags: r.tags.clone(),
            chapters: r.chapters.clone(),
        })
        .unwrap_or_default();
    let mut writer = PacketWriter::new_with_metadata(output, &specs, &options, &metadata)?;
    let mut stats = crate::media_info::LosslessStats {
        backend: "fvid",
        video_frames: 0,
        decoded_frames: 0,
        seek_used: false,
        video_packets: 0,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format: match first.subsampling {
            Some([2, 2]) => "yuv420p",
            Some([2, 1]) => "yuv422p",
            Some([1, 1]) => "yuv444p",
            Some([1, 2]) => "yuv440p",
            Some([4, 1]) => "yuv411p",
            Some([4, 4]) => "yuv410p",
            _ => return Err(invalid("unsupported planar FFV1 chroma layout")),
        }
        .into(),
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: geometry.vertical_flip,
        horizontal_flip: geometry.horizontal_flip,
    };
    if depth != 8 {
        stats.pixel_format = format!("{}{depth}le", stats.pixel_format);
    }
    if let Some(hook) = progress {
        hook.emit(writer.event());
    }
    let mut audio = Vec::new();
    if let Some(input) = webm.as_mut() {
        for packet_index in 0..input.packets.len() {
            let packet = input.packets[packet_index].clone();
            let index = input
                .tracks
                .iter()
                .position(|t| t.number == packet.track)
                .ok_or_else(|| invalid("unknown Matroska packet track"))?;
            if index == video_index {
                continue;
            }
            let track = input.tracks[index].clone();
            let inferred = if track.codec == "A_OPUS" {
                crate::container::opus_packet::duration_ns(&input.read_packet(packet_index)?)?
            } else {
                let config = crate::codec::config::AacConfig::parse(&track.codec_private)?;
                u64::from(config.frame_samples) * 1_000_000_000 / u64::from(config.sample_rate)
            };
            // Opus BlockDuration may describe the audible tail after padding;
            // the muxer needs the full coded packet span before applying padding.
            let duration = if track.codec == "A_OPUS" {
                inferred
            } else {
                packet.duration_ns.unwrap_or(inferred)
            };
            audio.push((
                packet.pts_ns as u64,
                packet_index,
                index,
                duration,
                packet.keyframe,
                packet.discard_padding_ns,
                packet.invisible,
            ));
        }
    }
    audio.sort_by_key(|p| (p.0, p.1));
    let mut copied = 0;
    let mut copy_audio = |until: u64,
                          writer: &mut PacketWriter<'_, W>,
                          stats: &mut crate::media_info::LosslessStats|
     -> Result<()> {
        while copied < audio.len() && audio[copied].0 <= until {
            check(cancel)?;
            let (pts, packet, index, duration, sync, padding, invisible) = audio[copied];
            let payload = webm
                .as_mut()
                .ok_or_else(|| invalid("missing audio source"))?
                .read_packet(packet)?;
            writer.write_packet_with_options(
                index,
                pts,
                duration,
                sync,
                &payload,
                crate::container::matroska_write::PacketOptions {
                    discard_padding_ns: padding,
                    invisible,
                },
            )?;
            stats.copied_packets += 1;
            copied += 1;
            if let Some(hook) = progress {
                hook.emit(writer.event());
            }
        }
        Ok(())
    };
    let mut next = Some(first);
    while let Some(mut samples) = next.take() {
        check(cancel)?;
        if (samples.width, samples.height, samples.subsampling) != shape {
            return Err(invalid("Y4M frame geometry changed"));
        }
        let (start, end, scale) = reader
            .frame_interval()
            .ok_or_else(|| invalid("missing Y4M timestamp"))?;
        let ns = |n: u128| -> Result<u64> {
            if scale == 0 {
                return Err(invalid("zero Y4M frame clock"));
            }
            u64::try_from(
                n.checked_mul(1_000_000_000)
                    .ok_or_else(|| invalid("Y4M clock overflow"))?
                    / u128::from(scale),
            )
            .map_err(|_| invalid("Y4M timestamp overflow"))
        };
        let (start, end) = (
            ns(start)?
                .checked_add(video_origin)
                .ok_or_else(|| invalid("video timestamp overflow"))?,
            ns(end)?
                .checked_add(video_origin)
                .ok_or_else(|| invalid("video timestamp overflow"))?,
        );
        if end <= start {
            return Err(invalid("Y4M frame duration below one nanosecond"));
        }
        if let Some(process)=processor.as_deref_mut() {
            process(&mut samples,depth,start)?;
            if (samples.width,samples.height,samples.subsampling)!=shape {return Err(invalid("processed frame geometry changed"));}
        }
        if processed {filters.apply_colour_at(&mut samples,depth,source_full_range,source_colour.matrix,stats.video_frames,Some(start as f64/1e9))?;}
        let packet = crate::codec::ffv1_encoder::encode(&samples, depth)?;
        check(cancel)?;
        copy_audio(start, &mut writer, &mut stats)?;
        writer.write_packet(video_index, start, end - start, true, &packet)?;
        stats.video_frames += 1;
        stats.decoded_frames += 1;
        stats.video_packets += 1;
        stats.fvid_crop_payload_copies += u64::from(geometry.crop.is_some());
        if let Some(hook) = progress {
            hook.emit(writer.event());
        }
        check(cancel)?;
        next = reader.read_frame_raw()?.as_ref().map(|frame|prepare(frame,stats.video_frames,reader.frame_interval().map(|(start,_,scale)|start as f64/scale as f64))).transpose()?;
    }
    check(cancel)?;
    copy_audio(u64::MAX, &mut writer, &mut stats)?;
    check(cancel)?;
    Ok((stats, writer.finish()?))
}
