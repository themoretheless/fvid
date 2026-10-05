"""Shared helpers for validation/benchmark scripts (Windows + POSIX)."""
from __future__ import annotations

import os
import pathlib
import platform
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]


def release_binary(name="fvid"):
    path = ROOT / "target" / "release" / name
    if platform.system() == "Windows":
        path = path.with_suffix(".exe")
    return path


def prepend_cuda_bin():
    """Ensure CUDA 13 NVRTC DLLs are discoverable on Windows."""
    if platform.system() != "Windows":
        return None
    candidates = []
    if os.environ.get("CUDA_PATH"):
        candidates.append(pathlib.Path(os.environ["CUDA_PATH"]) / "bin")
    toolkit = pathlib.Path(r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA")
    if toolkit.is_dir():
        candidates.extend(sorted(toolkit.glob("v13.*/bin"), reverse=True))
    candidates = [directory for candidate in candidates
                  for directory in (candidate / "x64", candidate)]
    for candidate in candidates:
        if candidate.is_dir() and (candidate / "nvrtc64_130_0.dll").is_file():
            path = str(candidate)
            current = os.environ.get("PATH", "")
            if path.lower() not in current.lower():
                os.environ["PATH"] = path + os.pathsep + current
            return path
    return None


def run_timed(command, **kwargs):
    """Run a process and return (elapsed_seconds, returncode, max_rss_bytes_or_None)."""
    start = time.perf_counter_ns()
    process = subprocess.Popen(command, **kwargs)
    rss = None
    if hasattr(os, "wait4"):
        _, status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
        rss = getattr(usage, "ru_maxrss", None)
    else:
        process.wait()
    elapsed = (time.perf_counter_ns() - start) / 1e9
    return elapsed, process.returncode, rss
