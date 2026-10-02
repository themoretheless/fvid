"""FFmpeg command construction for explicitly requested benchmarks only."""
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
