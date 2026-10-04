//! Production CUDA route using owned demux/scheduling/filter/encode/container code.
use crate::{
    HwFilterOptions, HwFilterStats,
    owned_nvdec_movie::{AvcMovieReader, AvcMovieRenderer},
    owned_nvdec_mp4::AvcMp4Input,
    owned_nvenc_movie::AvcMovieEncoder,
};
use std::{
    io::{BufReader, Read, Seek},
    path::Path,
    time::Duration,
};
struct Plan {
    transform: fvid_cuda::Nv12Transform,
    fps: (u32, u32),
    full_range: bool,
}
fn plan<R: Read + Seek>(
    source: &mut AvcMp4Input<R>,
    options: &HwFilterOptions,
) -> Result<Option<Plan>, String> {
    if options.shader_sampling && options.shader.is_none() {
        return Err("sampling shader mode requires shader source".into());
    }
    // Keep remaining legacy option semantics until they have owned acceptance.
    if options.host_bounce || options.interval.is_some() {
        return Ok(None);
    }
    let metadata = source.video_metadata();
    if metadata.options.video.is_some_and(|v| v.crop != [0; 4]) {
        return Ok(None);
    }
    if source.qualify_packets().is_err() {
        return Ok(None);
    }
    let (width, height) = source.coded_dimensions();
    let crop = options.crop.unwrap_or(fvid_media_info::CropRect {
        x: 0,
        y: 0,
        width: width as usize,
        height: height as usize,
    });
    let convert = |v| u32::try_from(v).map_err(|_| "CUDA crop exceeds u32");
    let transform = fvid_cuda::Nv12Transform {
        crop_x: convert(crop.x)?,
        crop_y: convert(crop.y)?,
        out_width: convert(crop.width)?,
        out_height: convert(crop.height)?,
        hflip: options.horizontal_flip,
        vflip: options.vertical_flip,
    };
    if transform.out_width == 0
        || transform.out_height == 0
        || (transform.crop_x | transform.crop_y | transform.out_width | transform.out_height) & 1
            != 0
        || transform
            .crop_x
            .checked_add(transform.out_width)
            .is_none_or(|end| end > width)
        || transform
            .crop_y
            .checked_add(transform.out_height)
            .is_none_or(|end| end > height)
    {
        return Err("CUDA crop must be even and within the source image".into());
    }
    // Derive the nominal rate from the original media duration; timeline blanks
    // and repeats still retain their exact individual presentation intervals.
    let duration = (0..source.packet_count())
        .try_fold(0u64, |sum, index| {
            sum.checked_add(u64::from(source.track().samples.get(index)?.duration))
        })
        .ok_or("CUDA nominal duration overflow")?;
    let numerator = u64::try_from(source.packet_count())
        .map_err(|_| "CUDA sample count overflow")?
        .checked_mul(u64::from(source.track().timescale))
        .ok_or("CUDA nominal rate overflow")?;
    if duration == 0 || numerator == 0 {
        return Err("CUDA input has no nominal rate".into());
    }
    let (mut a, mut b) = (numerator, duration);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let fps = (
        u32::try_from(numerator / a).map_err(|_| "CUDA rate numerator overflow")?,
        u32::try_from(duration / a).map_err(|_| "CUDA rate denominator overflow")?,
    );
    Ok(Some(Plan {
        transform,
        fps,
        full_range: metadata
            .options
            .video
            .and_then(|v| v.colour)
            .is_some_and(|c| c.full_range),
    }))
}
/// No legacy backend is available through this entrypoint.
pub fn hw_filter(
    source: &Path,
    destination: &Path,
    options: &HwFilterOptions,
) -> Result<HwFilterStats, String> {
    try_filter(source, destination, options)?
        .ok_or_else(|| "native CUDA route does not yet cover this input/options".into())
}
/// Returning None admits the existing legacy route before opening any GPU
/// resource. Once admitted, every execution error is final: no libav retry.
pub fn try_filter(
    source: &Path,
    destination: &Path,
    options: &HwFilterOptions,
) -> Result<Option<HwFilterStats>, String> {
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if destination
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
    {
        return Ok(None);
    }
    let input = BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
    let mut source = match AvcMp4Input::open(input, Default::default()) {
        Ok(source) => source,
        Err(_) => return Ok(None),
    };
    let Some(plan) = plan(&mut source, options)? else {
        return Ok(None);
    };
    let shader = options
        .shader
        .as_ref()
        .map(|source| {
            if options.shader_sampling {
                fvid_cuda::ByteShader::with_sampling(source.as_ref())
            } else {
                fvid_cuda::ByteShader::new(source.as_ref())
            }
        })
        .transpose()?;
    let reader = AvcMovieReader::new(source, options.device, 32, 2, 1_000_000, 16)?;
    let renderer = AvcMovieRenderer::new_with_shader(
        reader,
        plan.transform,
        plan.full_range,
        shader.as_ref(),
    )?;
    let device = renderer.device_name().to_owned();
    let mut encoder = AvcMovieEncoder::new(
        renderer,
        fvid_cuda::NvencCodec::H264,
        plan.fps.0,
        plan.fps.1,
        32,
        Duration::from_secs(30),
    )?;
    let stats = encoder.export_avc_matroska(destination, 32 << 20, None, None)?;
    Ok(Some(HwFilterStats {
        filter: if shader.is_some() {
            "cuda-shader"
        } else {
            "cuda-crop-flip"
        },
        backend: "owned-cuda-nvdec-nvenc",
        device,
        video_frames: stats.video_frames,
        width: stats.width,
        height: stats.height,
        host_frame_copies: 0,
        device_filter_passes: encoder.device_filter_passes(),
        encoder: "native-h264-nvenc",
        host_bounce: false,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA NVDEC and NVENC"]
    fn production_hw_filter_routes_synthetic_avc_without_libav() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/control.mp4");
        let destination = std::env::temp_dir().join(format!(
            "fvid-owned-hw-filter-{}-{}.mkv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let stats = hw_filter(
            &source,
            &destination,
            &HwFilterOptions {
                horizontal_flip: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "owned-cuda-nvdec-nvenc");
        assert_eq!(stats.host_frame_copies, 0);
        assert_eq!(stats.video_frames, 12);
        let mut input = crate::owned_webm::WebmReader::open(
            BufReader::new(std::fs::File::open(&destination).unwrap()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(input.packets.len(), 12);
        let mut decoder = fvid_codecs::codec::avc_decoder::AvcDecoder::new(
            &input.tracks[0].codec_private,
            16 << 20,
        )
        .unwrap();
        for index in 0..input.packets.len() {
            assert!(
                decoder
                    .decode_order(&input.read_packet(index).unwrap())
                    .unwrap()
                    .is_some()
            );
        }
        std::fs::remove_file(destination).unwrap();
    }
    #[test]
    fn synthetic_production_route_qualifies_packets_without_driver() {
        let mut source = AvcMp4Input::open(
            std::io::Cursor::new(
                include_bytes!("../../../tests/fixtures/playback-errors/control.mp4").as_slice(),
            ),
            Default::default(),
        )
        .unwrap();
        let plan = plan(&mut source, &HwFilterOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(plan.fps, (12, 1));
        for crop in [
            fvid_media_info::CropRect {
                x: 1,
                y: 0,
                width: 2,
                height: 2,
            },
            fvid_media_info::CropRect {
                x: usize::MAX,
                y: 0,
                width: 2,
                height: 2,
            },
        ] {
            assert!(
                super::plan(
                    &mut source,
                    &HwFilterOptions {
                        crop: Some(crop),
                        ..Default::default()
                    }
                )
                .is_err()
            );
        }
        assert!(source.read_next(&mut Vec::new()).unwrap().unwrap().sync);
        assert!(
            super::plan(
                &mut source,
                &HwFilterOptions {
                    host_bounce: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .is_none()
        );
        assert!(
            super::plan(
                &mut source,
                &HwFilterOptions {
                    shader_sampling: true,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}
