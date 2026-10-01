use eframe::egui_wgpu::wgpu;
use fvid::color::tonemap::{DisplayTarget, ToneMap};
use fvid::color::transfer::Transfer;
use fvid::color::{Grade, Settings};
use fvid::color::{Interpolation, Log, Lut, Primaries};
use fvid::hardware_export::{
    EncodedTime, EncoderCodec, HardwareFrame, TrackOptions, VideoMetadata,
};
use fvid::playback_native::{NativeReader, RawFrame};
use fvid::player_gpu::{ColorShader, MetalEncoderRenderer};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

const HELP: &str = "fvid shader-export INPUT.mp4|INPUT.y4m OUTPUT.mkv (--video-only|--copy-audio) --shader FILE.wgsl [--codec h264|hevc] [--depth 8|10] [--size WIDTHxHEIGHT] [--bitrate BITS_PER_SECOND] [--device N] [--sdr-nits N] [--tonemap linear|gamma|clip|reinhard|hable|mobius]\nNative Metal/VideoToolbox video export. Source colour/HDR is graded before the shader: SDR Rec.709 by default, or HDR BT.2020 with --display pq|hlg. Shader output must preserve the selected signal. --copy-audio preserves all AAC tracks in eligible MP4 inputs with edit/gapless timing; unsupported tracks and complex edits are rejected. Subtitle copying is pending. Existing outputs are never overwritten.";

#[derive(Clone, Copy)]
struct PresentationWindow {
    begin: u128,
    end: u128,
    scale: u32,
}
impl PresentationWindow {
    fn map(self, interval: (u128, u128, u32)) -> fvid::Result<Option<(u128, u128, u32)>> {
        let (start, end, scale) = interval;
        if scale != self.scale || end <= start {
            return Err(fvid::Error::Invalid("invalid edited video clock".into()));
        }
        let start = start.max(self.begin);
        let end = end.min(self.end);
        Ok((start < end).then(|| (start - self.begin, end - self.begin, scale)))
    }
}

#[cfg(test)]
mod timeline_tests {
    use super::PresentationWindow;

    #[test]
    fn edits_clip_crossing_frames_and_remove_preroll_and_tail() {
        let window = PresentationWindow {
            begin: 100,
            end: 250,
            scale: 1000,
        };
        assert_eq!(window.map((0, 100, 1000)).unwrap(), None);
        assert_eq!(window.map((80, 120, 1000)).unwrap(), Some((0, 20, 1000)));
        assert_eq!(window.map((120, 160, 1000)).unwrap(), Some((20, 60, 1000)));
        assert_eq!(
            window.map((240, 280, 1000)).unwrap(),
            Some((140, 150, 1000))
        );
        assert_eq!(window.map((250, 300, 1000)).unwrap(), None);
        assert!(window.map((100, 100, 1000)).is_err());
        assert!(window.map((100, 120, 90000)).is_err());
    }
}

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.first().is_some_and(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        println!(
            "Geometry: --crop X:Y:WIDTH:HEIGHT crops upright pixels after container rotation and before --size scaling."
        );
        println!(
            "Output HDR metadata: --max-cll N --max-fall N (integer nits), --mastering-display Rx,Ry,Gx,Gy,Bx,By,Wx,Wy,max_nits,min_nits. These describe the result after grading/shaders."
        );
        println!(
            "Output: --display sdr|pq|hlg --hdr-nits N. HDR requires --codec hevc --depth 10; shaders must preserve the selected output signal."
        );
        println!(
            "Grading options: --lut FILE --log CURVE --gamut NAME --grid 2..128 --interp nearest|trilinear|tetrahedral. Source conversion runs before the LUT and shader."
        );
        return Ok(());
    }
    if args.len() < 2 {
        return Err(HELP.into());
    }
    let mut codec = EncoderCodec::H264;
    let mut depth = 8;
    let mut bitrate = 8_000_000;
    let mut ordinal = 0usize;
    let mut size = None;
    let mut crop = None;
    let mut shader_path = None;
    let mut video_only = false;
    let mut copy_audio = false;
    let mut target_nits = 100.0f32;
    let mut tone_map = None;
    let mut lut_path = None;
    let mut log = None;
    let mut gamut = None;
    let mut grid = 33usize;
    let mut interpolation = Interpolation::Tetrahedral;
    let mut output_transfer = Transfer::Bt709;
    let mut hdr_nits = 1000.0f32;
    let mut output_metadata = fvid::color::HdrMetadata::default();
    let mut i = 2;
    while i < args.len() {
        let option = &args[i];
        if option == "--video-only" {
            video_only = true;
            i += 1;
            continue;
        }
        if option == "--copy-audio" {
            copy_audio = true;
            i += 1;
            continue;
        }
        let value = args.get(i + 1).ok_or("missing export option value")?;
        match option.as_str() {
            "--shader" => shader_path = Some(value),
            "--crop" => {
                let v = value
                    .split(':')
                    .map(str::parse::<usize>)
                    .collect::<Result<Vec<_>, _>>()?;
                if v.len() != 4 {
                    return Err("crop must be X:Y:WIDTH:HEIGHT".into());
                }
                crop = Some([v[0], v[1], v[2], v[3]]);
            }
            "--lut" => lut_path = Some(value),
            "--log" => log = Some(Log::from_label(value).ok_or("unknown camera log curve")?),
            "--gamut" => gamut = Some(Primaries::from_label(value).ok_or("unknown source gamut")?),
            "--grid" => grid = value.parse()?,
            "--interp" => {
                interpolation =
                    Interpolation::from_label(value).ok_or("unknown LUT interpolation")?
            }
            "--codec" => {
                codec = match value.as_str() {
                    "h264" => EncoderCodec::H264,
                    "hevc" => EncoderCodec::Hevc,
                    _ => return Err("codec must be h264 or hevc".into()),
                }
            }
            "--depth" => depth = value.parse::<u8>()?,
            "--bitrate" => bitrate = value.parse::<u32>()?,
            "--device" => ordinal = value.parse()?,
            "--sdr-nits" => target_nits = value.parse()?,
            "--hdr-nits" => hdr_nits = value.parse()?,
            "--max-cll" => output_metadata.light.max_cll = f32::from(value.parse::<u16>()?),
            "--max-fall" => output_metadata.light.max_fall = f32::from(value.parse::<u16>()?),
            "--mastering-display" => {
                let v = value
                    .split(',')
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>()?;
                if v.len() != 10 {
                    return Err(
                        "mastering display needs Rx,Ry,Gx,Gy,Bx,By,Wx,Wy,max_nits,min_nits".into(),
                    );
                }
                output_metadata.mastering = Some(
                    fvid::color::MasteringDisplay::from_corners(
                        (v[0], v[1]),
                        (v[2], v[3]),
                        (v[4], v[5]),
                        (v[6], v[7]),
                        v[8] as f32,
                        v[9] as f32,
                    )
                    .ok_or("invalid output mastering display")?,
                );
            }
            "--display" => {
                output_transfer = match value.as_str() {
                    "sdr" => Transfer::Bt709,
                    "pq" => Transfer::Pq,
                    "hlg" => Transfer::Hlg,
                    _ => return Err("display must be sdr, pq or hlg".into()),
                }
            }
            "--tonemap" => {
                tone_map = Some(match value.as_str() {
                    "linear" => ToneMap::Linear,
                    "gamma" => ToneMap::Gamma,
                    "clip" => ToneMap::Clip,
                    "reinhard" => ToneMap::Reinhard,
                    "hable" => ToneMap::Hable,
                    "mobius" => ToneMap::Mobius,
                    _ => return Err("unknown tone map".into()),
                })
            }
            "--size" => {
                let (w, h) = value.split_once('x').ok_or("size must be WIDTHxHEIGHT")?;
                size = Some([w.parse::<usize>()?, h.parse::<usize>()?]);
            }
            _ => return Err(format!("unknown shader-export option: {option}").into()),
        }
        i += 2;
    }
    if video_only == copy_audio {
        return Err("select exactly one of --video-only or --copy-audio".into());
    }
    if !target_nits.is_finite() || target_nits <= 0.0 {
        return Err("SDR target nits must be finite and positive".into());
    }
    let output_hdr = matches!(output_transfer, Transfer::Pq | Transfer::Hlg);
    if !output_hdr && !output_metadata.is_empty() {
        return Err("output HDR metadata requires --display pq or hlg".into());
    }
    if output_metadata.light.max_cll > 0.0
        && output_metadata.light.max_fall > output_metadata.light.max_cll
    {
        return Err("MaxFALL cannot exceed MaxCLL".into());
    }
    fvid::codec::hevc_sei::output_hdr_nal(&output_metadata)?;
    if !hdr_nits.is_finite() || hdr_nits < 262.0 {
        return Err("HDR peak must be finite and at least 262 nits".into());
    }
    if output_hdr && (depth != 10 || !matches!(codec, EncoderCodec::Hevc)) {
        return Err("HDR output requires --codec hevc --depth 10".into());
    }
    if !(2..=128).contains(&grid) {
        return Err("grid must be 2..128".into());
    }
    if ![8, 10].contains(&depth) || (depth == 10 && matches!(codec, EncoderCodec::H264)) {
        return Err("depth must be 8, or 10 with hevc".into());
    }
    if bitrate == 0 || bitrate > i32::MAX as u32 {
        return Err("bitrate must be 1..2147483647".into());
    }
    if Path::new(&args[1]).extension().and_then(|v| v.to_str()) != Some("mkv") {
        return Err("shader-export output must use .mkv".into());
    }
    let source = std::fs::read_to_string(shader_path.ok_or("--shader is required")?)?;
    let shader = ColorShader::new(&source)?;
    let lut = lut_path
        .map(|path| -> Result<_, Box<dyn std::error::Error>> {
            Ok(Lut::from_text(&std::fs::read_to_string(path)?)?)
        })
        .transpose()?;
    let audio_source = if copy_audio {
        let input = fvid::container::mp4::Mp4Reader::open(
            BufReader::new(File::open(&args[0])?),
            Default::default(),
        )?;
        if !fvid::container::mp4_matroska::eligible(&input) {
            return Err("audio copy requires AVC/HEVC/AAC MP4 with contiguous media edits".into());
        }
        Some(input)
    } else {
        None
    };
    let video_window = audio_source
        .as_ref()
        .map(|input| -> Result<_, Box<dyn std::error::Error>> {
            let track = input
                .tracks()
                .iter()
                .find(|t| t.handler == *b"vide")
                .ok_or("input contains no video track")?;
            let (begin, end) = match track.edits.as_slice() {
                [] => (0, u128::from(track.duration)),
                [edit] if edit.media_time >= 0 && input.movie_timescale() != 0 => {
                    let begin = edit.media_time as u128;
                    let length = (u128::from(edit.duration) * u128::from(track.timescale))
                        .div_ceil(u128::from(input.movie_timescale()));
                    (
                        begin,
                        begin.checked_add(length).ok_or("video edit overflow")?,
                    )
                }
                _ => return Err("unsupported video edits".into()),
            };
            if end <= begin {
                return Err("empty video edit".into());
            }
            Ok(PresentationWindow {
                begin,
                end,
                scale: track.timescale,
            })
        })
        .transpose()?;
    let mut reader = NativeReader::new(BufReader::new(File::open(&args[0])?), 256 * 1024 * 1024)?;
    if !reader.enable_shared_surfaces()? && !matches!(reader, NativeReader::Y4m(_)) {
        return Err("input requires native VideoToolbox surfaces or uncompressed Y4M".into());
    }
    let rotation = reader.rotation();
    let mut first_frame = Some(
        reader
            .read_frame_raw()?
            .ok_or("input contains no video frame")?,
    );
    let [w, h] = match first_frame.as_ref().unwrap() {
        RawFrame::Surface { surface, .. } => [surface.width(), surface.height()],
        RawFrame::Yuv { width, height, .. } => [*width, *height],
        RawFrame::Planar8(frame) => [frame.width, frame.height],
        _ => return Err("decoder did not return a native surface".into()),
    };
    let upright = if matches!(rotation, 90 | 270) {
        [h, w]
    } else {
        [w, h]
    };
    let [x, y, cw, ch] = crop.unwrap_or([0, 0, upright[0], upright[1]]);
    if cw == 0
        || ch == 0
        || x.checked_add(cw).is_none_or(|v| v > upright[0])
        || y.checked_add(ch).is_none_or(|v| v > upright[1])
    {
        return Err("crop exceeds upright input dimensions".into());
    }
    let output_size = size.unwrap_or([cw, ch]);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::METAL))
        .into_iter()
        .filter(|a| a.get_info().device_type != wgpu::DeviceType::Cpu)
        .nth(ordinal)
        .ok_or("requested physical Metal device is unavailable")?;
    let features = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    if !adapter.features().contains(features) {
        return Err("Metal adapter lacks native P010 surface formats".into());
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        ..Default::default()
    }))?;
    let (kr, kb) = if output_hdr {
        (0.2627, 0.0593)
    } else {
        (0.2126, 0.0722)
    };
    let mut renderer = MetalEncoderRenderer::new(&device, &shader, depth, false, kr, kb)?;
    renderer.set_window([
        x as f32 / upright[0] as f32,
        y as f32 / upright[1] as f32,
        (x + cw) as f32 / upright[0] as f32,
        (y + ch) as f32 / upright[1] as f32,
    ])?;
    let mut grade_cache = None;
    let mut serial = 0u64;
    let frames = std::iter::from_fn(|| {
        loop {
            let raw = match first_frame
                .take()
                .map_or_else(|| reader.read_frame_raw(), |raw| Ok(Some(raw)))
            {
                Ok(Some(raw)) => raw,
                Ok(None) => return None,
                Err(e) => return Some(Err(e)),
            };
            let interval = reader
                .frame_interval()
                .ok_or_else(|| fvid::Error::Invalid("frame has no exact timing".into()));
            let interval = interval.and_then(|i| video_window.map_or(Ok(Some(i)), |w| w.map(i)));
            let interval = match interval {
                Ok(Some(i)) => i,
                Ok(None) => continue,
                Err(e) => return Some(Err(e)),
            };
            return Some((|| {
                let signal = reader.colour();
                let hdr = reader.hdr();
                if grade_cache
                    .as_ref()
                    .is_none_or(|(previous_signal, previous_hdr, _)| {
                        *previous_signal != signal || *previous_hdr != hdr
                    })
                {
                    let settings = Settings {
                        tone_map,
                        log,
                        gamut,
                        size: grid,
                        interpolation,
                        to: output_transfer,
                        dest: if output_hdr {
                            Primaries::BT2020
                        } else {
                            Primaries::BT709
                        },
                        ..Settings::video(if output_hdr {
                            DisplayTarget::hdr(hdr_nits)
                        } else {
                            DisplayTarget::sdr(target_nits)
                        })
                    };
                    let grade = Grade::new(signal, &hdr, settings, lut.clone());
                    if !grade.is_gpu_grade() {
                        return Err(fvid::Error::Invalid(
                            "source grade cannot execute on GPU".into(),
                        ));
                    }
                    grade_cache = Some((signal, hdr, Arc::new(grade)));
                }
                let (start, end, scale) = interval;
                let duration = end
                    .checked_sub(start)
                    .ok_or_else(|| fvid::Error::Invalid("invalid frame interval".into()))?;
                let convert = |v| {
                    i64::try_from(v)
                        .map_err(|_| fvid::Error::Invalid("frame timing overflow".into()))
                };
                let grade = grade_cache.as_ref().map(|(_, _, grade)| grade);
                let surface = match raw {
                    RawFrame::Planar8(frame) => renderer.render_planar8_sized(
                        &device,
                        &queue,
                        &frame,
                        serial,
                        grade,
                        output_size,
                    )?,
                    RawFrame::Surface { surface, colour } => renderer.render_surface_sized(
                        &device,
                        &queue,
                        &surface,
                        colour,
                        serial,
                        grade,
                        rotation,
                        output_size,
                    )?,
                    RawFrame::Yuv {
                        data,
                        width,
                        height,
                        sx,
                        sy,
                        ..
                    } => {
                        let packed = fvid::playback_native::PackedPlanar::new(
                            fvid::native_geometry::GeometryFrame {
                                width,
                                height,
                                subsampling: Some([sx, sy]),
                                data,
                            },
                            8,
                            fvid::playback_native::AvcColour::default(),
                        )?;
                        renderer.render_packed_sized(
                            &device,
                            &queue,
                            &packed,
                            serial,
                            grade,
                            output_size,
                        )?
                    }
                    _ => {
                        return Err(fvid::Error::Invalid(
                            "unexpected shader input storage".into(),
                        ));
                    }
                };
                serial = serial
                    .checked_add(1)
                    .ok_or_else(|| fvid::Error::Invalid("frame serial overflow".into()))?;
                Ok(HardwareFrame {
                    surface,
                    pts: EncodedTime {
                        value: convert(start)?,
                        timescale: scale,
                    },
                    duration: EncodedTime {
                        value: convert(duration)?,
                        timescale: scale,
                    },
                    force_keyframe: false,
                })
            })());
        }
    });
    let options = TrackOptions {
        video: Some(VideoMetadata {
            hdr: output_metadata,
            colour: Some(fvid::color::hdr::ColourDescription {
                primaries: if output_hdr { 9 } else { 1 },
                transfer: output_transfer.code(),
                matrix: if output_hdr { 9 } else { 1 },
                full_range: false,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let stats = if let Some(audio_source) = audio_source {
        fvid::hardware_export::write_video_file_with_mp4_audio(
            Path::new(&args[1]),
            frames,
            codec,
            bitrate,
            options,
            audio_source,
        )?
    } else {
        fvid::hardware_export::write_video_file(
            Path::new(&args[1]),
            frames,
            codec,
            bitrate,
            options,
        )?
    };
    eprintln!(
        "frames={} compressed_bytes={} backend=metal encoder=videotoolbox",
        stats.frames, stats.compressed_bytes
    );
    Ok(())
}
