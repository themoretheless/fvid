//! Full CLI wall-clock suite: Y4M filters + media export (CPU↔CPU, GPU↔GPU).
//!
//! ```sh
//! cargo build --release --no-default-features --features media-cuda --target-dir target-media-cuda
//! cargo build --release --no-default-features --features media --target-dir target-media
//! cargo build --release --no-default-features --target-dir target-cpu
//! cargo bench --bench ffmpeg_compare
//! ```
//!
//! Keep media-cuda in `target-media-cuda/` so `cargo bench` does not overwrite it
//! when rebuilding the default `target/release/fvid` (Y4M/gpu features).
//!
//! Env:
//! - `FVID_BENCH_BIN_FULL` — release binary with media-cuda (default `target-media-cuda/release/fvid`)
//! - `FVID_BENCH_BIN_MEDIA` — media without CUDA (default `target-media/release/fvid`)
//! - `FVID_BENCH_BIN_CPU`  — cpu-only Y4M binary (default `target-cpu/release/fvid`)
//! - `FVID_BENCH_SKIP_MEDIA=1` — Y4M only
//! - `FVID_BENCH_SKIP_Y4M=1` — media only
//! - `FVID_BENCH_MIN_DELTA` — require fvid median < ffmpeg×(1−delta); default `0.15` (>+15%). `0` disables.
use airbug_bench::analysis::median;
use airbug_bench::{Config, Run, Suite};
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

struct Bins {
    full: Option<PathBuf>,
    media: Option<PathBuf>,
    cpu: Option<PathBuf>,
}

fn resolve_bins() -> airbug_bench::Result<Bins> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let full = std::env::var_os("FVID_BENCH_BIN_FULL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let isolated = exe(&root, "target-media-cuda/release/fvid");
            if isolated.is_file() {
                isolated
            } else {
                exe(&root, "target/release/fvid")
            }
        });
    let media = std::env::var_os("FVID_BENCH_BIN_MEDIA")
        .map(PathBuf::from)
        .unwrap_or_else(|| exe(&root, "target-media/release/fvid"));
    let cpu = std::env::var_os("FVID_BENCH_BIN_CPU")
        .map(PathBuf::from)
        .unwrap_or_else(|| exe(&root, "target-cpu/release/fvid"));
    let bins = Bins {
        full: full.is_file().then_some(full),
        media: media.is_file().then_some(media),
        cpu: cpu.is_file().then_some(cpu),
    };
    if bins.full.is_none() && bins.media.is_none() && bins.cpu.is_none() {
        return Err(airbug_bench::error(
            "no fvid binaries found; build:\n  cargo build --release --no-default-features --features media-cuda --target-dir target-media-cuda\n  cargo build --release --no-default-features --features media --target-dir target-media\n  cargo build --release --no-default-features --target-dir target-cpu",
        ));
    }
    Ok(bins)
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
    let out = Command::new(bin).args(["media", "hw-filter"]).output().ok();
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
            path.to_str()
                .ok_or_else(|| airbug_bench::error("non-utf8 path"))?,
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
            path.to_str()
                .ok_or_else(|| airbug_bench::error("non-utf8 path"))?,
        ])
        .status()?;
    if !status.success() {
        return Err(airbug_bench::error("ffmpeg MP4 fixture failed"));
    }
    Ok(path)
}

/// Closed-GOP H.264 (`-bf 0`, fixed keyint) so strict streamcopy trim can cut on IDR times.
fn make_mp4_gop(
    dir: &Path,
    width: u32,
    height: u32,
    seconds: u32,
) -> airbug_bench::Result<PathBuf> {
    let path = dir.join(format!("{width}x{height}-{seconds}s.gop.h264.mp4"));
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
            "-g",
            "30",
            "-keyint_min",
            "30",
            "-sc_threshold",
            "0",
            "-bf",
            "0",
            "-an",
            path.to_str()
                .ok_or_else(|| airbug_bench::error("non-utf8 path"))?,
        ])
        .status()?;
    if !status.success() {
        return Err(airbug_bench::error("ffmpeg GOP MP4 fixture failed"));
    }
    Ok(path)
}

fn run_checked(mut cmd: Command) -> airbug_bench::Result<()> {
    let output = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).output()?;
    if !output.status.success() {
        return Err(airbug_bench::error(format!(
            "command failed: {cmd:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn fresh_out(dir: &Path, prefix: &str) -> PathBuf {
    let n = OUT_SEQ.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{prefix}-{n}.mp4"))
}

fn fresh_mkv(dir: &Path, prefix: &str) -> PathBuf {
    let n = OUT_SEQ.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{prefix}-{n}.mkv"))
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

fn media_fvid_remux(bin: &Path, src: &Path, out: &Path) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args([
        "media",
        "remux",
        src.to_str().unwrap(),
        out.to_str().unwrap(),
        "--quiet",
    ]);
    cmd
}

fn media_ffmpeg_remux(src: &Path, out: &Path) -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-y", "-v", "error", "-i"])
        .arg(src)
        .args(["-c", "copy", "-an"])
        .arg(out);
    cmd
}

/// CPU filter fair-pair: decode → optional crop/hflip/vflip → discard (no encode).
fn media_fvid_cpu(bin: &Path, src: &Path, _out: &Path, flags: &[String]) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(["media", "decode", src.to_str().unwrap(), "--quiet"]);
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
        "--quiet",
    ]);
    cmd.args(flags);
    cmd
}

fn media_ffmpeg_cpu(src: &Path, _out: &Path, vf: &str) -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-v", "error", "-i"]).arg(src);
    if !vf.is_empty() {
        cmd.args(["-vf", vf]);
    }
    cmd.args(["-an", "-f", "null", "-"]);
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
    // FFmpeg lacks CUDA crop/flip filters in the qualified build, so those
    // operations require a host filter round-trip. Identity remains zero-copy.
    if !vf.is_empty() {
        cmd.args(["-vf", &format!("hwdownload,format=nv12,{vf},hwupload_cuda")]);
    }
    cmd.args(["-c:v", "h264_nvenc", "-preset", "p1", "-bf", "0", "-an"])
        .arg(out);
    cmd
}

fn media_fvid_trim(bin: &Path, src: &Path, out: &Path, from: &str, to: &str) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args([
        "media",
        "trim",
        src.to_str().unwrap(),
        out.to_str().unwrap(),
        "--from",
        from,
        "--to",
        to,
        "--quiet",
    ]);
    cmd
}

fn media_ffmpeg_trim(src: &Path, out: &Path, from: &str, to: &str) -> Command {
    // -ss after -i: demux from start (fair vs strict fvid trim that scans packets).
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-y", "-v", "error", "-i"])
        .arg(src)
        .args(["-ss", from, "-to", to, "-c", "copy", "-an"])
        .arg(out);
    cmd
}

fn media_fvid_concat(bin: &Path, a: &Path, b: &Path, out: &Path) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args([
        "media",
        "concat",
        out.to_str().unwrap(),
        a.to_str().unwrap(),
        b.to_str().unwrap(),
        "--quiet",
    ]);
    cmd
}

fn media_ffmpeg_concat(list: &Path, out: &Path) -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-nostdin", "-y", "-v", "error", "-f", "concat", "-safe", "0", "-i",
    ])
    .arg(list)
    .args(["-c", "copy", "-an"])
    .arg(out);
    cmd
}

fn media_ffmpeg_cut_cpu(src: &Path, _out: &Path, from: &str, to: &str) -> Command {
    // Decode-window twin: demux seek to the same exact closed-GOP boundary.
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-v", "error", "-ss", from, "-to", to, "-i"])
        .arg(src)
        .args(["-an", "-f", "null", "-"]);
    cmd
}

fn media_ffmpeg_cut_gpu(src: &Path, out: &Path, from: &str, to: &str) -> Command {
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
        "-ss",
        from,
        "-to",
        to,
        "-i",
    ])
    .arg(src)
    .args(["-c:v", "h264_nvenc", "-preset", "p1", "-bf", "0", "-an"])
    .arg(out);
    cmd
}

fn media_fvid_decode(bin: &Path, src: &Path, hw: bool) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(["media", "decode", src.to_str().unwrap(), "--quiet"]);
    if hw {
        cmd.args(["--device", "0"]);
    }
    cmd
}

fn media_fvid_cut_cpu(bin: &Path, src: &Path, from: &str, to: &str) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args([
        "media",
        "decode",
        src.to_str().unwrap(),
        "--from",
        from,
        "--to",
        to,
        "--quiet",
    ]);
    cmd
}

fn media_ffmpeg_decode(src: &Path, hw: bool) -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-v", "error"]);
    if hw {
        cmd.args(["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
    }
    cmd.args(["-i"]).arg(src);
    cmd.args(["-an", "-f", "null", "-"]);
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

fn min_delta() -> f64 {
    std::env::var("FVID_BENCH_MIN_DELTA")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.15)
}

fn case_median(run: &Run, case_suffix: &str) -> Option<f64> {
    let case_id = format!("ffmpeg_compare/{case_suffix}");
    let metric = run
        .cases
        .iter()
        .find(|c| c.id == case_id)?
        .metrics
        .iter()
        .find(|m| m.id == "wall")?;
    let mut values = Vec::new();
    for o in run
        .observations
        .iter()
        .filter(|o| o.case == case_id && o.metric == "wall")
    {
        if let Ok(Some(n)) = o.number() {
            values.push(if metric.statistic == "batch_total" {
                n / o.operations as f64
            } else {
                n
            });
        }
    }
    (!values.is_empty()).then(|| median(&values))
}

fn enforce_fair_pair_deltas(run: &Run) -> airbug_bench::Result<()> {
    let gate = min_delta();
    let pairs = [
        (
            "media/fvid_cpu/1080p/subtitle_remux",
            "media/ffmpeg_cpu/1080p/subtitle_remux",
            "cpu/subtitle_remux",
        ),
        (
            "media/fvid_cpu/1080p/copy",
            "media/ffmpeg_cpu/1080p/copy",
            "cpu/copy",
        ),
        (
            "media/fvid_cpu/1080p/crop",
            "media/ffmpeg_cpu/1080p/crop",
            "cpu/crop",
        ),
        (
            "media/fvid_cpu/1080p/hflip",
            "media/ffmpeg_cpu/1080p/hflip",
            "cpu/hflip",
        ),
        (
            "media/fvid_cpu/1080p/vflip",
            "media/ffmpeg_cpu/1080p/vflip",
            "cpu/vflip",
        ),
        (
            "media/fvid_cpu/1080p/fused",
            "media/ffmpeg_cpu/1080p/fused",
            "cpu/fused",
        ),
        (
            "media/fvid_cpu/1080p/cut",
            "media/ffmpeg_cpu/1080p/cut",
            "cpu/cut",
        ),
        (
            "media/fvid_cpu/1080p/trim",
            "media/ffmpeg_cpu/1080p/trim",
            "cpu/trim",
        ),
        (
            "media/fvid_cpu/1080p/concat",
            "media/ffmpeg_cpu/1080p/concat",
            "cpu/concat",
        ),
        (
            "media/fvid_cpu/1080p/decode",
            "media/ffmpeg_cpu/1080p/decode",
            "cpu/decode",
        ),
        (
            "media/fvid_gpu/1080p/copy",
            "media/ffmpeg_gpu/1080p/copy",
            "gpu/copy",
        ),
        (
            "media/fvid_gpu/1080p/crop",
            "media/ffmpeg_gpu/1080p/crop",
            "gpu/crop",
        ),
        (
            "media/fvid_gpu/1080p/hflip",
            "media/ffmpeg_gpu/1080p/hflip",
            "gpu/hflip",
        ),
        (
            "media/fvid_gpu/1080p/vflip",
            "media/ffmpeg_gpu/1080p/vflip",
            "gpu/vflip",
        ),
        (
            "media/fvid_gpu/1080p/fused",
            "media/ffmpeg_gpu/1080p/fused",
            "gpu/fused",
        ),
        (
            "media/fvid_gpu/1080p/cut",
            "media/ffmpeg_gpu/1080p/cut",
            "gpu/cut",
        ),
        (
            "media/fvid_gpu/1080p/decode",
            "media/ffmpeg_gpu/1080p/decode",
            "gpu/decode",
        ),
    ];
    println!("\n| Pair | fvid | ffmpeg | Δ |");
    println!("|---|---:|---:|---:|");
    let mut failed = Vec::new();
    for (fvid, ffmpeg, label) in pairs {
        let (fv, ff) = match (case_median(run, fvid), case_median(run, ffmpeg)) {
            (Some(fv), Some(ff)) => (fv, ff),
            (None, None) => continue,
            (Some(_), None) => {
                failed.push(format!("{label}: missing ffmpeg observation"));
                continue;
            }
            (None, Some(_)) => {
                failed.push(format!("{label}: missing fvid observation"));
                continue;
            }
        };
        if ff <= 0.0 {
            continue;
        }
        let delta = 1.0 - fv / ff;
        println!("| {label} | {fv:.0} | {ff:.0} | {:+.1}% |", delta * 100.0);
        if gate > 0.0 && delta <= gate {
            failed.push(format!(
                "{label}: {:+.1}% (need >{:+.0}%)",
                delta * 100.0,
                gate * 100.0
            ));
        }
    }
    if gate <= 0.0 {
        println!("\nFair-pair delta gate disabled (FVID_BENCH_MIN_DELTA={gate}).");
        return Ok(());
    }
    if failed.is_empty() {
        println!(
            "\nFair-pair gate: all present pairs > +{:.0}%.",
            gate * 100.0
        );
        Ok(())
    } else {
        Err(airbug_bench::error(format!(
            "fair-pair gate failed:\n  {}",
            failed.join("\n  ")
        )))
    }
}

fn main() -> airbug_bench::Result<()> {
    require_ffmpeg()?;
    let bins = resolve_bins()?;
    if let Some(ref p) = bins.full {
        eprintln!(
            "bench: fvid/full → {} ({} bytes)",
            p.display(),
            std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
        );
    }
    if let Some(ref p) = bins.media {
        eprintln!(
            "bench: fvid/media → {} ({} bytes)",
            p.display(),
            std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
        );
    }
    if let Some(ref p) = bins.cpu {
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
        samples: 21,
        warmup: std::time::Duration::from_millis(100),
        sample_time: std::time::Duration::from_millis(25),
        max_iterations: 16,
    });

    let bin_full = bins.full.clone();
    let bin_cpu = bins.cpu.clone();
    let bin_media = bins.media.clone();

    if !skip_y4m() {
        let y4m_cases = [
            ("720p", 1280u32, 720u32, 180u32),
            ("1080p", 1920, 1080, 120),
            ("2160p", 3840, 2160, 60),
        ];
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

    if !skip_media() {
        let media_cpu = bin_media
            .as_ref()
            .filter(|b| fvid_has_media(b))
            .cloned()
            .or_else(|| bin_full.as_ref().filter(|b| fvid_has_media(b)).cloned())
            .or_else(|| bin_cpu.as_ref().filter(|b| fvid_has_media(b)).cloned());
        let media_gpu = bin_full
            .as_ref()
            .filter(|b| fvid_has_media(b) && fvid_has_hw_filter(b))
            .cloned();

        if media_cpu.is_none() && media_gpu.is_none() {
            eprintln!(
                "bench: skip media — binary lacks `media` (build with --features media / media-cuda)"
            );
        } else {
            let nvenc = ffmpeg_has_encoder("h264_nvenc");
            let hw = media_gpu.is_some();
            eprintln!(
                "bench: media export — fvid_media_cpu={} hw-filter={} ffmpeg_nvenc={}",
                media_cpu.is_some(),
                hw,
                nvenc
            );

            let (label, w, h, seconds, frames) = ("1080p", 1920u32, 1080u32, 10u32, 300u64);
            let cut_from = "2";
            let cut_to = "5";
            let cut_frames = 90u64;
            let trim_from = "8";
            let trim_to = "10";
            let trim_frames = 60u64;
            let src_cpu_short = make_mp4(&dir, w, h, 2)?;
            let src_cpu_decode = make_mp4(&dir, w, h, 3)?;
            let src_cpu = make_mp4(&dir, w, h, 5)?;
            let src = make_mp4(&dir, w, h, seconds)?;
            let src_subtitle = {
                let captions = dir.join("captions.srt");
                if !captions.is_file() {
                    std::fs::write(&captions, "1\n00:00:00,125 --> 00:00:00,625\nalpha\n")?;
                }
                captions
            };
            let src_gpu_decode = make_mp4(&dir, w, h, 1)?;
            let src_gpu_fused = make_mp4(&dir, w, h, 20)?;
            let src_gpu_hflip = make_mp4(&dir, w, h, 30)?;
            let src_gop = make_mp4_gop(&dir, w, h, seconds)?;
            let out_dir = dir.clone();

            // Concat parts: two 3s closed-GOP clips with identical codec params.
            let concat_a = make_mp4_gop(&dir, w, h, 3)?;
            let concat_b = {
                let path = dir.join(format!("{w}x{h}-3s.gop.b.h264.mp4"));
                if !path.is_file() {
                    std::fs::copy(&concat_a, &path)?;
                }
                path
            };
            let concat_list = dir.join("concat.txt");
            if !concat_list.is_file() {
                let a = concat_a.to_str().unwrap().replace('\\', "/");
                let b = concat_b.to_str().unwrap().replace('\\', "/");
                std::fs::write(&concat_list, format!("file '{a}'\nfile '{b}'\n"))?;
            }

            // Warm paths.
            if let Some(ref media_bin) = media_cpu {
                let out = fresh_out(&out_dir, "warm-cpu");
                run_checked(media_fvid_remux(media_bin, &src_cpu, &out))?;
                remove_quiet(&out);
                run_checked(media_fvid_cpu(
                    media_bin,
                    &src_cpu,
                    &out_dir.join("warm-null"),
                    &["--vflip".into()],
                ))?;
                run_checked(media_fvid_decode(media_bin, &src_cpu, false))?;
                let out = fresh_out(&out_dir, "warm-trim");
                run_checked(media_fvid_trim(
                    media_bin, &src_gop, &out, trim_from, trim_to,
                ))?;
                remove_quiet(&out);
                let out = fresh_out(&out_dir, "warm-cat");
                run_checked(media_fvid_concat(media_bin, &concat_a, &concat_b, &out))?;
                remove_quiet(&out);
                run_checked(media_fvid_cut_cpu(media_bin, &src_gop, cut_from, cut_to))?;
            }
            {
                let out = fresh_out(&out_dir, "warm-ffcpu");
                run_checked(media_ffmpeg_remux(&src_cpu, &out))?;
                remove_quiet(&out);
                run_checked(media_ffmpeg_cpu(&src_cpu, &out, "vflip"))?;
                run_checked(media_ffmpeg_decode(&src_cpu, false))?;
            }
            if let Some(ref media_bin) = media_gpu {
                let out = fresh_out(&out_dir, "warm-gpu");
                run_checked(media_fvid_gpu(media_bin, &src, &out, &[]))?;
                remove_quiet(&out);
                run_checked(media_fvid_decode(media_bin, &src, true))?;
                let out = fresh_out(&out_dir, "warm-gcut");
                run_checked(media_fvid_gpu(
                    media_bin,
                    &src,
                    &out,
                    &[
                        "--from".into(),
                        cut_from.into(),
                        "--to".into(),
                        cut_to.into(),
                    ],
                ))?;
                remove_quiet(&out);
            }
            if nvenc && hw {
                let out = fresh_out(&out_dir, "warm-ffgpu");
                run_checked(media_ffmpeg_gpu(&src, &out, ""))?;
                remove_quiet(&out);
                run_checked(media_ffmpeg_decode(&src, true))?;
            }

            {
                let src_c = src_subtitle.clone();
                let out_dir_c = out_dir.clone();
                suite
                    .bench(
                        &format!("media/ffmpeg_cpu/{label}/subtitle_remux"),
                        move || {
                            let out = fresh_mkv(&out_dir_c, "xsub");
                            run_checked(media_ffmpeg_remux(&src_c, &out))
                                .expect("ffmpeg subtitle remux");
                            remove_quiet(&out);
                        },
                    )
                    .tag("media")
                    .tag("ffmpeg_cpu")
                    .work_units("packets", 1)
                    .parameter("resolution", label)
                    .parameter("op", "subtitle_remux");
            }
            if let Some(ref media_bin) = media_cpu {
                let bin = media_bin.clone();
                let src_c = src_subtitle.clone();
                let out_dir_c = out_dir.clone();
                suite
                    .bench(
                        &format!("media/fvid_cpu/{label}/subtitle_remux"),
                        move || {
                            let out = fresh_mkv(&out_dir_c, "fsub");
                            run_checked(media_fvid_remux(&bin, &src_c, &out))
                                .expect("fvid subtitle remux");
                            remove_quiet(&out);
                        },
                    )
                    .tag("media")
                    .tag("fvid_cpu")
                    .work_units("packets", 1)
                    .parameter("resolution", label)
                    .parameter("op", "subtitle_remux");
            }

            for (op, flags, vf) in editor_ops(w, h) {
                let src_c = if op == "vflip" {
                    src_cpu_short.clone()
                } else if op == "crop" || op == "hflip" {
                    src_cpu_decode.clone()
                } else {
                    src_cpu.clone()
                };
                let cpu_frames = if op == "vflip" {
                    60
                } else if op == "crop" || op == "hflip" {
                    90
                } else {
                    150
                };
                let gpu_frames = if op == "hflip" {
                    900
                } else if op == "fused" {
                    600
                } else if op == "decode" {
                    30
                } else {
                    frames
                };
                let out_dir_c = out_dir.clone();
                let vf_c = vf.clone();
                let op_is_copy = op == "copy";
                suite
                    .bench(&format!("media/ffmpeg_cpu/{label}/{op}"), move || {
                        let out = fresh_out(&out_dir_c, "xc");
                        if op_is_copy {
                            run_checked(media_ffmpeg_remux(&src_c, &out))
                                .expect("ffmpeg_cpu remux");
                        } else {
                            run_checked(media_ffmpeg_cpu(&src_c, &out, &vf_c)).expect("ffmpeg_cpu");
                        }
                        remove_quiet(&out);
                    })
                    .tag("media")
                    .tag("ffmpeg_cpu")
                    .work_units("frames", cpu_frames)
                    .parameter("resolution", label)
                    .parameter("op", op);

                if let Some(ref media_bin) = media_cpu {
                    let bin = media_bin.clone();
                    let src_c = if op == "vflip" {
                        src_cpu_short.clone()
                    } else if op == "crop" || op == "hflip" {
                        src_cpu_decode.clone()
                    } else {
                        src_cpu.clone()
                    };
                    let out_dir_c = out_dir.clone();
                    let flags_c = flags.clone();
                    let op_is_copy = op == "copy";
                    suite
                        .bench(&format!("media/fvid_cpu/{label}/{op}"), move || {
                            let out = fresh_out(&out_dir_c, "fc");
                            if op_is_copy {
                                run_checked(media_fvid_remux(&bin, &src_c, &out))
                                    .expect("fvid_cpu remux");
                            } else {
                                run_checked(media_fvid_cpu(&bin, &src_c, &out, &flags_c))
                                    .expect("fvid_cpu media");
                            }
                            remove_quiet(&out);
                        })
                        .tag("media")
                        .tag("fvid_cpu")
                        .work_units("frames", cpu_frames)
                        .parameter("resolution", label)
                        .parameter("op", op);
                }

                if nvenc && hw {
                    let src_c = if op == "hflip" {
                        src_gpu_hflip.clone()
                    } else if op == "fused" {
                        src_gpu_fused.clone()
                    } else {
                        src.clone()
                    };
                    let out_dir_c = out_dir.clone();
                    let vf_c = vf.clone();
                    suite
                        .bench(&format!("media/ffmpeg_gpu/{label}/{op}"), move || {
                            let out = fresh_out(&out_dir_c, "xg");
                            run_checked(media_ffmpeg_gpu(&src_c, &out, &vf_c)).expect("ffmpeg_gpu");
                            remove_quiet(&out);
                        })
                        .tag("media")
                        .tag("ffmpeg_gpu")
                        .work_units("frames", gpu_frames)
                        .parameter("resolution", label)
                        .parameter("op", op);
                }

                if let Some(ref media_bin) = media_gpu {
                    let bin = media_bin.clone();
                    let src_c = if op == "hflip" {
                        src_gpu_hflip.clone()
                    } else if op == "fused" {
                        src_gpu_fused.clone()
                    } else {
                        src.clone()
                    };
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
                        .work_units("frames", gpu_frames)
                        .parameter("resolution", label)
                        .parameter("op", op);
                }
            }

            // cut window (CPU = decode→null; GPU = NVENC reencode)
            {
                let src_c = src_gop.clone();
                suite
                    .bench(&format!("media/ffmpeg_cpu/{label}/cut"), move || {
                        run_checked(media_ffmpeg_cut_cpu(
                            &src_c,
                            Path::new(""),
                            cut_from,
                            cut_to,
                        ))
                        .expect("ffmpeg cut");
                    })
                    .tag("media")
                    .tag("ffmpeg_cpu")
                    .work_units("frames", cut_frames)
                    .parameter("resolution", label)
                    .parameter("op", "cut");
            }
            if let Some(ref media_bin) = media_cpu {
                let bin = media_bin.clone();
                let src_c = src_gop.clone();
                suite
                    .bench(&format!("media/fvid_cpu/{label}/cut"), move || {
                        run_checked(media_fvid_cut_cpu(&bin, &src_c, cut_from, cut_to))
                            .expect("fvid cut");
                    })
                    .tag("media")
                    .tag("fvid_cpu")
                    .work_units("frames", cut_frames)
                    .parameter("resolution", label)
                    .parameter("op", "cut");
            }
            if nvenc && hw {
                let src_c = src.clone();
                let out_dir_c = out_dir.clone();
                suite
                    .bench(&format!("media/ffmpeg_gpu/{label}/cut"), move || {
                        let out = fresh_out(&out_dir_c, "xgcut");
                        run_checked(media_ffmpeg_cut_gpu(&src_c, &out, cut_from, cut_to))
                            .expect("ffmpeg gpu cut");
                        remove_quiet(&out);
                    })
                    .tag("media")
                    .tag("ffmpeg_gpu")
                    .work_units("frames", cut_frames)
                    .parameter("resolution", label)
                    .parameter("op", "cut");
            }
            if let Some(ref media_bin) = media_gpu {
                let bin = media_bin.clone();
                let src_c = src.clone();
                let out_dir_c = out_dir.clone();
                suite
                    .bench(&format!("media/fvid_gpu/{label}/cut"), move || {
                        let out = fresh_out(&out_dir_c, "fgcut");
                        run_checked(media_fvid_gpu(
                            &bin,
                            &src_c,
                            &out,
                            &[
                                "--from".into(),
                                cut_from.into(),
                                "--to".into(),
                                cut_to.into(),
                            ],
                        ))
                        .expect("fvid gpu cut");
                        remove_quiet(&out);
                    })
                    .tag("media")
                    .tag("fvid_gpu")
                    .work_units("frames", cut_frames)
                    .parameter("resolution", label)
                    .parameter("op", "cut");
            }

            // trim streamcopy (CPU only) — closed-GOP source
            {
                let src_c = src_gop.clone();
                let out_dir_c = out_dir.clone();
                suite
                    .bench(&format!("media/ffmpeg_cpu/{label}/trim"), move || {
                        let out = fresh_out(&out_dir_c, "xtr");
                        run_checked(media_ffmpeg_trim(&src_c, &out, trim_from, trim_to))
                            .expect("ffmpeg trim");
                        remove_quiet(&out);
                    })
                    .tag("media")
                    .tag("ffmpeg_cpu")
                    .work_units("frames", trim_frames)
                    .parameter("resolution", label)
                    .parameter("op", "trim");
            }
            if let Some(ref media_bin) = media_cpu {
                let bin = media_bin.clone();
                let src_c = src_gop.clone();
                let out_dir_c = out_dir.clone();
                suite
                    .bench(&format!("media/fvid_cpu/{label}/trim"), move || {
                        let out = fresh_out(&out_dir_c, "ftr");
                        run_checked(media_fvid_trim(&bin, &src_c, &out, trim_from, trim_to))
                            .expect("fvid trim");
                        remove_quiet(&out);
                    })
                    .tag("media")
                    .tag("fvid_cpu")
                    .work_units("frames", trim_frames)
                    .parameter("resolution", label)
                    .parameter("op", "trim");
            }

            // concat streamcopy (CPU only)
            {
                let list = concat_list.clone();
                let out_dir_c = out_dir.clone();
                suite
                    .bench(&format!("media/ffmpeg_cpu/{label}/concat"), move || {
                        let out = fresh_out(&out_dir_c, "xcat");
                        run_checked(media_ffmpeg_concat(&list, &out)).expect("ffmpeg concat");
                        remove_quiet(&out);
                    })
                    .tag("media")
                    .tag("ffmpeg_cpu")
                    .work_units("frames", 180)
                    .parameter("resolution", label)
                    .parameter("op", "concat");
            }
            if let Some(ref media_bin) = media_cpu {
                let bin = media_bin.clone();
                let a = concat_a.clone();
                let b = concat_b.clone();
                let out_dir_c = out_dir.clone();
                suite
                    .bench(&format!("media/fvid_cpu/{label}/concat"), move || {
                        let out = fresh_out(&out_dir_c, "fcat");
                        run_checked(media_fvid_concat(&bin, &a, &b, &out)).expect("fvid concat");
                        remove_quiet(&out);
                    })
                    .tag("media")
                    .tag("fvid_cpu")
                    .work_units("frames", 180)
                    .parameter("resolution", label)
                    .parameter("op", "concat");
            }

            // decode → null
            {
                let src_c = src_cpu_decode.clone();
                suite
                    .bench(&format!("media/ffmpeg_cpu/{label}/decode"), move || {
                        run_checked(media_ffmpeg_decode(&src_c, false)).expect("ffmpeg decode");
                    })
                    .tag("media")
                    .tag("ffmpeg_cpu")
                    .work_units("frames", 90)
                    .parameter("resolution", label)
                    .parameter("op", "decode");
            }
            if let Some(ref media_bin) = media_cpu {
                let bin = media_bin.clone();
                let src_c = src_cpu_decode.clone();
                suite
                    .bench(&format!("media/fvid_cpu/{label}/decode"), move || {
                        run_checked(media_fvid_decode(&bin, &src_c, false)).expect("fvid decode");
                    })
                    .tag("media")
                    .tag("fvid_cpu")
                    .work_units("frames", 90)
                    .parameter("resolution", label)
                    .parameter("op", "decode");
            }
            if hw {
                let src_c = src_gpu_decode.clone();
                suite
                    .bench(&format!("media/ffmpeg_gpu/{label}/decode"), move || {
                        run_checked(media_ffmpeg_decode(&src_c, true)).expect("ffmpeg nvdec");
                    })
                    .tag("media")
                    .tag("ffmpeg_gpu")
                    .work_units("frames", frames)
                    .parameter("resolution", label)
                    .parameter("op", "decode");
            }
            if let Some(ref media_bin) = media_gpu {
                let bin = media_bin.clone();
                let src_c = src_gpu_decode.clone();
                suite
                    .bench(&format!("media/fvid_gpu/{label}/decode"), move || {
                        run_checked(media_fvid_decode(&bin, &src_c, true)).expect("fvid nvdec");
                    })
                    .tag("media")
                    .tag("fvid_gpu")
                    .work_units("frames", frames)
                    .parameter("resolution", label)
                    .parameter("op", "decode");
            }
        }
    }

    if std::env::args().any(|a| a == "--list") {
        for id in suite.list("") {
            println!("{id}");
        }
        return Ok(());
    }

    let filter = std::env::var("FVID_BENCH_FILTER").unwrap_or_default();
    let run = suite.run(&filter)?;
    println!("{}", airbug_bench::report::markdown(&run)?);
    println!(
        "\nFair pairs: `y4m/fvid_cpu`↔`y4m/ffmpeg`, `media/fvid_cpu`↔`media/ffmpeg_cpu`, `media/fvid_gpu`↔`media/ffmpeg_gpu`.\nOps: copy/crop/hflip/vflip/fused, subtitle remux, cut, trim, concat, decode.\n`y4m/fvid_full` is CUDA Y4M (PCIe round-trip), not NVENC export.\n"
    );
    enforce_fair_pair_deltas(&run)?;
    Ok(())
}
