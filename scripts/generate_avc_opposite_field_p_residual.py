"""Own opposite-parity P residual matrices over scalar compact-field prediction."""
from pathlib import Path
from generate_avc_mono_field_p_residual import generate as mono
from generate_avc_422_field_p_residual import generate as wide
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def generate():
    mono(ROOT/'avc-mono-field-p-opposite',ROOT/'avc-mono-field-p-opposite-residual')
    wide(ROOT/'avc-422-field-p-opposite',ROOT/'avc-422-field-p-opposite-residual')
if __name__=='__main__':generate()
