"""Authored AVC+AAC files: excluded coarse audio must not gate video concat."""
from pathlib import Path
import hashlib,json
from generate_mp4_audio_selection_regressions import boxes,build_aac_pulse_packet,build_esds,make_audio_track,mp4a_v0_header,synthesize_analytical_pcm
from generate_vp9_chroma import atom
from generate_mp4_quantized_audio_clock import quantize
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/mp4-concat-unselected-plan'
def generate():
    stem=ROOT/'tests/fixtures/playback-errors/avc-444-pcm-deblock/y8-c8-o12-12-m1-e0'
    parts=dict(boxes(stem.with_suffix('.mp4').read_bytes()));ftyp=atom(b'ftyp',parts[b'ftyp']);mdat=atom(b'mdat',parts[b'mdat']);payloads=[build_aac_pulse_packet(100,1,8)]*3
    entry=mp4a_v0_header(1,48000)+atom(b'esds',build_esds(b'\x11\x88'))
    track=make_audio_track(2,3,b'mp4a',entry,payloads,len(ftyp)+len(mdat)+8)
    base=b''.join(atom(k,b[:-4]+(3).to_bytes(4,'big')if k==b'mvhd' else b)for k,b in boxes(parts[b'moov']))
    exact=ftyp+mdat+atom(b'mdat',b''.join(payloads))+atom(b'moov',base+track)
    files={'exact.mp4':exact,'audio.f32le':synthesize_analytical_pcm([(100,1,8)]*3),'video.yuv16':stem.with_suffix('.y16').read_bytes(),'video.packet':stem.with_suffix('.packet').read_bytes()}
    for scale in [1000,1001]:files[f'coarse-{scale}.mp4']=quantize(exact,scale,'floor')
    files['manifest.json']=(json.dumps(dict(provenance='FVid-authored single40ms AVC PCM picture with independent scalar YUV oracle and three AAC pulse frames with analytic mono48kHz PCM. AAC media clock is exact48000Hz or cumulatively floor-quantized1000/1001Hz. Video and AAC payloads remain identical. No private media,FFmpeg,encoder or network.',cases=['exact.mp4','coarse-1000.mp4','coarse-1001.mp4'],sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
