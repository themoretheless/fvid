"""Mandatory physical CUDA tests used by the existing hardware validator."""
import re


def require_executed_tests(stdout, minimum, required=()):
    """Cargo exit zero is insufficient when platform cfg removes all tests."""
    summaries = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", stdout)
    passed = sum(int(item[0]) for item in summaries)
    failed = sum(int(item[1]) for item in summaries)
    if failed or passed < minimum:
        raise RuntimeError(f"CUDA qualification executed {passed} passing tests; requires at least {minimum}")
    for name in required:
        if not re.search(r"^test \S*" + re.escape(name) + r" \.\.\. ok$", stdout, re.MULTILINE):
            raise RuntimeError(f"CUDA qualification did not execute required test {name}")
    return passed


def qualify_native_tests(root, run):
    suites = [
        ("fvid-cuda", "pipeline::tests", ["resident_cuda_shaders_fuse_geometry_and_keep_one_host_roundtrip", "sampling_shader_matches_independent_crop_and_reflection_reference"], []),
        ("fvid-cuda", "nv12_buffer::tests", [
            "device_black_fill_has_correct_luma_chroma_and_padding",
        ], []),
        ("fvid-cuda", "nv12::native_nv12::shader_tests", ["native_nv12_sampling_shader_preserves_pitches_and_matches_cpu", "native_p010_sampling_shader_preserves_pitches_and_matches_cpu", "followed_stream_survives_parameter_changes_and_processor_drop"], []),
        ("fvid-media", "owned_nvdec_avc", [
            "synthetic_owned_avc_picture_decodes_and_maps_on_nvidia",
            "synthetic_ipb_packets_decode_map_and_release_without_libav",
            "synthetic_avc_decode_filter_encode_chain_without_libav",
        ], ["--no-default-features", "--features", "native-cuda"]),
        ("fvid-media", "owned_nvdec_hevc", [
            "synthetic_owned_hevc_idr_submits_and_maps_on_nvidia",
            "owned_hevc_ipb_scheduler_submits_and_maps_on_nvidia",
        ], ["--no-default-features", "--features", "native-cuda"]),
        ("fvid-media", "owned_nvdec_mp4", [
            "own_mp4_packets_decode_on_nvidia_without_libav",
        ], ["--no-default-features", "--features", "native-cuda"]),
        ("fvid-media", "owned_hw_filter", [
            "production_hw_filter_routes_synthetic_avc_without_libav",
            "production_hw_filter_routes_sps_crop_without_libav",
        ], ["--no-default-features", "--features", "native-cuda"]),
        ("fvid-media", "owned_nvenc_movie", [
            "synthetic_movie_encodes_and_muxes_without_libav",
        ], ["--no-default-features", "--features", "native-cuda"]),
        ("fvid-media", "owned_nvdec_movie", [
            "synthetic_movie_blanks_and_repeated_ranges_present_on_nvidia",
        ], ["--no-default-features", "--features", "native-cuda"]),
        ("fvid-media", "hw_cuda::layout_tests", ["main10_filter_encodes_hevc_without_host_frame_copies", "vertical_reflection_copies_host_frames_only_when_explicitly_requested"], ["--features", "cuda-hw"]),
    ]
    results = []
    for package, selector, required, features in suites:
        result = run(["cargo", "test", "--manifest-path", str(root / "crates" / package / "Cargo.toml"),
                      *features, "--lib", selector, "--", "--ignored", "--test-threads=1"])
        passed = require_executed_tests(result.stdout, len(required), required)
        results.append({"suite": selector, "passed": passed, "required_tests": required})
    return results


def require_cli_reference_report(report, binary_sha256):
    """The performance gate needs CLI/reference evidence, not Cargo-only success.

    Historical reports predate scope flags and are accepted only with all seven
    named comparisons, preserving their existing validation evidence.
    """
    required = {
        "crop decoded frames match FFmpeg",
        "copy decoded frames match FFmpeg",
        "hflip decoded frames match FFmpeg",
        "vflip decoded frames match FFmpeg",
        "fused decoded frames match FFmpeg",
        "cut interval timeline and decoded frames match FFmpeg",
        "decode-only frame count matches FFmpeg",
    }
    if report.get("status") != "passed" or not required.issubset(report.get("checks", [])):
        raise RuntimeError("CUDA CLI reference report is incomplete; rerun validate_hw_cuda.py --benchmark-reference")
    flags = ("reference_requested", "reference_completed", "provided_cli_checks_completed")
    if any(flag in report for flag in flags) and not all(report.get(flag) is True for flag in flags):
        raise RuntimeError("CUDA report does not prove completed CLI reference checks; rerun validate_hw_cuda.py --benchmark-reference")
    if report.get("binary_sha256") != binary_sha256:
        raise RuntimeError("CUDA validation report does not match --binary; rerun validate_hw_cuda.py --benchmark-reference")
