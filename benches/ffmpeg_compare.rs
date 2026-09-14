//! Full CLI wall-clock suite: Y4M filters + media export (CPU↔CPU, GPU↔GPU).
//!
//! ```sh
//! cargo build --release --features media-cuda
//! cargo build --release --no-default-features --target-dir target-cpu
//! cargo bench --bench ffmpeg_compare
//! ```
//!
//! Env:
//! - `FVID_BENCH_BIN_FULL` — release binary with media-cuda (default `target/release/fvid`)
//! - `FVID_BENCH_BIN_CPU`  — cpu-only Y4M binary (default `target-cpu/release/fvid`)
//! - `FVID_BENCH_SKIP_MEDIA=1` — Y4M only
//! - `FVID_BENCH_SKIP_Y4M=1` — media only
use airbug_bench::{Config, Suite};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static OUT_SEQ: AtomicU64 = AtomicU64::new(0);

fn exe(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.join(rel);
    if cfg!(windows) {
        p.set_extension("exe");
    }
    p
}

fn resolve_bins() -> airbug_bench::Result<(Option<PathBuf>, Option<PathBuf>)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let full = std::env::var_os("FVID_BENCH_BIN_FULL")
        .map(PathBuf::from)
        .unwrap_or_else(|| exe(&root, "target/release/fvid"));
    let cpu = std::env::var_os("FVID_BENCH_BIN_CPU")
        .map(PathBuf::from)
        .unwrap_or_else(|| exe(&root, "target-cpu/release/fvid"));
    let full = full.is_file().then_some(full);
    let cpu = cpu.is_file().then_some(cpu);
    if full.is_none() && cpu.is_none() {
        return Err(airbug_bench::error(
            "no fvid binaries found; build:\n  cargo build --release --features media-cuda\n  cargo build --release --no-default-features --target-dir target-cpu",
        ));
    }
    Ok((full, cpu))
}

fn require_ffmpeg() -> airbug_bench::Result<()> {
    let ok = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(airbug_bench::error("ffmpeg not found on PATH"))
    }
}

fn ffmpeg_has_encoder(name: &str) -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(name))
        .unwrap_or(false)
}

fn fvid_has_media(bin: &Path) -> bool {
    Command::new(bin)
        .args(["media", "capabilities"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn fvid_has_hw_filter(bin: &Path) -> bool {
    // capabilities succeeds with media; hw-filter needs media-cuda — probe help text via failed run.
    let out = Command::new(bin)
        .args(["media", "hw-filter"])
        .output()
        .ok();
    out.map(|o| {
        let err = String::from_utf8_lossy(&o.stderr);
        let combined = format!("{}{}", String::from_utf8_lossy(&o.stdout), err);
        !combined.contains("requires cargo build") && !combined.contains("media-cuda")
    })
    .unwrap_or(false)
}

fn make_y4m(dir: &Path, width: u32, height: u32, frames: u32) -> airbug_bench::Result<PathBuf> {
    let path = dir.join(format!("{width}x{height}-{frames}.y4m"));
    if path.is_file() {
        return Ok(path);
    }
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size={width}x{height}:rate=30"),
            "-frames:v",
            &frames.to_string(),
            "-pix_fmt",
            "yuv420p",
            "-strict",
            "-1",
            path.to_str().ok_or_else(|| airbug_bench::error("non-utf8 path"))?,
        ])
        .status()?;
    if !status.success() {
        return Err(airbug_bench::error("ffmpeg Y4M fixture failed"));
    }
    Ok(path)
}

fn make_mp4(dir: &Path, width: u32, height: u32, seconds: u32) -> airbug_bench::Result<PathBuf> {
    let path = dir.join(format!("{width}x{height}-{seconds}s.h264.mp4"));
    if path.is_file() {
        return Ok(path);
    }
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size={width}x{height}:rate=30"),
            "-t",
            &seconds.to_string(),
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-an",
            path.to_str().ok_or_else(|| airbug_bench::error("non-utf8 path"))?,
        ])
        .status()?;
    if !status.success() {
        return Err(airbug_bench::error("ffmpeg MP4 fixture failed"));
    }
    Ok(path)
}

fn run_checked(mut cmd: Command) -> airbug_bench::Result<()> {
    let status = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).status()?;
    if !status.success() {
        return Err(airbug_bench::error(format!("command failed: {cmd:?}")));
    }
    Ok(())
}

fn fresh_out(dir: &Path, prefix: &str) -> PathBuf {
    let n = OUT_SEQ.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{prefix}-{n}.mp4"))
}

fn remove_quiet(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn y4m_fvid(bin: &Path, src: &Path, args: &[String]) -> Command {
    let mut cmd = Command::new(bin);
    cmd.arg(src).arg("-").args(args);
    cmd
}

fn y4m_ffmpeg(src: &Path, filters: &str) -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-v", "error", "-i"])
        .arg(src)
        .args(["-an", "-sn"]);
    if !filters.is_empty() {
        cmd.args(["-vf", filters]);
    }
    cmd.args([
        "-c:v",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        "-strict",
        "-1",
        "-f",
        "yuv4mpegpipe",
        "-",
    ]);
    cmd
}

fn media_fvid_cpu(bin: &Path, src: &Path, out: &Path, flags: &[String]) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args([
        "media",
        "transcode",
        src.to_str().unwrap(),
        out.to_str().unwrap(),
        "--encoder",
        "libx264",
        "--encoder-option",
        "preset=veryfast",
        "--encoder-option",
        "crf=23",
    ]);
    cmd.args(flags);
    cmd
}

fn media_fvid_gpu(bin: &Path, src: &Path, out: &Path, flags: &[String]) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args([
        "media",
        "hw-filter",
        src.to_str().unwrap(),
        out.to_str().unwrap(),
    ]);
    cmd.args(flags);
    cmd
}

fn media_ffmpeg_cpu(src: &Path, out: &Path, vf: &str) -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-y", "-v", "error", "-i"])
        .arg(src);
    if !vf.is_empty() {
        cmd.args(["-vf", vf]);
    }
    cmd.args([
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "23",
        "-an",
    ])
    .arg(out);
    cmd
}

fn media_ffmpeg_gpu(src: &Path, out: &Path, vf: &str) -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-nostdin",
        "-y",
        "-v",
        "error",
        "-hwaccel",
        "cuda",
        "-hwaccel_output_format",
        "cuda",
        "-i",
    ])
    .arg(src);
    if !vf.is_empty() {
        // Match historical show_bench GPU filter path (host bounce around software vf).
        cmd.args([
            "-vf",
            &format!("hwdownload,format=nv12,{vf},hwupload_cuda"),
        ]);
    }
    cmd.args(["-c:v", "h264_nvenc", "-preset", "p1", "-bf", "0", "-an"])
        .arg(out);
    cmd
}

fn editor_ops(w: u32, h: u32) -> Vec<(&'static str, Vec<String>, String)> {
    let crop_x = (w / 8 / 2 * 2) as usize;
    let crop_y = (h / 8 / 2 * 2) as usize;
    let crop_w = (w / 2) as usize;
    let crop_h = (h / 2) as usize;
    let crop_arg = format!("{crop_x}:{crop_y}:{crop_w}:{crop_h}");
    vec![
        ("copy", vec![], String::new()),
        (
            "crop",
            vec!["--crop".into(), crop_arg.clone()],
            format!("crop={crop_w}:{crop_h}:{crop_x}:{crop_y}"),
        ),
        ("hflip", vec!["--hflip".into()], "hflip".into()),
        ("vflip", vec!["--vflip".into()], "vflip".into()),
        (
            "fused",
            vec![
                "--crop".into(),
                crop_arg,
                "--hflip".into(),
                "--vflip".into(),
            ],
            format!("crop={crop_w}:{crop_h}:{crop_x}:{crop_y},hflip,vflip"),
        ),
    ]
}

fn skip_y4m() -> bool {
    std::env::var_os("FVID_BENCH_SKIP_Y4M").is_some()
}

fn skip_media() -> bool {
    std::env::var_os("FVID_BENCH_SKIP_MEDIA").is_some()
}

fn main() -> airbug_bench::Result<()> {
    require_ffmpeg()?;
    let (bin_full, bin_cpu) = resolve_bins()?;
    if let Some(ref p) = bin_full {
        eprintln!(
            "bench: fvid/full → {} ({} bytes)",
            p.display(),
            std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
        );
    }
    if let Some(ref p) = bin_cpu {
        eprintln!(
            "bench: fvid/cpu → {} ({} bytes)",
            p.display(),
            std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
        );
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let dir = std::env::temp_dir().join(format!("fvid-airbug-bench-{stamp}"));
    std::fs::create_dir_all(&dir)?;

    let mut suite = Suite::new("ffmpeg_compare");
    suite.config(Config {
        samples: 7,
        warmup: std::time::Duration::from_millis(100),
        sample_time: std::time::Duration::from_millis(25),
        max_iterations: 16,
    });

    // ── Y4M: fvid_cpu / fvid_full / ffmpeg (raw) ───────────────────────────
    if !skip_y4m() {
        let y4m_cases = [
            ("720p", 1280u32, 720u32, 180u32),
            ("1080p", 1920, 1080, 120),
            ("2160p", 3840, 2160, 60),
        ];
        // Y4M ops without standalone crop (fused covers crop+flips).
        let y4m_ops: &[&str] = &["copy", "hflip", "vflip", "fused"];

        for (label, w, h, frames) in y4m_cases {
            let src = make_y4m(&dir, w, h, frames)?;
            if let Some(ref bin) = bin_cpu {
                run_checked(y4m_fvid(bin, &src, &[]))?;
            }
            if let Some(ref bin) = bin_full {
                run_checked(y4m_fvid(bin, &src, &[]))?;
            }
            run_checked(y4m_ffmpeg(&src, ""))?;

            let ops = editor_ops(w, h);
            for op_name in y4m_ops {
                let (fvid_args, ff_filter) = ops
                    .iter()
                    .find(|(n, _, _)| n == op_name)
                    .map(|(_, a, f)| (a.clone(), f.clone()))
                    .expect("op");

                if let Some(ref bin) = bin_cpu {
                    let src_c = src.clone();
                    let bin_c = bin.clone();
                    let args = fvid_args.clone();
                    suite
                        .bench(&format!("y4m/fvid_cpu/{label}/{op_name}"), move || {
                            run_checked(y4m_fvid(&bin_c, &src_c, &args)).expect("fvid_cpu");
                        })
                        .tag("y4m")
                        .tag("fvid_cpu")
                        .work_units("frames", u64::from(frames))
                        .parameter("resolution", label)
                        .parameter("op", *op_name);
                }
                if let Some(ref bin) = bin_full {
                    let src_c = src.clone();
                    let bin_c = bin.clone();
                    let args = fvid_args.clone();
                    suite
                        .bench(&format!("y4m/fvid_full/{label}/{op_name}"), move || {
                            run_checked(y4m_fvid(&bin_c, &src_c, &args)).expect("fvid_full");
                        })
                        .tag("y4m")
                        .tag("fvid_full")
                        .work_units("frames", u64::from(frames))
                        .parameter("resolution", label)
                        .parameter("op", *op_name);
                }

                let src_ff = src.clone();
                let filter = ff_filter;
                suite
                    .bench(&format!("y4m/ffmpeg/{label}/{op_name}"), move || {
                        run_checked(y4m_ffmpeg(&src_ff, &filter)).expect("ffmpeg");
                    })
                    .tag("y4m")
                    .tag("ffmpeg")
                    .work_units("frames", u64::from(frames))
                    .parameter("resolution", label)
                    .parameter("op", *op_name);
            }
        }
    }

    // ── Media export: CPU↔CPU (x264 veryfast) + GPU↔GPU (NVENC p1) ─────────
    if !skip_media() {
        let media_bin = bin_full
            .as_ref()
            .filter(|b| fvid_has_media(b))
            .cloned()
            .or_else(|| bin_cpu.as_ref().filter(|b| fvid_has_media(b)).cloned());

        if media_bin.is_none() {
            eprintln!(
                "bench: skip media — binary lacks `media` (build with --features media-cuda)"
            );
        } else {
            let media_bin = media_bin.unwrap();
            let nvenc = ffmpeg_has_encoder("h264_nvenc");
            let hw = fvid_has_hw_filter(&media_bin);
            eprintln!(
                "bench: media export — fvid_media=yes hw-filter={} ffmpeg_nvenc={}",
                hw, nvenc
            );

            let (label, w, h, seconds, frames) = ("1080p", 1920u32, 1080u32, 5u32, 150u64);
            let src = make_mp4(&dir, w, h, seconds)?;
            let out_dir = dir.clone();

            // Warm each path once.
            {
                let out = fresh_out(&out_dir, "warm-cpu");
                run_checked(media_fvid_cpu(&media_bin, &src, &out, &[]))?;
                remove_quiet(&out);
            }
            {
                let out = fresh_out(&out_dir, "warm-ffcpu");
                run_checked(media_ffmpeg_cpu(&src, &out, ""))?;
                remove_quiet(&out);
            }
            if hw {
                let out = fresh_out(&out_dir, "warm-gpu");
                run_checked(media_fvid_gpu(&media_bin, &src, &out, &[]))?;
                remove_quiet(&out);
            }
            if nvenc {
                let out = fresh_out(&out_dir, "warm-ffgpu");
                run_checked(media_ffmpeg_gpu(&src, &out, ""))?;
                remove_quiet(&out);
            }

            for (op, flags, vf) in editor_ops(w, h) {
                let bin = media_bin.clone();
                let src_c = src.clone();
                let out_dir_c = out_dir.clone();
                let flags_c = flags.clone();
                suite
                    .bench(&format!("media/fvid_cpu/{label}/{op}"), move || {
                        let out = fresh_out(&out_dir_c, "fc");
                        run_checked(media_fvid_cpu(&bin, &src_c, &out, &flags_c))
                            .expect("fvid_cpu media");
                        remove_quiet(&out);
                    })
                    .tag("media")
                    .tag("fvid_cpu")
                    .work_units("frames", frames)
                    .parameter("resolution", label)
                    .parameter("op", op);

                let src_c = src.clone();
                let out_dir_c = out_dir.clone();
                let vf_c = vf.clone();
                suite
                    .bench(&format!("media/ffmpeg_cpu/{label}/{op}"), move || {
                        let out = fresh_out(&out_dir_c, "xc");
                        run_checked(media_ffmpeg_cpu(&src_c, &out, &vf_c)).expect("ffmpeg_cpu");
                        remove_quiet(&out);
                    })
                    .tag("media")
                    .tag("ffmpeg_cpu")
                    .work_units("frames", frames)
                    .parameter("resolution", label)
                    .parameter("op", op);

                if hw {
                    let bin = media_bin.clone();
                    let src_c = src.clone();
                    let out_dir_c = out_dir.clone();
                    let flags_c = flags.clone();
                    suite
                        .bench(&format!("media/fvid_gpu/{label}/{op}"), move || {
                            let out = fresh_out(&out_dir_c, "fg");
                            run_checked(media_fvid_gpu(&bin, &src_c, &out, &flags_c))
                                .expect("fvid_gpu");
                            remove_quiet(&out);
                        })
                        .tag("media")
                        .tag("fvid_gpu")
                        .work_units("frames", frames)
                        .parameter("resolution", label)
                        .parameter("op", op);
                }

                if nvenc {
                    let src_c = src.clone();
                    let out_dir_c = out_dir.clone();
                    let vf_c = vf.clone();
                    suite
                        .bench(&format!("media/ffmpeg_gpu/{label}/{op}"), move || {
                            let out = fresh_out(&out_dir_c, "xg");
                            run_checked(media_ffmpeg_gpu(&src_c, &out, &vf_c))
                                .expect("ffmpeg_gpu");
                            remove_quiet(&out);
                        })
                        .tag("media")
                        .tag("ffmpeg_gpu")
                        .work_units("frames", frames)
                        .parameter("resolution", label)
                        .parameter("op", op);
                }
            }
        }
    }

    if std::env::args().any(|a| a == "--list") {
        for id in suite.list("") {
            println!("{id}");
        }
        return Ok(());
    }

    let run = suite.run("")?;
    println!("{}", airbug_bench::report::markdown(&run)?);
    println!(
        "\nFair pairs: `y4m/fvid_cpu`↔`y4m/ffmpeg`, `media/fvid_cpu`↔`media/ffmpeg_cpu`, `media/fvid_gpu`↔`media/ffmpeg_gpu`.\n`y4m/fvid_full` is CUDA Y4M (PCIe round-trip), not NVENC export.\n"
    );
    Ok(())
}
