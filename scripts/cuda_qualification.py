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
        ("fvid-cuda", "nv12::native_nv12::shader_tests", ["native_nv12_sampling_shader_preserves_pitches_and_matches_cpu", "native_p010_sampling_shader_preserves_pitches_and_matches_cpu", "followed_stream_survives_parameter_changes_and_processor_drop"], []),
        ("fvid-media", "hw_cuda::layout_tests", ["main10_filter_encodes_hevc_without_host_frame_copies", "vertical_reflection_copies_host_frames_only_when_explicitly_requested"], ["--features", "cuda-hw"]),
    ]
    results = []
    for package, selector, required, features in suites:
        result = run(["cargo", "test", "--manifest-path", str(root / "crates" / package / "Cargo.toml"),
                      *features, "--lib", selector, "--", "--ignored", "--test-threads=1"])
        passed = require_executed_tests(result.stdout, len(required), required)
        results.append({"suite": selector, "passed": passed, "required_tests": required})
    return results
