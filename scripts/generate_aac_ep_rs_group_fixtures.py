"""Own EP2 grouped RS fixtures; explicit generation, no external codecs/network."""
from pathlib import Path
from generate_aac_ep_multiclass_fixtures import generate
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aac-ep-rs-groups'
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate(rs_groups=True).items():(OUT/name).write_bytes(data)
