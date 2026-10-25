"""Short owned playback witnesses; exact encoded controls retain the authored payloads."""
from pathlib import Path
import json
from generate_mp4_quantized_audio_clock import generate as clocks
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-quantized-playback'
def generate():
    source=clocks(playback_controls=True);files={n:b for n,b in source.items()if n.startswith('exact-')}
    files['aac-playback-900.mp4']=source['constant-aac-900-floor.mp4']
    cases=json.loads(source['manifest.json'])['cases']
    for c in cases:c['exact']='exact-'+c['name'].rsplit('-',2)[0]+'.mp4'
    from generate_aac_ssr_fixtures import packet,oracle
    from generate_mp4_aac_transitions import generate as aac
    seq=[1,2,3];windows=[0,1,0]
    control=([packet(i,s,w,2,True)for i,(s,w)in enumerate(zip(seq,windows))],bytes(oracle(windows,2,True,sequences=seq)[0]))
    native=aac([(24000,2,1024,3,6)]*3,[control]*3)
    files['ssr-native-window.mp4']=native['control-0.mp4'];files['ssr-native-window.f32le']=control[1]
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned short neutral video and authored codec packets; exact controls preserve payload bytes before timestamp quantization. No private media, external codec, process, network or FFmpeg.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
