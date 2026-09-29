//! Owned MP4 AVC/HEVC to FFV1/Matroska, retaining companion AAC packets.
use crate::{
    Result,
    container::{
        matroska_write::{Encoding, FileMetadata, PacketWriter, TrackSpec},
        mp4::Mp4Reader,
        mp4_matroska,
    },
    invalid,
    playback_native::{NativeReader, RawFrame},
};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    fs::File,
    io::{BufReader, Read, Seek, Write},
    path::Path,
};
fn check(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(|c| c.is_cancelled()) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}
/// All tracks must be represented; no unsupported stream may disappear.
pub fn eligible(source: &Path) -> Result<bool> {
    let mut input = File::open(source)?;
    let mut prefix = [0; 8];
    if input.read(&mut prefix)? != 8 || &prefix[4..] != b"ftyp" {
        return Ok(false);
    }
    let input = Mp4Reader::open(BufReader::new(File::open(source)?), Default::default())?;
    Ok(mp4_matroska::eligible(&input)
        && input
            .tracks()
            .iter()
            .filter(|t| t.handler == *b"vide")
            .count()
            == 1)
}
/// Caller owns atomic publication. Progress never reports done before publication.
pub fn write_mp4<W: Write + Seek>(
    source: &Path,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<(crate::media_info::LosslessStats, ProgressEvent)> {
    check(cancel)?;
    let mut input = Mp4Reader::open(BufReader::new(File::open(source)?), Default::default())?;
    if !mp4_matroska::eligible(&input)
        || input
            .tracks()
            .iter()
            .filter(|t| t.handler == *b"vide")
            .count()
            != 1
    {
        return Err(invalid(
            "owned FFV1 export requires one AVC/HEVC video and supported AAC companion tracks",
        ));
    }
    let tracks = input.tracks().to_vec();
    let video = tracks.iter().position(|t| t.handler == *b"vide").unwrap();
    let plans = tracks
        .iter()
        .map(|t| mp4_matroska::plan(t, input.movie_timescale(), cancel))
        .collect::<Result<Vec<_>>>()?;
    let specs = tracks
        .iter()
        .enumerate()
        .map(|(index, t)| -> Result<_> {
            Ok(TrackSpec {
                encoding: if index == video {
                    Encoding::Ffv1V1 {
                        width: t.width.into(),
                        height: t.height.into(),
                    }
                } else {
                    Encoding::Aac {
                        configuration: crate::codec::config::aac_specific_config(&t.configuration)?,
                        sample_rate: t.sample_rate,
                        channels: t.channels,
                    }
                },
                name: &t.name,
                language: &t.language,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let options = plans.iter().map(|p| p.options.clone()).collect::<Vec<_>>();
    let mut reader = NativeReader::software(BufReader::new(File::open(source)?), usize::MAX)?;
    let mut writer =
        PacketWriter::new_with_metadata(output, &specs, &options, &FileMetadata::from_mp4(&input))?;
    if let Some(hook) = progress {
        hook.emit(writer.event());
    }
    check(cancel)?;
    let mut audio = BinaryHeap::new();
    for (i, plan) in plans.iter().enumerate() {
        if i != video {
            if let Some(first) = plan.packets.first() {
                audio.push(Reverse((first.dts, i, 0usize)));
            }
        }
    }
    let mut payload = Vec::new();
    let mut stats = crate::media_info::LosslessStats {
        backend: "fvid",
        video_frames: 0,
        decoded_frames: 0,
        seek_used: false,
        video_packets: 0,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format: String::new(),
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: false,
        horizontal_flip: false,
    };
    let mut depth = None;
    loop {
        check(cancel)?;
        let frame = reader.read_frame_raw()?;
        let time = if frame.is_some() {
            let (start, end, scale) = reader
                .frame_interval()
                .ok_or_else(|| invalid("missing FFV1 frame timing"))?;
            let ns = |n: u128| -> Result<u64> {
                if scale == 0 {
                    return Err(invalid("zero FFV1 clock"));
                }
                u64::try_from(
                    n.checked_mul(1_000_000_000)
                        .ok_or_else(|| invalid("FFV1 clock overflow"))?
                        / u128::from(scale),
                )
                .map_err(|_| invalid("FFV1 timestamp overflow"))
            };
            let start = ns(start)?;
            let end = ns(end)?;
            if end <= start {
                return Err(invalid("FFV1 frame duration is below one nanosecond"));
            }
            Some((start, end - start))
        } else {
            None
        };
        while audio
            .peek()
            .is_some_and(|Reverse((dts, _, _))| time.is_none_or(|(pts, _)| *dts <= i128::from(pts)))
        {
            check(cancel)?;
            let Reverse((_, track, index)) = audio.pop().unwrap();
            input.read_packet(track, index, &mut payload)?;
            let packet = &plans[track].packets[index];
            writer.write_packet_with_options(
                track,
                packet.pts,
                packet.duration,
                true,
                &payload,
                packet.options,
            )?;
            stats.copied_packets += 1;
            if let Some(next) = plans[track].packets.get(index + 1) {
                audio.push(Reverse((next.dts, track, index + 1)));
            }
            if let Some(hook) = progress {
                hook.emit(writer.event());
            }
        }
        let Some(frame) = frame else {
            break;
        };
        let (w, h, bit_depth) = match &frame {
            RawFrame::Avc { picture, .. } => {
                let (w, h) = picture.dimensions();
                (w, h, picture.bit_depth)
            }
            RawFrame::Planar8(planes) => (planes.width, planes.height, 8),
            _ => return Err(invalid("unexpected FFV1 input picture type")),
        };
        if w != usize::from(tracks[video].width)
            || h != usize::from(tracks[video].height)
            || depth.is_some_and(|d| d != bit_depth)
        {
            return Err(invalid("FFV1 stream geometry or depth changed"));
        }
        depth = Some(bit_depth);
        let samples = crate::native_geometry::VideoGeometry::default().apply(&frame, w, h)?;
        let packet = crate::codec::ffv1_encoder::encode(&samples, bit_depth)?;
        check(cancel)?;
        let (pts, duration) = time.unwrap();
        writer.write_packet(video, pts, duration, true, &packet)?;
        stats.video_frames += 1;
        stats.decoded_frames += 1;
        stats.video_packets += 1;
        let format = match samples.subsampling {
            Some([2, 2]) => "yuv420p",
            Some([2, 1]) => "yuv422p",
            Some([1, 1]) => "yuv444p",
            _ => return Err(invalid("unexpected FFV1 input subsampling")),
        };
        stats.pixel_format = if bit_depth == 8 {
            format.into()
        } else {
            format!("{format}{bit_depth}le")
        };
        if let Some(hook) = progress {
            hook.emit(writer.event());
        }
    }
    if stats.video_frames == 0 {
        return Err(invalid("input has no decoded video frames"));
    }
    check(cancel)?;
    let event = writer.finish()?;
    Ok((stats, event))
}

/// Exhaustive admission for the currently migrated identity transform.
pub fn identity(transform: &crate::media_info::LosslessTransform) -> bool {
    matches!(
        transform,
        crate::media_info::LosslessTransform {
            crop: None,
            vertical_flip: false,
            horizontal_flip: false,
            scale: None,
            epx: None,
            transpose: None,
            rotate: None,
            pad: None,
            burn_subs: None,
            overlay: None,
            xfade: None,
            yadif: None,
            bwdif: None,
            w3fdif: None,
            tblend: None,
            tmix: None,
            hqdn3d: None,
            gblur: None,
            eq: None,
            unsharp: None,
            hue: None,
            avgblur: None,
            boxblur: None,
            negate: None,
            edgedetect: None,
            sobel: None,
            prewitt: None,
            roberts: None,
            kirsch: None,
            scharr: None,
            atadenoise: None,
            owdenoise: None,
            vaguedenoiser: None,
            nlmeans: None,
            bm3d: None,
            dctdnoiz: None,
            fftdnoiz: None,
            smartblur: None,
            sab: None,
            bilateral: None,
            cas: None,
            vignette: None,
            curves: None,
            colorbalance: None,
            colorlevels: None,
            colorchannelmixer: None,
            deflicker: None,
            photosensitivity: None,
            monochrome: None,
            grayworld: None,
            drawbox: None,
            drawgrid: None,
            lagfun: None,
            amplify: None,
            bitplanenoise: None,
            deband: None,
            gradfun: None,
            lenscorrection: None,
            pixelize: None,
            removegrain: None,
            yaepblur: None,
            vibrance: None,
            dilation: None,
            erosion: None,
            colorize: None,
            exposure: None,
            chromashift: None,
            colorcontrast: None,
            colorcorrect: None,
            histeq: None,
            shuffleplanes: None,
            lutyuv: None,
            colorhold: None,
            fade: None,
            perspective: None,
            lumakey: None,
            chromakey: None,
            colorkey: None,
            despill: None,
            selectivecolor: None,
            stereo3d: None,
            field: None,
            hqx: None,
            xbr: None,
            il: None,
            super2xsai: None,
            kerndeint: None,
            phase: None,
            estdif: None,
            tinterlace: None,
            separatefields: None,
            weave: None,
            doubleweave: None,
            framepack: None,
            telecine: None,
            pullup: None,
            decimate: None,
            mpdecimate: None,
            framestep: None,
            tile: None,
            untile: None,
            shuffleframes: None,
            reverse: None,
            r#loop: None,
            thumbnail: None,
            freezedetect: None,
            pseudocolor: None,
            minterpolate: None,
            fps: None,
            colorspace: None,
            zscale: None,
            tonemap: None,
            pix_fmt: None,
            interval: None,
            seek: false,
        }
    )
}
