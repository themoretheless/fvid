"""Owned nonzero AAC with legal FIL padding; offline fixture generation only."""
import json
from pathlib import Path
from generate_mp4_aac_transitions import individual, oracle, generate, packet
from generate_mp4_audio_selection_regressions import pack_bits
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/adts-packet-working-set'

def main():
    specs = [[(150+f*3+c, (f+c)%4, 4+f+c) for c in range(2)] for f in range(3)]
    raw = [pack_bits('00100000' + ''.join(individual(*s) for s in frame)
                    + ('1101111' + f'{241:08b}' + '0'*(255*8))*4 + '111') for frame in specs]
    data = bytearray()
    for payload in raw:
        n = len(payload)+7
        data += bytes([255,241,76,128 | (n>>11), (n>>3)&255, ((n&7)<<5)|31,252])+payload
    pcm = oracle(1024, specs)
    # Reuse the established synthetic movie builder; save only this control.
    mono = [[(150+f*3, f%4, 4+f)] for f in range(3)]
    files = generate(controls=[(raw, pcm),
        ([packet(s) for s in mono], oracle(960, mono)),
        ([packet(s) for s in specs], pcm)])
    OUT.mkdir(parents=True,exist_ok=True)
    for name,body in [('padded.aac',data),('padded.mp4',files['control-0.mp4']),('oracle.f32le',pcm)]:
        (OUT/name).write_bytes(body)
    (OUT/'manifest.json').write_text(json.dumps(dict(frames=3,rate=48000,channels=2,
        payload_bytes=list(map(len,raw)),provenance='Owned CPE pulse spectra, four legal zero FIL payloads per frame, analytic IMDCT oracle and synthetic VP9 companion. No FFmpeg, network or private media.'),indent=2)+'\n')
if __name__=='__main__':main()
