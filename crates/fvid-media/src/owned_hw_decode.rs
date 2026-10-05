//! GPU-only decode admission and execution using owned MP4/AVC/HEVC code.
use crate::owned_nvdec_source::MovieSource;
use std::{
    io::{BufReader, Read, Seek},
    path::Path,
};

fn qualify<R: Read + Seek>(source: &mut MovieSource<R>) -> Result<(), String> {
    source.qualify_packets()
}

fn visible_dimensions<R: Read + Seek>(source: &MovieSource<R>) -> Result<(u32, u32), String> {
    let (width, height) = source.coded_dimensions();
    let [left, right, top, bottom] = source
        .video_metadata()
        .options
        .video
        .map_or([0; 4], |v| v.crop);
    let width = width
        .checked_sub(left)
        .and_then(|n| n.checked_sub(right))
        .filter(|n| *n > 0)
        .ok_or("invalid CUDA video horizontal crop")?;
    let height = height
        .checked_sub(top)
        .and_then(|n| n.checked_sub(bottom))
        .filter(|n| *n > 0)
        .ok_or("invalid CUDA video vertical crop")?;
    Ok((width, height))
}

/// Native-only decode: unsupported syntax is reported without a legacy retry.
pub fn decode_video_cuda(
    source: &Path,
    ordinal: usize,
) -> Result<fvid_media_info::DecodeStats, String> {
    let input = BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
    let mut source = open_qualified(input)?;
    let (width, height) = visible_dimensions(&source)?;
    let pixel_format = if source.bit_depth() == 10 {
        "cuda/p010"
    } else {
        "cuda/nv12"
    };
    let mut decoder = source.create_decoder(ordinal, 32, 2)?;
    let mut scratch = Vec::new();
    let mut count = 0u64;
    while let Some(packet) = source.decode_next(&mut decoder, &mut scratch)? {
        if let Some(frame) = packet.frame {
            // Mapping waits for reconstruction without copying pixels to CPU.
            let surface = decoder.map(&frame)?;
            decoder.unmap(surface.slot)?;
            count = count
                .checked_add(1)
                .ok_or("native CUDA decoded frame count overflow")?;
        }
    }
    decoder.close()?;
    Ok(fvid_media_info::DecodeStats {
        backend: "owned-cuda-nvdec",
        video_frames: count,
        width,
        height,
        pixel_format: pixel_format.into(),
        decode_errors: 0,
    })
}

fn open_qualified<R: Read + Seek>(input: R) -> Result<MovieSource<R>, String> {
    let mut source = MovieSource::open(input, Default::default())?;
    qualify(&mut source)?;
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_avc_main_and_main10_qualify_without_driver_or_libav() {
        for bytes in [
            include_bytes!("../../../tests/fixtures/playback-errors/control.mp4").as_slice(),
            include_bytes!("../../../tests/fixtures/hevc/main-ipb.mp4").as_slice(),
            include_bytes!("../../../tests/fixtures/hevc/main10-ipb.mp4").as_slice(),
        ] {
            let mut source =
                MovieSource::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
            qualify(&mut source).unwrap();
            assert!(source.read_next(&mut Vec::new()).unwrap().unwrap().sync);
        }
    }
    #[test]
    fn corrupt_synthetic_nal_keeps_precise_admission_error_without_driver() {
        let bytes =
            include_bytes!("../../../tests/fixtures/playback-errors/shared-mp4-corrupt-nal.mp4");
        let error = match open_qualified(std::io::Cursor::new(bytes.as_slice())) {
            Ok(_) => panic!("corrupt NAL must not reach device creation"),
            Err(error) => error,
        };
        assert!(error.contains("invalid NAL payload length"), "{error}");
        assert!(!error.contains("does not yet cover"), "{error}");
    }
    #[test]
    fn decode_statistics_use_visible_sps_crop_dimensions() {
        let bytes = include_bytes!("../../../tests/fixtures/playback-errors/avc-display-crop.mp4");
        let source =
            MovieSource::open(std::io::Cursor::new(bytes.as_slice()), Default::default()).unwrap();
        assert_eq!(source.coded_dimensions(), (64, 48));
        assert_eq!(visible_dimensions(&source).unwrap(), (62, 46));
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA AVC/HEVC Main/Main10 NVDEC"]
    fn production_decode_device_uses_owned_code_without_libav() {
        for (relative, count, dimensions, format) in [
            (
                "../../tests/fixtures/playback-errors/cuda-h264-crop.mp4",
                8,
                (318, 238),
                "cuda/nv12",
            ),
            (
                "../../tests/fixtures/playback-errors/cuda-hevc.mp4",
                17,
                (256, 192),
                "cuda/nv12",
            ),
            (
                "../../tests/fixtures/playback-errors/cuda-hevc-main10.mp4",
                17,
                (256, 192),
                "cuda/p010",
            ),
        ] {
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
            let stats = decode_video_cuda(&source, 0).unwrap();
            assert_eq!(stats.backend, "owned-cuda-nvdec");
            assert_eq!(stats.video_frames, count);
            assert_eq!((stats.width, stats.height), dimensions);
            assert_eq!(stats.pixel_format, format);
        }
    }
}
