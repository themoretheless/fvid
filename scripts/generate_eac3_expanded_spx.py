"""Owned expanded spectral-extension/noise/DRC assembly witnesses."""
from pathlib import Path
from generate_eac3_mp4_aht import generate as author
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-expanded-spx'
def generate():return author(coupled='spx')
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
