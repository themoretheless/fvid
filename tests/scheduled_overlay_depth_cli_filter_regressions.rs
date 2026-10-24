use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/scheduled-overlay-depth-cli-filters")
}

#[test]
fn scheduled_overlay_supports_high_depth_mixed_precision_transposed_subsampling_matroska_default_duration_cli_filters_and_track_dispositions() {
    let dir = fixture_dir();
    let main_10 = dir.join("main-420p10.y4m");
    let over_8 = dir.join("overlay-420p8.y4m");
    let main_422 = dir.join("main-422-aspect.y4m");
    let over_440 = dir.join("overlay-440-transposed.y4m");
    let avc_mkv = dir.join("avc-default-duration-disabled-opus.mkv");
    let over_5fps = dir.join("overlay-4x4-5fps.y4m");
    let avc_mp4 = dir.join("avc-disabled-aac.mp4");

    let temp = std::env::temp_dir().join(format!(
        "fvid-scheduled-overlay-depth-cli-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // 1. 10-bit main Y4M (`C420p10`) + 8-bit foreground Y4M (`C420`) scheduled overlay
    // in `owned_y4m_decode` and `owned_lossless`:
    let transform_10 = fvid::media_info::DecodeTransform {
        overlay: Some(fvid::media_info::OverlaySpec {
            path: over_8.clone(),
            x: 2,
            y: 2,
        }),
        ..Default::default()
    };
    let lossless_10 = fvid::media_info::LosslessTransform {
        overlay: transform_10.overlay.clone(),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_lossless::supports(&main_10, &lossless_10, &Default::default()),
        "10-bit YUV420p10 main with 8-bit YUV420 overlay must be supported by owned_lossless/owned_y4m_decode"
    );
    let mut frames_10 = Vec::new();
    let stats_10 = fvid_media::owned_y4m_decode::visit_reader_transformed(
        BufReader::new(File::open(&main_10).unwrap()),
        &transform_10,
        |header, pixels, _pts, _dur| {
            assert_eq!(header.depth(), 10);
            let samples: Vec<u16> = pixels
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            frames_10.push(samples);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(stats_10.video_frames, 3);
    assert_eq!(stats_10.pixel_format, "yuv420p10le");
    // Frame 0 (t=0) and Frame 1 (t=0.25s) use overlay frame 0 (Y=50<<2=200, Cb=80<<2=320, Cr=160<<2=640)
    // Frame 2 (t=0.50s) uses overlay frame 1 (Y=200<<2=800, Cb=90<<2=360, Cr=170<<2=680)
    for (idx, samples) in frames_10.iter().enumerate() {
        let base_y = (100 + idx * 20) as u16;
        let (ov_y, ov_cb, ov_cr) = if idx < 2 {
            (200u16, 320u16, 640u16)
        } else {
            (800u16, 360u16, 680u16)
        };
        assert_eq!(samples[0], base_y);
        for pos in [10, 11, 14, 15] {
            assert_eq!(samples[pos], ov_y, "frame {idx} pos {pos}");
        }
        assert_eq!(&samples[16..20], &[512, 512, 512, ov_cb]);
        assert_eq!(&samples[20..24], &[512, 512, 512, ov_cr]);
    }

    // 2. Transposed YUV422 (A2:1 -> YUV440 A1:2) with YUV440 A1:2 scheduled overlay:
    let transform_422_transposed = fvid::media_info::DecodeTransform {
        transpose: Some(fvid::media_info::TransposeMode::Clock),
        overlay: Some(fvid::media_info::OverlaySpec {
            path: over_440.clone(),
            x: 0,
            y: 2,
        }),
        ..Default::default()
    };
    let lossless_422_transposed = fvid::media_info::LosslessTransform {
        transpose: transform_422_transposed.transpose.clone(),
        overlay: transform_422_transposed.overlay.clone(),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_lossless::supports(
            &main_422,
            &lossless_422_transposed,
            &Default::default()
        ),
        "transposed YUV422 (becomes YUV440 A1:2) with YUV440 A1:2 overlay must be supported"
    );
    let stats_422 =
        fvid_media::owned_y4m_decode::decode_video_transformed(&main_422, transform_422_transposed)
            .unwrap();
    assert_eq!(stats_422.width, 2);
    assert_eq!(stats_422.height, 4);
    assert_eq!(stats_422.pixel_format, "yuv440p");
    assert_eq!(stats_422.video_frames, 2);

    // 3. Matroska AVC with DefaultDuration=400_000_000ns (F5:2 token) + scheduled Y4M overlay
    // in `owned_video_decode` and `owned_lossless`:
    let mkv_decode_transform = fvid::media_info::DecodeTransform {
        overlay: Some(fvid::media_info::OverlaySpec {
            path: over_5fps.clone(),
            x: 0,
            y: 0,
        }),
        ..Default::default()
    };
    let mkv_decode_stats = fvid_media::decode_video_transformed(&avc_mkv, mkv_decode_transform.clone())
        .expect("owned_video_decode should decode Matroska AVC with overlay");
    assert_eq!(mkv_decode_stats.video_frames, 2);
    let out_mkv_overlay = temp.join("mkv-overlay-clock.mkv");
    let mkv_lossless_transform = fvid::media_info::LosslessTransform {
        overlay: mkv_decode_transform.overlay.clone(),
        ..Default::default()
    };
    fvid::media::transcode_lossless(
        &avc_mkv,
        &out_mkv_overlay,
        mkv_lossless_transform,
        &Default::default(),
    )
    .unwrap();
    let mut reader = fvid::playback_native::NativeReader::software(
        BufReader::new(File::open(&out_mkv_overlay).unwrap()),
        usize::MAX,
    )
    .unwrap();
    assert!(reader.read_frame().unwrap());
    let rgb0 = reader.rgb()[0];
    assert!(reader.read_frame().unwrap());
    let rgb1 = reader.rgb()[0];
    // At t=0.0s (frame 0), 5fps overlay is at index 0 (Y=32, Cr=160 -> limited-range R=70).
    // At t=0.40s (frame 1), 5fps overlay is at index 2 (Y=210, Cr=160 -> limited-range R=255), NOT frame 0!
    assert_eq!(
        (rgb0, rgb1),
        (70, 255),
        "Matroska DefaultDuration overlay clock should advance from frame 0 ({rgb0}) to frame 2 ({rgb1})"
    );
    // Also verify `fvid media decode` succeeds on `avc-default-duration-disabled-opus.mkv` with `--overlay`:
    let mkv_decode_cli = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            avc_mkv.to_str().unwrap(),
            "--overlay",
            over_5fps.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        mkv_decode_cli.status.success(),
        "fvid media decode on Matroska AVC with DefaultDuration + overlay failed: {}",
        String::from_utf8_lossy(&mkv_decode_cli.stderr)
    );

    // 4. CLI `fvid media transcode-lossless`, `fvid media overlay`, and `fvid media decode --overlay`
    // with `--rotate`, `--curves`, `--fade`, `--hqdn3d`, `--tmix`, `--lagfun`, and `--metadata`:
    let out_cli_lossless = temp.join("cli-lossless-curves-fade.mkv");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "transcode-lossless",
            main_10.to_str().unwrap(),
            out_cli_lossless.to_str().unwrap(),
            "--overlay",
            over_8.to_str().unwrap(),
            "--overlay-x",
            "2",
            "--overlay-y",
            "2",
            "--rotate",
            "90",
            "--curves",
            "master='0/0 1/1'",
            "--fade",
            "t=in:st=0:d=0.5",
            "--hqdn3d",
            "2:1:2:1",
            "--tmix",
            "frames=2:weights='1 1'",
            "--lagfun",
            "decay=0.5",
            "--metadata",
            "TITLE=OverlayCliParity",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "transcode-lossless with overlay + rotate/curves/fade/hqdn3d/tmix/lagfun/metadata failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let probed_cli = fvid_media::owned_webm_probe::probe_webm(&out_cli_lossless).unwrap();
    assert_eq!(probed_cli.streams[0].width, 4);
    assert_eq!(probed_cli.streams[0].height, 4);
    assert_eq!(
        probed_cli.metadata.get("title").map(String::as_str),
        Some("OverlayCliParity")
    );

    // Also verify `fvid media decode` with `--overlay` and `--rotate 90 --curves ... --hqdn3d ...`:
    let decode_cli = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            main_10.to_str().unwrap(),
            "--overlay",
            over_8.to_str().unwrap(),
            "--rotate",
            "90",
            "--curves",
            "master='0/0 1/1'",
            "--hqdn3d",
            "2:1:2:1",
            "--tmix",
            "frames=2:weights='1 1'",
            "--lagfun",
            "decay=0.5",
        ])
        .output()
        .unwrap();
    assert!(
        decode_cli.status.success(),
        "decode --overlay with rotate/curves/hqdn3d/tmix/lagfun failed: {}",
        String::from_utf8_lossy(&decode_cli.stderr)
    );
    let decode_json: serde_json::Value = serde_json::from_slice(&decode_cli.stdout).unwrap();
    assert_eq!(decode_json["backend"], "fvid");
    assert_eq!(decode_json["video_frames"], 3);

    // Also verify `fvid media overlay` with `--from` / `--to` interval and `--rotate`:
    let out_cli_interval = temp.join("cli-overlay-interval.mkv");
    let interval_cli = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "overlay",
            main_10.to_str().unwrap(),
            over_8.to_str().unwrap(),
            out_cli_interval.to_str().unwrap(),
            "--overlay-x",
            "2",
            "--overlay-y",
            "2",
            "--from",
            "0.25",
            "--to",
            "0.75",
            "--rotate",
            "180",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        interval_cli.status.success(),
        "overlay with --from/--to and --rotate failed: {}",
        String::from_utf8_lossy(&interval_cli.stderr)
    );
    assert_eq!(
        fvid::native_media::decode_video(&out_cli_interval)
            .unwrap()
            .video_frames,
        2
    );

    // 5. Track disposition (`enabled` and `default`) preservation in `native_lossless` (MP4)
    // and `native_lossless_y4m` (Matroska):
    let out_mp4_lossless = temp.join("mp4-dispositions.mkv");
    fvid::native_export::transcode_mp4_ffv1(&avc_mp4, &out_mp4_lossless, None, None).unwrap();
    let mp4_mkv_info = fvid_media::owned_webm_probe::probe_webm(&out_mp4_lossless).unwrap();
    assert_eq!(mp4_mkv_info.streams.len(), 3);
    assert!(mp4_mkv_info.streams[0].disposition & 1 != 0);
    assert!(mp4_mkv_info.streams[1].disposition & 1 != 0);
    assert!(
        mp4_mkv_info.streams[2].disposition & 1 == 0,
        "disabled MP4 companion audio track must not become default in native_lossless output"
    );
    let mp4_webm = fvid::container::webm::WebmReader::open(
        BufReader::new(File::open(&out_mp4_lossless).unwrap()),
        Default::default(),
    )
    .unwrap();
    assert!(mp4_webm.track_enabled(mp4_webm.tracks[0].number));
    assert!(mp4_webm.track_enabled(mp4_webm.tracks[1].number));
    assert!(
        !mp4_webm.track_enabled(mp4_webm.tracks[2].number),
        "disabled MP4 companion audio track must preserve FlagEnabled=0 in native_lossless output"
    );

    let out_mkv_lossless = temp.join("mkv-dispositions.mkv");
    fvid::native_export::transcode_mp4_ffv1(&avc_mkv, &out_mkv_lossless, None, None).unwrap();
    let mkv_info = fvid_media::owned_webm_probe::probe_webm(&out_mkv_lossless).unwrap();
    assert_eq!(mkv_info.streams.len(), 2);
    assert!(mkv_info.streams[0].disposition & 1 != 0);
    assert!(
        mkv_info.streams[1].disposition & 1 == 0,
        "non-default Matroska companion audio track must preserve FlagDefault=0 in native_lossless_y4m output"
    );
    let mkv_webm = fvid::container::webm::WebmReader::open(
        BufReader::new(File::open(&out_mkv_lossless).unwrap()),
        Default::default(),
    )
    .unwrap();
    assert_eq!(mkv_webm.tracks.len(), 2);
    assert!(mkv_webm.track_enabled(mkv_webm.tracks[0].number));
    assert!(
        !mkv_webm.track_enabled(mkv_webm.tracks[1].number),
        "disabled Matroska companion audio track must preserve FlagEnabled=0 in native_lossless_y4m output"
    );

    let _ = std::fs::remove_dir_all(&temp);
}
