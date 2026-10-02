#!/usr/bin/env python3
"""Opt-in CUDA CLI/FFmpeg reference comparisons; never imported by default validation."""
import json
import pathlib
import tempfile
from common import ROOT


def run_reference(run, binary, report, checks):
    with tempfile.TemporaryDirectory(prefix="fvid-hw-") as temp:
        root = pathlib.Path(temp)
        source = root / "src.mp4"
        output = root / "out.mp4"
        run([
            "ffmpeg", "-nostdin", "-v", "error",
            "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25",
            "-t", "1", "-an", "-c:v", "libx264", "-pix_fmt", "yuv420p", str(source),
        ])
        result = run([
            str(binary), "media", "hw-filter", str(source), str(output),
            "--crop", "16:16:320:180", "--hflip", "--vflip",
        ])
        fused_stats = json.loads(result.stdout)
        assert fused_stats["backend"] == "cuda-nvdec-nvenc"
        assert fused_stats["host_frame_copies"] == 0
        assert fused_stats["device_filter_passes"] == 25
        assert fused_stats["video_frames"] == 25
        assert fused_stats["width"] == 320 and fused_stats["height"] == 180
        run(["ffmpeg", "-nostdin", "-v", "error", "-i", str(output), "-f", "null", "-"])
        direct = root / "direct-crop.mp4"
        reference = root / "ffmpeg-crop.mp4"
        crop_stats = json.loads(run([
            str(binary), "media", "hw-filter", str(source), str(direct),
            "--crop", "16:16:320:180",
        ]).stdout)
        run([
            "ffmpeg", "-nostdin", "-y", "-v", "error",
            "-hwaccel", "cuda", "-hwaccel_output_format", "cuda", "-i", str(source),
            "-vf", "hwdownload,format=nv12,crop=320:180:16:16,hwupload_cuda",
            "-c:v", "h264_nvenc", "-preset", "p1", "-bf", "0", "-an", str(reference),
        ])
        def frame_md5(path):
            return run([
                "ffmpeg", "-nostdin", "-v", "error", "-i", str(path),
                "-map", "0:v:0", "-f", "framemd5", "-",
            ]).stdout
        assert crop_stats["host_frame_copies"] == 0
        assert crop_stats["device_filter_passes"] == 25
        assert frame_md5(direct) == frame_md5(reference)
        checks.append("crop decoded frames match FFmpeg")
        print("direct crop matches FFmpeg decoded frames:", crop_stats)
        operation_specs = {
            "copy": ([], None),
            "hflip": (["--hflip"], "hwdownload,format=nv12,hflip,hwupload_cuda"),
            "vflip": (["--vflip"], "hwdownload,format=nv12,vflip,hwupload_cuda"),
        }
        for name, (fvid_flags, vf) in operation_specs.items():
            actual = root / f"{name}.mp4"
            expected = root / f"ffmpeg-{name}.mp4"
            stats = json.loads(run([
                str(binary), "media", "hw-filter", str(source), str(actual), *fvid_flags,
            ]).stdout)
            command = [
                "ffmpeg", "-nostdin", "-y", "-v", "error",
                "-hwaccel", "cuda", "-hwaccel_output_format", "cuda", "-i", str(source),
            ]
            if vf:
                command += ["-vf", vf]
            run(command + [
                "-c:v", "h264_nvenc", "-preset", "p1", "-bf", "0", "-an", str(expected),
            ])
            assert frame_md5(actual) == frame_md5(expected)
            assert stats["video_frames"] == 25
            assert stats["host_frame_copies"] == 0
            checks.append(f"{name} decoded frames match FFmpeg")
        fused_reference = root / "ffmpeg-fused.mp4"
        run([
            "ffmpeg", "-nostdin", "-y", "-v", "error",
            "-hwaccel", "cuda", "-hwaccel_output_format", "cuda", "-i", str(source),
            "-vf", "hwdownload,format=nv12,crop=320:180:16:16,hflip,vflip,hwupload_cuda",
            "-c:v", "h264_nvenc", "-preset", "p1", "-bf", "0", "-an", str(fused_reference),
        ])
        assert frame_md5(output) == frame_md5(fused_reference)
        checks.append("fused decoded frames match FFmpeg")
        interval = root / "interval.mp4"
        interval_reference = root / "ffmpeg-interval.mp4"
        interval_stats = json.loads(run([
            str(binary), "media", "hw-filter", str(source), str(interval),
            "--from", "0.2", "--to", "0.6",
        ]).stdout)
        run([
            "ffmpeg", "-nostdin", "-y", "-v", "error",
            "-hwaccel", "cuda", "-hwaccel_output_format", "cuda",
            "-ss", "0.2", "-to", "0.6", "-i", str(source),
            "-c:v", "h264_nvenc", "-preset", "p1", "-bf", "0", "-an",
            str(interval_reference),
        ])
        interval_probe = json.loads(run([
            "ffprobe", "-v", "error", "-count_frames", "-select_streams", "v:0",
            "-show_entries", "stream=nb_read_frames:format=duration", "-of", "json",
            str(interval),
        ]).stdout)
        assert interval_stats["video_frames"] == 10
        assert interval_probe["streams"][0]["nb_read_frames"] == "10"
        assert abs(float(interval_probe["format"]["duration"]) - 0.4) < 0.001
        assert frame_md5(interval) == frame_md5(interval_reference)
        checks.append("cut interval timeline and decoded frames match FFmpeg")
        print("CUDA interval timeline matches FFmpeg:", interval_stats)
        print("hw-filter fused smoke passed:", fused_stats)
        decode_stats = json.loads(run([
            str(binary), "media", "decode", str(source), "--device", "0",
        ]).stdout)
        run([
            "ffmpeg", "-nostdin", "-v", "error", "-hwaccel", "cuda",
            "-hwaccel_output_format", "cuda", "-i", str(source), "-an", "-f", "null", "-",
        ])
        assert decode_stats["video_frames"] == 25
        checks.append("decode-only frame count matches FFmpeg")
        main10 = ROOT / "tests/fixtures/hevc/main10-ipb.mp4"
        main10_output = root / "main10-shader.mkv"
        shader_stats = json.loads(run([
            str(binary), "media", "hw-filter", str(main10), str(main10_output),
            "--crop", "0:0:64:64", "--hflip", "--vflip",
            "--cuda-sampling-shader", str(ROOT / "shaders/boxblur.cu"),
        ]).stdout)
        def video_probe(path):
            return json.loads(run([
                "ffprobe", "-v", "error", "-count_frames", "-select_streams", "v:0",
                "-show_entries", "stream=codec_name,pix_fmt,width,height,nb_read_frames",
                "-of", "json", str(path),
            ]).stdout)["streams"][0]
        source_probe = video_probe(main10)
        shader_probe = video_probe(main10_output)
        assert shader_stats["encoder"] == "hevc_nvenc"
        assert shader_stats["filter"] == "cuda-sampling-shader"
        assert shader_stats["host_frame_copies"] == 0
        assert shader_stats["video_frames"] == int(source_probe["nb_read_frames"])
        assert shader_stats["device_filter_passes"] == shader_stats["video_frames"]
        assert shader_probe["codec_name"] == "hevc"
        assert shader_probe["pix_fmt"] == "yuv420p10le"
        assert (shader_probe["width"], shader_probe["height"]) == (64, 64)
        assert shader_probe["nb_read_frames"] == source_probe["nb_read_frames"]
        report["main10_cli"] = {"stats": shader_stats, "probe": shader_probe}
        checks.append("provided CLI binary executes P010 sampling shader and encodes decodable HEVC Main10")
