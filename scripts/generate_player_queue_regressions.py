#!/usr/bin/env python3
"""Generate synthetic fixtures for player queue, playlist, and skin regressions."""

from __future__ import annotations

import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURE_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "player-queue"


def build_pls() -> bytes:
    return (
        "[playlist]\r\n"
        "NumberOfEntries=2\r\n"
        "File1=\"clip.avi\"\r\n"
        "Title1=Synthetic AVI\r\n"
        "File2=clip.wv\r\n"
        "Title2=Synthetic WavPack\r\n"
        "Version=2\r\n"
    ).encode("utf-8")


def build_m3u() -> bytes:
    return (
        "#EXTM3U\n"
        "#EXTINF:1,Synthetic AVI\n"
        "\"clip.avi\"\n"
        "#EXTINF:1,Synthetic WavPack\n"
        "clip.wv\n"
    ).encode("utf-8")


def generate(out_dir: pathlib.Path = FIXTURE_DIR) -> dict[str, pathlib.Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "queue.pls": out_dir / "queue.pls",
        "queue.m3u": out_dir / "queue.m3u",
        "clip.avi": out_dir / "clip.avi",
        "clip.wv": out_dir / "clip.wv",
        "companion.wvc": out_dir / "companion.wvc",
    }
    files["queue.pls"].write_bytes(build_pls())
    files["queue.m3u"].write_bytes(build_m3u())
    files["clip.avi"].write_bytes(b"RIFF\x04\x00\x00\x00AVI ")
    files["clip.wv"].write_bytes(b"wvpk")
    files["companion.wvc"].write_bytes(b"wvpk")
    return files


if __name__ == "__main__":
    generate()
