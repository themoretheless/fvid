"""Owned six-block 8-channel E-AC-3 with active dependent AHT planes."""
from pathlib import Path
from generate_eac3_presentation import generate as presentations
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-expanded-aht'
def generate():return presentations(expanded_aht=True)
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
