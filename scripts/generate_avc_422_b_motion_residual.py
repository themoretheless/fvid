"""Own moving-reference 422 B residual matrix, both CAVLC and CABAC."""
from pathlib import Path
from generate_avc_422_b_residual import generate as cavlc
from generate_avc_422_b_residual_cabac import generate as cabac
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def generate():
    base=ROOT/'avc-422-b-motion'
    scalar=ROOT/'avc-422-b-motion-residual'
    cavlc(base,scalar)
    cabac(scalar,base,ROOT/'avc-422-b-motion-residual-cabac')
if __name__=='__main__': generate()
