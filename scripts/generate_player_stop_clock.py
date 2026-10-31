#!/usr/bin/env python3
"""Authored sources for live-clock versus future picture-end stop scheduling."""
from pathlib import Path
import json
from generate_player_stop_bound import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-stop-clock'
def generate():
    files=source()
    manifest=json.loads(files['manifest.json'])
    manifest['provenance']+=' Scheduling witnesses use a controlled real audio worker clock and silent frame deadlines;only authored donor material.'
    files['manifest.json']=(json.dumps(manifest,indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
