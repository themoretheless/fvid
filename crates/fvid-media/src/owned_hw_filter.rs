//! Production CUDA route using owned demux/scheduling/filter/encode/container code.
use crate::{
    HwFilterOptions, HwFilterStats,
    owned_nvdec_movie::{MovieReader, MovieRenderer},
    owned_nvdec_source::MovieSource,
    owned_nvenc_movie::MovieEncoder,
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
    interval: Option<(i64, i64)>,
}
fn plan<R: Read + Seek>(
    source: &mut MovieSource<R>,
    options: &HwFilterOptions,
) -> Result<Option<Plan>, String> {
    if options.shader_sampling && options.shader.is_none() {
        return Err("sampling shader mode requires shader source".into());
    }
    // Keep component depth intact through owned filter and encoder surfaces.
    if !matches!(source.bit_depth(), 8 | 10) {
        return Ok(None);
    }
    let interval = options
        .interval
        .map(|(from, to)| -> Result<(i64, i64), String> {
            if from < 0 || from >= to {
                return Err("hw-filter interval requires 0 <= from < to".into());
            }
            let ticks = |us: i64| -> Result<i64, String> {
                let numerator = i128::from(us) * i128::from(source.track().timescale);
                if numerator % 1_000_000 != 0 {
                    return Err("interval boundary is not exact in video time base".into());
                }
                i64::try_from(numerator / 1_000_000)
                    .map_err(|_| "interval timestamp overflow".into())
            };
            Ok((ticks(from)?, ticks(to)?))
        })
        .transpose()?;
    let metadata = source.video_metadata();
    if source.qualify_packets().is_err() {
        return Ok(None);
    }
    if interval.is_some() {
        crate::owned_nvdec_movie::select_presentations(
            source.movie_presentations(1_000_000)?,
            interval,
            crate::owned_nvdec_movie::IntervalSelection::FrameStarts,
        )?;
    }
    let (width, height) = source.coded_dimensions();
    let [left, right, top, bottom] = metadata.options.video.map_or([0; 4], |video| video.crop);
    let visible_width = width
        .checked_sub(left)
        .and_then(|width| width.checked_sub(right))
        .ok_or("invalid video horizontal display crop")?;
    let visible_height = height
        .checked_sub(top)
        .and_then(|height| height.checked_sub(bottom))
        .ok_or("invalid video vertical display crop")?;
    let crop = options.crop.unwrap_or(fvid_media_info::CropRect {
        x: 0,
        y: 0,
        width: visible_width as usize,
        height: visible_height as usize,
    });
    if crop
        .x
        .checked_add(crop.width)
        .is_none_or(|end| end > visible_width as usize)
        || crop
            .y
            .checked_add(crop.height)
            .is_none_or(|end| end > visible_height as usize)
    {
        return Err("CUDA crop exceeds the visible source image".into());
    }
    let convert = |v| u32::try_from(v).map_err(|_| "CUDA crop exceeds u32");
    let transform = fvid_cuda::Nv12Transform {
        crop_x: convert(crop.x)?
            .checked_add(left)
            .ok_or("CUDA crop x overflow")?,
        crop_y: convert(crop.y)?
            .checked_add(top)
            .ok_or("CUDA crop y overflow")?,
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
        interval,
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
    let mut source = match MovieSource::open(input, Default::default()) {
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
    let reader = MovieReader::new_with_selection(
        source,
        options.device,
        32,
        2,
        1_000_000,
        16,
        plan.interval,
        crate::owned_nvdec_movie::IntervalSelection::FrameStarts,
    )?;
    let renderer = MovieRenderer::new_with_shader_and_host_bounce(
        reader,
        plan.transform,
        plan.full_range,
        shader.as_ref(),
        options.host_bounce,
    )?;
    let device = renderer.device_name().to_owned();
    let codec = if renderer.buffer().bit_depth() == 10 {
        fvid_cuda::NvencCodec::Hevc
    } else {
        fvid_cuda::NvencCodec::H264
    };
    let mut encoder = MovieEncoder::new(
        renderer,
        codec,
        plan.fps.0,
        plan.fps.1,
        32,
        Duration::from_secs(30),
    )?;
    let stats = encoder.export_matroska(destination, 32 << 20, None, None)?;
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
        host_frame_copies: encoder.host_frame_copies(),
        device_filter_passes: encoder.device_filter_passes(),
        encoder: if codec == fvid_cuda::NvencCodec::Hevc {
            "native-hevc-nvenc"
        } else {
            "native-h264-nvenc"
        },
        host_bounce: options.host_bounce,
    }))
}
#[cfg(test)]
mod tests {
    #[test]
    fn synthetic_hevc_main_and_repeated_edits_enter_native_production_route() {
        for bytes in [
            include_bytes!("../../../tests/fixtures/hevc/main-ipb.mp4").as_slice(),
            include_bytes!("../../../tests/fixtures/hevc/main10-ipb.mp4").as_slice(),
            include_bytes!(
                "../../../tests/fixtures/playback-errors/hevc-main10-cuda-edit-repeat.mp4"
            )
            .as_slice(),
            include_bytes!("../../../tests/fixtures/playback-errors/hevc-cuda-edit-repeat.mp4")
                .as_slice(),
        ] {
            let mut source =
                MovieSource::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
            assert!(matches!(source, MovieSource::Hevc(_)));
            let plan = plan(&mut source, &HwFilterOptions::default())
                .unwrap()
                .expect("HEVC Main must enter owned route");
            assert_eq!(plan.fps, (30, 1));
            assert_eq!(
                (plan.transform.out_width, plan.transform.out_height),
                source.coded_dimensions()
            );
            assert!(source.read_next(&mut Vec::new()).unwrap().unwrap().sync);
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA HEVC Main10 NVDEC/NVENC and CUDA"]
    fn production_host_bounce_preserves_main10_shader_and_movie_clock() {
        for interval in [None, Some((200_000, 500_000))] {
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/playback-errors/cuda-hevc-main10-edit-repeat.mp4");
            struct Output(std::path::PathBuf);
            impl Drop for Output {
                fn drop(&mut self) {
                    let _ = std::fs::remove_file(&self.0);
                }
            }
            let output = Output(std::env::temp_dir().join(format!(
                "fvid-main10-bounce-{}-{}.mkv",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
            let mut input = MovieSource::open(
                BufReader::new(std::fs::File::open(&source).unwrap()),
                Default::default(),
            )
            .unwrap();
            let scale = input.track().timescale;
            let ticks = interval.map(|(from, to)| {
                (
                    from * i64::from(scale) / 1_000_000,
                    to * i64::from(scale) / 1_000_000,
                )
            });
            let events = crate::owned_nvdec_movie::select_presentations(
                input.movie_presentations(1000).unwrap(),
                ticks,
                crate::owned_nvdec_movie::IntervalSelection::FrameStarts,
            )
            .unwrap();
            let options = HwFilterOptions {
                host_bounce: true,
                interval,
                crop: Some(fvid_media_info::CropRect {
                    x: 2,
                    y: 2,
                    width: 252,
                    height: 188,
                }),
                shader: Some(std::sync::Arc::from(
                    "__device__ unsigned int process_byte(unsigned int value, unsigned int plane, unsigned int x, unsigned int y) { return plane == 0u ? 940u : 512u; }",
                )),
                ..Default::default()
            };
            let stats = hw_filter(&source, &output.0, &options).unwrap();
            assert_eq!(stats.backend, "owned-cuda-nvdec-nvenc");
            assert!(stats.host_bounce);
            assert_eq!(stats.host_frame_copies, events.len() as u64 * 2);
            assert_eq!(
                stats.device_filter_passes,
                (events.len() + events.iter().filter(|e| e.sample.is_some()).count()) as u64
            );
            assert_eq!((stats.width, stats.height), (252, 188));
            let mut saved = crate::owned_webm::WebmReader::open(
                BufReader::new(std::fs::File::open(&output.0).unwrap()),
                Default::default(),
            )
            .unwrap();
            saved.scan_all().unwrap();
            let mut times: Vec<_> = saved
                .packets
                .iter()
                .map(|p| (p.pts_ns as u64, p.duration_ns.unwrap()))
                .collect();
            let mut expected: Vec<_> = events
                .iter()
                .map(|e| {
                    (
                        (e.start as u64 * 1_000_000_000) / u64::from(scale),
                        ((e.end - e.start) as u64 * 1_000_000_000) / u64::from(scale),
                    )
                })
                .collect();
            times.sort_unstable();
            expected.sort_unstable();
            assert_eq!(times, expected);
            let mut decoder = fvid_codecs::codec::hevc_decoder::HevcDecoder::from_configuration(
                &saved.tracks[0].codec_private,
                64 << 20,
            )
            .unwrap();
            for index in 0..saved.packets.len() {
                let frame = decoder
                    .decode_packet(&saved.read_packet(index).unwrap())
                    .unwrap()
                    .unwrap();
                assert_eq!(frame.picture.depth, [10, 10]);
                assert!(
                    frame.picture.planes[0]
                        .samples()
                        .iter()
                        .all(|v| v.abs_diff(940) <= 2)
                );
                assert!(
                    frame.picture.planes[1..]
                        .iter()
                        .flat_map(|p| p.samples())
                        .all(|v| v.abs_diff(512) <= 2)
                );
            }
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA HEVC Main10 NVDEC and NVENC"]
    fn production_hw_filter_routes_main10_movie_without_libav() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/cuda-hevc-main10-edit-repeat.mp4");
        struct Output(std::path::PathBuf);
        impl Drop for Output {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let output = Output(std::env::temp_dir().join(format!(
                "fvid-main10-{}-{}.mkv",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        let stats = hw_filter(&source, &output.0, &HwFilterOptions::default()).unwrap();
        assert_eq!(stats.backend, "owned-cuda-nvdec-nvenc");
        assert_eq!(stats.encoder, "native-hevc-nvenc");
        assert_eq!(stats.host_frame_copies, 0);
        assert_eq!(stats.video_frames, 14);
        let mut saved = crate::owned_webm::WebmReader::open(
            BufReader::new(std::fs::File::open(&output.0).unwrap()),
            Default::default(),
        )
        .unwrap();
        saved.scan_all().unwrap();
        let config =
            fvid_codecs::codec::config::HevcConfig::parse(&saved.tracks[0].codec_private).unwrap();
        assert_eq!((config.bit_depth_luma, config.bit_depth_chroma), (10, 10));
        let mut decoder = fvid_codecs::codec::hevc_decoder::HevcDecoder::from_configuration(
            &saved.tracks[0].codec_private,
            64 << 20,
        )
        .unwrap();
        for index in 0..saved.packets.len() {
            let frame = decoder
                .decode_packet(&saved.read_packet(index).unwrap())
                .unwrap()
                .unwrap();
            assert_eq!(frame.picture.depth, [10, 10]);
        }
        assert_eq!(saved.packets.len(), 14);
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA HEVC NVDEC and H264 NVENC"]
    fn production_hw_filter_routes_hevc_movie_without_libav() {
        for relative in [
            "../../tests/fixtures/playback-errors/cuda-hevc.mp4",
            "../../tests/fixtures/playback-errors/cuda-hevc-edit-repeat.mp4",
        ] {
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
            let mut own = MovieSource::open(
                BufReader::new(std::fs::File::open(&source).unwrap()),
                Default::default(),
            )
            .unwrap();
            let events = own.movie_presentations(1000).unwrap();
            let timescale = own.track().timescale;
            let destination = std::env::temp_dir().join(format!(
                "fvid-owned-hevc-{}-{}.mkv",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let stats = hw_filter(&source, &destination, &HwFilterOptions::default()).unwrap();
            assert_eq!(stats.backend, "owned-cuda-nvdec-nvenc");
            assert_eq!(stats.host_frame_copies, 0);
            assert_eq!(stats.video_frames as usize, events.len());
            let mut input = crate::owned_webm::WebmReader::open(
                BufReader::new(std::fs::File::open(&destination).unwrap()),
                Default::default(),
            )
            .unwrap();
            input.scan_all().unwrap();
            assert_eq!(input.packets.len(), events.len());
            let mut decoder = fvid_codecs::codec::avc_decoder::AvcDecoder::new(
                &input.tracks[0].codec_private,
                16 << 20,
            )
            .unwrap();
            for (index, event) in events.iter().enumerate() {
                let packet = &input.packets[index];
                let expected =
                    i64::try_from(i128::from(event.start) * 1_000_000_000 / i128::from(timescale))
                        .unwrap();
                assert!(
                    (packet.pts_ns - expected).abs() <= 1_000_000,
                    "Matroska clock quantization must not change movie occurrence"
                );
                assert!(
                    decoder
                        .decode_order(&input.read_packet(index).unwrap())
                        .unwrap()
                        .is_some()
                );
            }
            std::fs::remove_file(destination).unwrap();
        }
    }
    use super::*;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA NVDEC and NVENC"]
    fn production_hw_filter_routes_synthetic_avc_without_libav() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/cuda-h264.mp4");
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
                interval: Some((250_000, 750_000)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "owned-cuda-nvdec-nvenc");
        assert_eq!(stats.host_frame_copies, 0);
        assert_eq!(stats.video_frames, 6);
        let mut input = crate::owned_webm::WebmReader::open(
            BufReader::new(std::fs::File::open(&destination).unwrap()),
            Default::default(),
        )
        .unwrap();
        input.scan_all().unwrap();
        assert_eq!(input.packets.len(), 6);
        assert_eq!(input.packets[0].pts_ns, 0);
        assert!(input.packets.iter().all(|packet| packet.pts_ns >= 0
            && packet.pts_ns < 500_000_000
            && packet.duration_ns == Some(83_333_333)));
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
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA NVDEC and NVENC"]
    fn production_hw_filter_routes_sps_crop_without_libav() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/cuda-h264-crop.mp4");
        for crop in [
            None,
            Some(fvid_media_info::CropRect {
                x: 2,
                y: 4,
                width: 314,
                height: 230,
            }),
        ] {
            let destination = std::env::temp_dir().join(format!(
                "fvid-owned-crop-{}-{}.mkv",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let dimensions = crop.map_or((318, 238), |crop| (crop.width as u32, crop.height as u32));
            let stats = hw_filter(
                &source,
                &destination,
                &HwFilterOptions {
                    crop,
                    horizontal_flip: true,
                    vertical_flip: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(stats.backend, "owned-cuda-nvdec-nvenc");
            assert_eq!((stats.width, stats.height), dimensions);
            assert_eq!(stats.host_frame_copies, 0);
            assert_eq!(stats.video_frames, 8);
            let mut input = crate::owned_webm::WebmReader::open(
                BufReader::new(std::fs::File::open(&destination).unwrap()),
                Default::default(),
            )
            .unwrap();
            input.scan_all().unwrap();
            assert_eq!(
                (input.tracks[0].width, input.tracks[0].height),
                (u64::from(dimensions.0), u64::from(dimensions.1))
            );
            assert_eq!(input.tracks[0].crop, [0; 4]);
            assert_eq!(input.packets.len(), 8);
            let mut decoder = fvid_codecs::codec::avc_decoder::AvcDecoder::new(
                &input.tracks[0].codec_private,
                16 << 20,
            )
            .unwrap();
            for index in 0..input.packets.len() {
                assert_eq!(
                    decoder
                        .decode_order(&input.read_packet(index).unwrap())
                        .unwrap()
                        .unwrap()
                        .dimensions(),
                    (dimensions.0 as usize, dimensions.1 as usize)
                );
            }
            std::fs::remove_file(destination).unwrap();
        }
    }
    #[test]
    fn synthetic_sps_crop_is_removed_before_native_encoding() {
        let mut source = MovieSource::open(
            std::io::Cursor::new(
                include_bytes!("../../../tests/fixtures/playback-errors/avc-display-crop.mp4")
                    .as_slice(),
            ),
            Default::default(),
        )
        .unwrap();
        assert_eq!(source.coded_dimensions(), (64, 48));
        assert_eq!(
            source.video_metadata().options.video.unwrap().crop,
            [0, 2, 0, 2]
        );
        // This specific nonzero SPS crop triggered the old production refusal.
        let plan = plan(&mut source, &HwFilterOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            (plan.transform.out_width, plan.transform.out_height),
            (62, 46)
        );
        assert_eq!((plan.transform.crop_x, plan.transform.crop_y), (0, 0));
        let nested = super::plan(
            &mut source,
            &HwFilterOptions {
                crop: Some(fvid_media_info::CropRect {
                    x: 2,
                    y: 4,
                    width: 58,
                    height: 40,
                }),
                horizontal_flip: true,
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            (
                nested.transform.crop_x,
                nested.transform.crop_y,
                nested.transform.out_width,
                nested.transform.out_height
            ),
            (2, 4, 58, 40)
        );
        assert!(
            super::plan(
                &mut source,
                &HwFilterOptions {
                    crop: Some(fvid_media_info::CropRect {
                        x: 0,
                        y: 0,
                        width: 64,
                        height: 48
                    }),
                    ..Default::default()
                }
            )
            .is_err()
        );
        let mut decoder = fvid_codecs::codec::avc_decoder::AvcDecoder::new(
            &source.track().configuration,
            16 << 20,
        )
        .unwrap();
        source.rewind_packets();
        let mut packet = Vec::new();
        let mut count = 0;
        while source.read_next(&mut packet).unwrap().is_some() {
            assert_eq!(
                decoder.decode_order(&packet).unwrap().unwrap().dimensions(),
                (62, 46)
            );
            count += 1;
        }
        assert_eq!(count, 8);
    }
    #[test]
    fn empty_edits_and_repeated_ranges_use_container_clock_in_production_intervals() {
        for bytes in [
            include_bytes!("../../../tests/fixtures/playback-errors/edit-empty-spans.mov")
                .as_slice(),
            include_bytes!("../../../tests/fixtures/playback-errors/hevc-cuda-edit-repeat.mp4")
                .as_slice(),
            include_bytes!(
                "../../../tests/fixtures/playback-errors/hevc-main10-cuda-edit-repeat.mp4"
            )
            .as_slice(),
        ] {
            let mut source =
                MovieSource::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
            let options = HwFilterOptions {
                interval: Some((0, 500_000)),
                ..Default::default()
            };
            let plan = plan(&mut source, &options)
                .unwrap()
                .expect("blank and repeated movie events must be admitted");
            let scale = source.track().timescale;
            assert_eq!(plan.interval, Some((0, i64::from(scale) / 2)));
            let events = crate::owned_nvdec_movie::select_presentations(
                source.movie_presentations(1000).unwrap(),
                plan.interval,
                crate::owned_nvdec_movie::IntervalSelection::FrameStarts,
            )
            .unwrap();
            assert_eq!(events[0].start, 0);
            assert!(events[0].sample.is_none());
            assert!(events.iter().any(|e| e.sample.is_some()));
            assert!(events.iter().all(|e| e.start < i64::from(scale) / 2));
            assert!(source.read_next(&mut Vec::new()).unwrap().unwrap().sync);
        }
    }
    #[test]
    fn synthetic_production_route_qualifies_packets_without_driver() {
        let mut source = MovieSource::open(
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
        let interval_plan = super::plan(
            &mut source,
            &HwFilterOptions {
                interval: Some((250_000, 750_000)),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(interval_plan.interval, Some((3000, 9000)));
        assert!(
            super::plan(
                &mut source,
                &HwFilterOptions {
                    interval: Some((1, 500_000)),
                    ..Default::default()
                }
            )
            .is_err()
        );
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
            .is_some()
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
