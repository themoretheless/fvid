"""Owned expanded MP4 AHT with a nonzero block-varying base 5.1."""
from pathlib import Path
from generate_eac3_mp4_aht import generate as author
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-mp4-aht-sounding-base'
def generate():return author(sounding_base=True)
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
