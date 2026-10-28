"""Own EP3 large independent HCR regions; explicit offline generation."""
from pathlib import Path
from generate_aac_er_predictive_esc_fixtures import generate
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aac-er-esc-large-hcr'
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate(large=True).items():(OUT/name).write_bytes(data)
