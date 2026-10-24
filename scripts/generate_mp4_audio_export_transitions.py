"""Owned fixed-output-geometry AAC setup changes, no external codecs."""
from pathlib import Path
from generate_mp4_aac_transitions import generate as aac
from generate_mp4_variable_rate_edits import edit_movie

def generate():
    source=aac([(48000,2,1024,2,3),(48000,2,960,1,3),(48000,2,1024,1,3)])
    files={name:source[name] for name in ['changes.mp4','fragmented.mp4','third.mp4','equivalent.mp4']}
    pcm=b''.join(source[f'control-{i}.f32le'] for i in [0,1,2,0])
    files['expected.f32le']=pcm
    # Repeated source range crosses two configuration boundaries; silence is explicit.
    stride=8
    edited=bytes(240*stride)+pcm[960*stride:7200*stride]+bytes(96*stride)+pcm[960*stride:7200*stride]
    files['edited.f32le']=edited
    for stem in ['changes','fragmented','third','equivalent']:
        files[stem+'-edits.mp4']=edit_movie(files[stem+'.mp4'],[(5,-1),(130,14112000//50),(2,-1),(130,14112000//50)])
    files['later-edits.mp4']=edit_movie(files['third.mp4'],[(5,-1),(80,14112000*70//1000),(2,-1),(80,14112000*70//1000)])
    files['later-edited.f32le']=bytes(240*8)+pcm[3360*8:7200*8]+bytes(96*8)+pcm[3360*8:7200*8]
    from generate_aac_ssr_fixtures import packet as ssr_packet,oracle as ssr_oracle
    from generate_mp4_aac_transitions import packet,oracle
    seq=[1,2,3];shapes=[0,1,0]
    controls=[([ssr_packet(i,s,shape,2,True) for i,(s,shape) in enumerate(zip(seq,shapes))],bytes(ssr_oracle(shapes,2,True,sequences=seq)[0]))]
    for _ in range(2):
        specs=[[(150+f*3+c,(f+c)%4,4+f+c) for c in range(2)]for f in range(3)]
        controls.append(([packet(s) for s in specs],oracle(1024,specs)))
    delayed=aac([(24000,2,1024,3,6),(24000,2,1024,2,6),(24000,2,1024,1,6)],controls)
    for stem in ['changes','fragmented','third']:
        files['ssr-'+stem+'.mp4']=delayed[stem+'.mp4']
    files['ssr-expected.f32le']=b''.join(controls[i][1] for i in [0,1,2,0])
    files['ssr-edits.mp4']=edit_movie(files['ssr-third.mp4'],[(5,-1),(160,14112000*70//1000),(2,-1),(160,14112000*70//1000)])
    pcm=files['ssr-expected.f32le']
    files['ssr-edited.f32le']=bytes(120*8)+pcm[1680*8:5520*8]+bytes(48*8)+pcm[1680*8:5520*8]
    from generate_mp4_audio_reconfiguration import generate as pcm
    source=pcm([(b'sowt',48000,2,16),(b'fl32',48000,2,32),(b'in24',48000,2,24)],True)
    for name in ['pcm','pcm-fragmented','pcm-start-third','pcm-start-third-fragmented']:
        files[name+'.mp4']=source[name+'.mp4']
    files['pcm-expected.f32le']=b''.join(source[f'pcm-{i}.f32le'] for i in range(6))
    return files
if __name__=='__main__':
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-audio-export-transitions'
    out.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(out/name).write_bytes(data)
