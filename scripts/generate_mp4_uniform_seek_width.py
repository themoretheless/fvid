"""Short authored uniform/expanded MP4 seek-width controls; no tools/network."""
from pathlib import Path
import json
from generate_mp4_audio_clock_regressions import generate as clocks
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-uniform-seek-width'
def generate():
    source=clocks()
    files={f'{codec}-{rate}.mp4':source[f'{codec}-{rate}.mp4'] for codec in ['pcm','alac'] for rate in [1000,48000]}
    def fixed_pcm(data,audio=False):
        out=b''
        for k,b in boxes(data):
            active=audio
            if k==b'trak':active=dict(boxes(dict(boxes(b))[b'mdia']))[b'hdlr'][8:12]==b'soun'
            if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=fixed_pcm(b,active)
            elif active and k==b'stsz':
                count=int.from_bytes(b[8:12],'big');sizes=[int.from_bytes(b[i:i+4],'big')for i in range(12,len(b),4)]
                assert sizes==[320]*count
                b=word(0,320,count)
            out+=atom(k,b)
        return out
    for n in list(files):
        if n.startswith('pcm-'):files[n]=fixed_pcm(files[n])
    files['expected.f32le']=source['expected.f32le']
    files['manifest.json']=(json.dumps(dict(provenance='Authored60ms neutral VP9 and8kHz mono PCM/raw ALAC. Uniform PCM and expanded ALAC controls at1000/48000 media clocks. All media stays short; only seek requests cross32-bit tick boundaries. No private source, encoder or network.',cases=[dict(file=n,uniform=n.startswith('pcm-'),delta=20 if '-1000.' in n else 960)for n in files if n.endswith('.mp4')]),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
