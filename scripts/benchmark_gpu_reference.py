"""FFmpeg command construction for explicitly requested benchmarks only."""
import random
import statistics
import subprocess
import time
from validate_gpu import FORMATS

def ffmpeg_command(ffmpeg, source, chroma, crop, horizontal, vertical, one_thread=False):
    filters = []
    if crop:
        x, y, width, height = crop
        filters.append(f"crop={width}:{height}:{x}:{y}:exact=1")
    filters += (["hflip"] if horizontal else []) + (["vflip"] if vertical else [])
    command = [ffmpeg, "-nostdin", "-v", "error"]
    if one_thread:
        command += ["-filter_threads", "1", "-threads", "1"]
    command += ["-i", str(source), "-an", "-sn"]
    if filters:
        command += ["-vf", ",".join(filters)]
    command += ["-c:v", "rawvideo", "-pix_fmt", FORMATS[chroma][2]]
    if one_thread:
        command += ["-threads", "1"]
    return command + ["-strict", "-1", "-f", "yuv4mpegpipe", "-"]

def benchmark(args, work, active, report):
    from validate_gpu import make_input, file_sha256, options, success, save
    report["benchmarks"] = []
    rng = random.Random(451)
    dimensions = {"720": (1280, 720), "1080": (1920, 1080), "4k": (3840, 2160)}
    # Avoid benchmarking auto and an explicit backend twice on the same device.
    engines = ["cpu"]
    for requested in sorted(active, key=lambda key: key == "auto"):
        actual = active[requested]
        if actual != "cpu" and not any(active[old] == actual for old in engines):
            engines.append(requested)
    for resolution in args.resolutions:
        width, height = dimensions[resolution]
        source = work / f"benchmark-{resolution}.y4m"
        make_input(source, width, height, "420", args.frames)
        input_hash = file_sha256(source)
        for case, crop, horizontal, vertical in [("hflip", None, True, False), ("fused", (width // 8 // 2 * 2, height // 8 // 2 * 2, width // 2, height // 2), True, True)]:
            commands = {engine: [args.binary, str(source), "-", "--backend", engine, "--device", str(args.device if engine != "cpu" else 0)] + options(crop, horizontal, vertical) for engine in engines}
            for name, one_thread in [("ffmpeg_default", False), ("ffmpeg_1thread", True)]:
                commands[name] = ffmpeg_command(args.ffmpeg, source, "420", crop, horizontal, vertical, one_thread)
            timings = {engine: [] for engine in commands}
            diagnostics = {}
            for engine, command in commands.items():
                diagnostics[engine] = success(command, stdout=subprocess.DEVNULL).stderr.decode(errors="replace")
            orders = []
            for _ in range(args.rounds):
                order = list(commands)
                rng.shuffle(order)
                orders.append(order)
                for engine in order:
                    start = time.perf_counter_ns()
                    success(commands[engine], stdout=subprocess.DEVNULL)
                    timings[engine].append((time.perf_counter_ns() - start) / 1e9)
            medians = {engine: statistics.median(samples) for engine, samples in timings.items()}
            record = dict(resolution=f"{width}x{height}", frames=args.frames, case=case, input_sha256=input_hash, commands=commands, diagnostics=diagnostics, seconds=timings, median_seconds=medians, fps={engine: args.frames / seconds for engine, seconds in medians.items()}, round_orders=orders)
            report["benchmarks"].append(record)
            save(args.report, report)
            print(f"Benchmark {record['resolution']} {case}: " + ", ".join(f"{engine}={seconds:.3f}s" for engine, seconds in medians.items()), flush=True)
        source.unlink()


def run_benchmarks(args, work, active, report):
    from validate_gpu import success
    report["ffmpeg"] = success([args.ffmpeg, "-version"], stdout=subprocess.PIPE).stdout.decode().splitlines()[0]
    benchmark(args, work, active, report)
