"""Own class-restructured multi-AU RS groups; offline explicit generation only."""
from pathlib import Path
from generate_aac_ep_concatenated_fixtures import generate
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aac-ep-rs-group-concatenated'
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate(grouped=True).items():(OUT/name).write_bytes(data)
