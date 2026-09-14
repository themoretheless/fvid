#!/usr/bin/env python3
"""Multi-metric show-bench: fvid CPU/GPU vs FFmpeg CPU/GPU.

Metrics per engine:
  - median_ms / min_ms / max_ms / fps
  - cold_ms (first timed sample after process start)
  - peak_working_set_mb (Windows; optional)
  - output_bytes (media writers)
  - host_frame_copies (fvid hw-filter JSON)

Dimensions:
  - family: y4m | media
  - op: copy | hflip | fused
  - resolution / frames

Writes benchmarks/show-results.json
"""
from __future__ import annotations

import datetime
import json
import pathlib
import platform
import statistics
import subprocess
import sys
import tempfile
import threading
import time

from common import ROOT, prepend_cuda_bin, release_binary, run_timed

DATA = ROOT / "benchmarks" / "show_data"
OUT = ROOT / "benchmarks" / "show-results.json"
ROUNDS = 5
WARMUP = 1


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, stderr=subprocess.PIPE, **kw)


def which(name: str) -> str | None:
    from shutil import which as _which

    return _which(name)


def make_y4m(w: int, h: int, n: int) -> pathlib.Path:
    DATA.mkdir(parents=True, exist_ok=True)
    path = DATA / f"{w}x{h}-{n}.y4m"
    if path.exists():
        return path
    run(
        [
            "ffmpeg",
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            f"testsrc2=size={w}x{h}:rate=30",
            "-frames:v",
            str(n),
            "-pix_fmt",
            "yuv420p",
            "-strict",
            "-1",
            str(path),
        ]
    )
    return path


def make_mp4(w: int, h: int, seconds: int) -> pathlib.Path:
    DATA.mkdir(parents=True, exist_ok=True)
    path = DATA / f"{w}x{h}-{seconds}s.h264.mp4"
    if path.exists():
        return path
    run(
        [
            "ffmpeg",
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            f"testsrc2=size={w}x{h}:rate=30",
            "-t",
            str(seconds),
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-pix_fmt",
            "yuv420p",
            "-an",
            str(path),
        ]
    )
    return path


def has_encoder(name: str) -> bool:
    probe = subprocess.run(
        ["ffmpeg", "-hide_banner", "-encoders"], capture_output=True, text=True
    )
    return name in (probe.stdout or "")


def gpu_name() -> str | None:
    try:
        out = subprocess.check_output(
            ["nvidia-smi", "--query-gpu=name", "--format=csv,noheader"],
            text=True,
            stderr=subprocess.DEVNULL,
        )
        return out.strip().splitlines()[0].strip()
    except Exception:
        return None


def _peak_ws_mb_windows(pid: int, stop: threading.Event) -> float | None:
    """Poll WorkingSetSize while process runs; return peak MiB."""
    try:
        import ctypes
        from ctypes import wintypes

        class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
            _fields_ = [
                ("cb", wintypes.DWORD),
                ("PageFaultCount", wintypes.DWORD),
                ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t),
            ]

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)
        PROCESS_QUERY_INFORMATION = 0x0400
        PROCESS_VM_READ = 0x0010
        handle = kernel32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
        if not handle:
            return None
        peak = 0
        counters = PROCESS_MEMORY_COUNTERS()
        counters.cb = ctypes.sizeof(counters)
        try:
            while not stop.is_set():
                if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                    peak = max(peak, int(counters.PeakWorkingSetSize))
                time.sleep(0.005)
            if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                peak = max(peak, int(counters.PeakWorkingSetSize))
        finally:
            kernel32.CloseHandle(handle)
        return peak / (1024 * 1024) if peak else None
    except Exception:
        return None


def run_timed_rich(command: list[str], *, capture_stdout: bool = False):
    """Return (elapsed_s, returncode, peak_ws_mb, stdout_text)."""
    stdout_target = subprocess.PIPE if capture_stdout else subprocess.DEVNULL
    start = time.perf_counter_ns()
    process = subprocess.Popen(
        command, stdout=stdout_target, stderr=subprocess.PIPE
    )
    stop = threading.Event()
    peak_holder: list[float | None] = [None]
    poller = None
    if platform.system() == "Windows":
        poller = threading.Thread(
            target=lambda: peak_holder.__setitem__(
                0, _peak_ws_mb_windows(process.pid, stop)
            ),
            daemon=True,
        )
        poller.start()
    out_b, err_b = process.communicate()
    stop.set()
    if poller:
        poller.join(timeout=1.0)
    elapsed = (time.perf_counter_ns() - start) / 1e9
    text = (out_b or b"").decode("utf-8", errors="replace") if capture_stdout else ""
    return elapsed, process.returncode, peak_holder[0], text, (err_b or b"").decode(
        "utf-8", errors="replace"
    )


def summarize(samples: list[float], frames: int, peaks: list[float], cold_ms: float) -> dict:
    med = statistics.median(samples)
    out = {
        "seconds": samples,
        "median_ms": med * 1000,
        "min_ms": min(samples) * 1000,
        "max_ms": max(samples) * 1000,
        "fps": frames / med,
        "cold_ms": cold_ms,
        "ms_per_frame": (med * 1000) / frames,
    }
    if peaks:
        out["peak_working_set_mb"] = statistics.median(peaks)
        out["peak_working_set_mb_max"] = max(peaks)
    return out


def time_stream_cmd(cmd: list[str], frames: int) -> dict:
    """Y4M → null sink (stdout discarded)."""
    # cold = first process invocation (includes DLL load)
    cold_s, code, peak, _, err = run_timed_rich(cmd)
    if code:
        return {"error": err[-500:] or f"exit {code}"}
    cold_ms = cold_s * 1000
    # warmup already done via cold; more warmups if needed
    for _ in range(max(0, WARMUP - 1)):
        e, c, _, _, err = run_timed_rich(cmd)
        if c:
            return {"error": err[-500:] or f"exit {c}"}
    samples, peaks = [], []
    for _ in range(ROUNDS):
        e, c, peak, _, err = run_timed_rich(cmd)
        if c:
            return {"error": err[-500:] or f"exit {c}"}
        samples.append(e)
        if peak is not None:
            peaks.append(peak)
    return summarize(samples, frames, peaks, cold_ms)


def time_file_cmd(
    cmd: list[str], out_path: pathlib.Path, frames: int, *, capture_json: bool = False
) -> dict:
    def once():
        if out_path.exists():
            out_path.unlink()
        return run_timed_rich(cmd, capture_stdout=capture_json)

    cold_s, code, peak, out_text, err = once()
    if code:
        return {"error": err[-500:] or f"exit {code}"}
    cold_ms = cold_s * 1000
    host_copies = None
    if capture_json and out_text.strip():
        try:
            host_copies = json.loads(out_text).get("host_frame_copies")
        except json.JSONDecodeError:
            pass
    for _ in range(max(0, WARMUP - 1)):
        e, c, _, _, err = once()
        if c:
            return {"error": err[-500:] or f"exit {c}"}
    samples, peaks, sizes = [], [], []
    for _ in range(ROUNDS):
        e, c, peak, out_text, err = once()
        if c:
            return {"error": err[-500:] or f"exit {c}"}
        samples.append(e)
        if peak is not None:
            peaks.append(peak)
        if out_path.exists():
            sizes.append(out_path.stat().st_size)
        if capture_json and out_text.strip() and host_copies is None:
            try:
                host_copies = json.loads(out_text).get("host_frame_copies")
            except json.JSONDecodeError:
                pass
    stats = summarize(samples, frames, peaks, cold_ms)
    if sizes:
        stats["output_bytes"] = int(statistics.median(sizes))
    if host_copies is not None:
        stats["host_frame_copies"] = host_copies
    return stats


def y4m_ffmpeg(src: pathlib.Path, filters: str, *, gpu: bool) -> list[str]:
    cmd = ["ffmpeg", "-nostdin", "-v", "error"]
    if gpu:
        cmd += ["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]
        cmd += ["-f", "yuv4mpegpipe", "-i", str(src)]
        vf = "hwdownload,format=yuv420p"
        if filters:
            vf = f"{vf},{filters}"
        cmd += ["-vf", vf]
    else:
        cmd += ["-i", str(src), "-an", "-sn"]
        if filters:
            cmd += ["-vf", filters]
    cmd += [
        "-c:v",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        "-strict",
        "-1",
        "-f",
        "yuv4mpegpipe",
        "-",
    ]
    return cmd


def fvid_y4m(bin_path: pathlib.Path, src: pathlib.Path, args: list[str], backend: str) -> list[str]:
    return [str(bin_path), str(src), "-", *args, "--backend", backend]


def main() -> int:
    prepend_cuda_bin()
    if not which("ffmpeg"):
        print("ffmpeg not on PATH", file=sys.stderr)
        return 1
    bin_full = release_binary()
    if not bin_full.is_file():
        print(f"missing {bin_full}; cargo build --release --features media-cuda", file=sys.stderr)
        return 1

    nvenc = has_encoder("h264_nvenc")
    cases = []
    metrics_catalog = [
        {"id": "fps", "label": "Throughput (fps)", "higher_is_better": True},
        {"id": "median_ms", "label": "Median wall time (ms)", "higher_is_better": False},
        {"id": "cold_ms", "label": "Cold start (ms)", "higher_is_better": False},
        {"id": "ms_per_frame", "label": "ms / frame", "higher_is_better": False},
        {"id": "peak_working_set_mb", "label": "Peak working set (MiB)", "higher_is_better": False},
        {"id": "output_bytes", "label": "Output size (bytes)", "higher_is_better": False},
        {"id": "host_frame_copies", "label": "Host frame copies", "higher_is_better": False},
    ]

    # --- Editor basics on raw Y4M (1080p timeline ops) ---
    # Common NLE-style primitives: passthrough, crop, flips, crop+flip, fused.
    w, h, n = 1920, 1080, 180
    src = make_y4m(w, h, n)
    cx, cy = (w // 8 // 2 * 2), (h // 8 // 2 * 2)
    cw, ch = w // 2, h // 2
    crop_arg = f"{cx}:{cy}:{cw}:{ch}"
    crop_ff = f"crop={cw}:{ch}:{cx}:{cy}"
    editor_ops = [
        ("copy", "Прогон без правок", [], ""),
        ("crop", "Кадрирование", ["--crop", crop_arg], crop_ff),
        ("hflip", "Зеркало горизонталь", ["--hflip"], "hflip"),
        ("vflip", "Зеркало вертикаль", ["--vflip"], "vflip"),
        (
            "crop_hflip",
            "Кадр + зеркало H",
            ["--crop", crop_arg, "--hflip"],
            f"{crop_ff},hflip",
        ),
        (
            "fused",
            "Кадр + зеркала HV",
            ["--crop", crop_arg, "--hflip", "--vflip"],
            f"{crop_ff},hflip,vflip",
        ),
    ]
    for op_name, editor_title, fvid_args, ff_filter in editor_ops:
        engines: dict[str, list[str]] = {
            "fvid_cpu": fvid_y4m(bin_full, src, fvid_args, "cpu"),
            "fvid_gpu": fvid_y4m(bin_full, src, fvid_args, "cuda"),
            "ffmpeg_cpu": y4m_ffmpeg(src, ff_filter, gpu=False),
        }
        measured = {}
        for eng, cmd in engines.items():
            print(f"editor/y4m {op_name} {eng}…", flush=True)
            measured[eng] = time_stream_cmd(cmd, n)
        cases.append(
            {
                "family": "editor_y4m",
                "label": "1080p",
                "resolution": f"{w}x{h}",
                "frames": n,
                "op": op_name,
                "editor_title": editor_title,
                "matrix": list(engines),
                "engines": measured,
            }
        )

    # --- 720p spot-check (same editor ops, lighter clip) ---
    w720, h720, n720 = 1280, 720, 300
    src720 = make_y4m(w720, h720, n720)
    cx720, cy720 = (w720 // 8 // 2 * 2), (h720 // 8 // 2 * 2)
    cw720, ch720 = w720 // 2, h720 // 2
    crop720 = f"{cx720}:{cy720}:{cw720}:{ch720}"
    crop720_ff = f"crop={cw720}:{ch720}:{cx720}:{cy720}"
    for op_name, editor_title, fvid_args, ff_filter in [
        ("copy", "Прогон без правок", [], ""),
        ("hflip", "Зеркало горизонталь", ["--hflip"], "hflip"),
        (
            "fused",
            "Кадр + зеркала HV",
            ["--crop", crop720, "--hflip", "--vflip"],
            f"{crop720_ff},hflip,vflip",
        ),
    ]:
        engines = {
            "fvid_cpu": fvid_y4m(bin_full, src720, fvid_args, "cpu"),
            "fvid_gpu": fvid_y4m(bin_full, src720, fvid_args, "cuda"),
            "ffmpeg_cpu": y4m_ffmpeg(src720, ff_filter, gpu=False),
        }
        measured = {}
        for eng, cmd in engines.items():
            print(f"editor/y4m720 {op_name} {eng}…", flush=True)
            measured[eng] = time_stream_cmd(cmd, n720)
        cases.append(
            {
                "family": "editor_y4m",
                "label": "720p",
                "resolution": f"{w720}x{h720}",
                "frames": n720,
                "op": op_name,
                "editor_title": editor_title,
                "matrix": list(engines),
                "engines": measured,
            }
        )

    # --- Editor basics on media (export pipeline) ---
    mp4 = make_mp4(1920, 1080, 5)
    frames_media = 5 * 30
    media_editor = [
        ("copy", "Экспорт без правок", ""),
        ("crop", "Экспорт с кадром", f"crop={cw}:{ch}:{cx}:{cy}"),
        ("hflip", "Экспорт с зеркалом H", "hflip"),
        ("vflip", "Экспорт с зеркалом V", "vflip"),
        (
            "fused",
            "Экспорт кадр+зеркала",
            f"crop={cw}:{ch}:{cx}:{cy},hflip,vflip",
        ),
    ]
    with tempfile.TemporaryDirectory(prefix="fvid-show-") as tmp:
        tmp_path = pathlib.Path(tmp)
        for op_name, editor_title, ff_vf in media_editor:
            out_fc = tmp_path / f"fvid_cpu_{op_name}.mp4"
            out_fg = tmp_path / f"fvid_gpu_{op_name}.mp4"
            out_xc = tmp_path / f"ff_cpu_{op_name}.mp4"
            out_xg = tmp_path / f"ff_gpu_{op_name}.mp4"

            fvid_flags: list[str] = []
            if op_name == "crop":
                fvid_flags = ["--crop", crop_arg]
            elif op_name == "hflip":
                fvid_flags = ["--hflip"]
            elif op_name == "vflip":
                fvid_flags = ["--vflip"]
            elif op_name == "fused":
                fvid_flags = ["--crop", crop_arg, "--hflip", "--vflip"]

            jobs: list[tuple[str, list[str], pathlib.Path, bool]] = [
                (
                    "fvid_cpu",
                    [
                        str(bin_full),
                        "media",
                        "transcode",
                        str(mp4),
                        str(out_fc),
                        "--encoder",
                        "libx264",
                        "--encoder-option",
                        "preset=veryfast",
                        *fvid_flags,
                    ],
                    out_fc,
                    False,
                ),
                (
                    "fvid_gpu",
                    [
                        str(bin_full),
                        "media",
                        "hw-filter",
                        str(mp4),
                        str(out_fg),
                        *fvid_flags,
                    ],
                    out_fg,
                    True,
                ),
            ]
            ff_cpu = ["ffmpeg", "-nostdin", "-y", "-v", "error", "-i", str(mp4)]
            if ff_vf:
                ff_cpu += ["-vf", ff_vf]
            ff_cpu += [
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-an",
                str(out_xc),
            ]
            jobs.append(("ffmpeg_cpu", ff_cpu, out_xc, False))
            if nvenc:
                ff_gpu = [
                    "ffmpeg",
                    "-nostdin",
                    "-y",
                    "-v",
                    "error",
                    "-hwaccel",
                    "cuda",
                    "-hwaccel_output_format",
                    "cuda",
                    "-i",
                    str(mp4),
                ]
                if ff_vf:
                    ff_gpu += [
                        "-vf",
                        f"hwdownload,format=nv12,{ff_vf},hwupload_cuda",
                    ]
                ff_gpu += [
                    "-c:v",
                    "h264_nvenc",
                    "-preset",
                    "p1",
                    "-an",
                    str(out_xg),
                ]
                jobs.append(("ffmpeg_gpu", ff_gpu, out_xg, False))

            measured = {}
            for name, cmd, out_path, cap_json in jobs:
                print(f"editor/media {op_name} {name}…", flush=True)
                measured[name] = time_file_cmd(
                    cmd, out_path, frames_media, capture_json=cap_json
                )
            cases.append(
                {
                    "family": "editor_media",
                    "label": "1080p_5s",
                    "resolution": "1920x1080",
                    "frames": frames_media,
                    "op": op_name,
                    "editor_title": editor_title,
                    "matrix": list(measured),
                    "engines": measured,
                }
            )

    report = {
        "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "cpu": platform.processor() or "unknown",
        "gpu": gpu_name(),
        "fvid": str(bin_full),
        "ffmpeg": subprocess.check_output(["ffmpeg", "-version"], text=True).splitlines()[0],
        "ffmpeg_nvenc": nvenc,
        "method": (
            f"{WARMUP} warmup + {ROUNDS} timed rounds; editor basics = copy/crop/hflip/vflip/"
            "crop_hflip/fused on Y4M + media export variants; cold_ms = first process invoke."
        ),
        "metrics": metrics_catalog,
        "cases": cases,
    }
    OUT.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"Saved {OUT}", flush=True)

    for case in cases:
        print(f"\n== {case['family']} {case['label']}/{case['op']} ==")
        for name in case["matrix"]:
            s = case["engines"][name]
            if "error" in s:
                print(f"  {name}: ERROR {s['error'][:100]}")
            else:
                bits = [f"{s['median_ms']:.1f}ms", f"{s['fps']:.0f}fps", f"cold={s['cold_ms']:.0f}ms"]
                if "peak_working_set_mb" in s:
                    bits.append(f"rss={s['peak_working_set_mb']:.0f}MiB")
                if "output_bytes" in s:
                    bits.append(f"out={s['output_bytes']}")
                if "host_frame_copies" in s:
                    bits.append(f"copies={s['host_frame_copies']}")
                print(f"  {name}: " + " · ".join(bits))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
