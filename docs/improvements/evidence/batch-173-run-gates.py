import subprocess, os, hashlib, json
from pathlib import Path

root = Path.cwd()
evidence = root / "docs/improvements/evidence"
sources = [
    root / p
    for p in [
        "src/container/mp4.rs",
        "tests/mp4_trun_flags.rs",
        "scripts/generate_mp4_trun_flag_exclusivity_regressions.py",
        "tests/fixtures/playback-errors/mp4-trun-flags/manifest.json",
    ]
]
hashes = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
env = dict(os.environ, CARGO_INCREMENTAL="0", CARGO_TARGET_DIR="target")
base = ["cargo", "test", "--locked", "--offline", "--features", "player"]
checks = [
    ("after", base + ["--test", "mp4_trun_flags", "--", "--nocapture"]),
    (
        "related-after",
        base
        + sum(
            (
                ["--test", n]
                for n in [
                    "mp4_signed_offset",
                    "mp4_fragment_description",
                    "mp4",
                    "mp4_seek_io_regressions",
                    "mp4_enabled_tracks",
                    "hls_parser_regressions",
                    "vp9_chroma",
                    "mp4_audio_clocks",
                    "mp4_pcm_choice",
                ]
            ),
            [],
        ),
    ),
    ("mp4-units", base + ["--lib", "container::mp4::tests"]),
    (
        "core",
        ["cargo", "test", "--locked", "--offline", "--no-default-features"]
        + sum(
            (
                ["--test", n]
                for n in [
                    "mp4_trun_flags",
                    "mp4_signed_offset",
                    "mp4_fragment_description",
                    "mp4",
                    "mp4_seek_io_regressions",
                ]
            ),
            [],
        ),
    ),
    ("player-build", ["cargo", "build", "--locked", "--offline", "--features", "player"]),
    (
        "fixtures-after",
        [
            "python3",
            "-m",
            "unittest",
            "discover",
            "-s",
            "scripts",
            "-p",
            "test_mp4_trun_flag_exclusivity_regressions.py",
        ],
    ),
]
results = []
for name, command in checks:
    log = evidence / f"batch-173-{name}.log"
    with log.open("w") as f:
        r = subprocess.run(command, stdout=f, stderr=subprocess.STDOUT, env=env)
    results.append(
        dict(name=name, command=command, exit_code=r.returncode, log=str(log.relative_to(root)))
    )
    (evidence / "batch-173-gate-results.json").write_text(
        json.dumps(dict(source_hashes=hashes, checks=results), indent=2) + "\n"
    )
    print(name, r.returncode, flush=True)
    assert r.returncode == 0, name
assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest() == h for p, h in hashes.items())
