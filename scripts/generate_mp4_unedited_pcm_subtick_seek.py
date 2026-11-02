"""Own unedited PCM native-clock seeks with independent analytic waveforms."""
from pathlib import Path
import json,hashlib,struct
from generate_mp4_audio_clock_regressions import generate as clocks
from generate_mp4_uniform_plan_scan import generate as compact
from generate_mp4_audio_reconfiguration import generate as changes
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-unedited-pcm-subtick-seek'
def generate():
    source=clocks();small=compact();files={'reference-0.f32le':source['expected.f32le'],'reference-1.f32le':struct.pack('<64f',*[0.125]*64)};cases=[]
    def add(name,data,rate,count,control,scale):
        files[name]=data;end=count*10**9//rate;queries=[]
        for ns in sorted(set([0,1,20833,20834,124999,125000,125001,19999999,20000000,20000001,39999999,40000001,end-1,end,end+1,18446744073709551615999999999])):
            first=min(count,(ns*rate+10**9-1)//10**9);parts=[]if first==count else[dict(control=control,rate=rate,channels=1,first=first,last=count,start_ns=0)];queries.append(dict(target_ns=str(ns),parts=parts,anchor_ns=first*10**9//rate))
        cases.append(dict(file=name,queries=queries,duration_ns=end,scale=scale,video_frames=3))
    for scale in [8000,48000,90000,1000]:
        name=f'pcm-{scale}.mp4';add(name,source[name],8000,480,0,scale)
    for source_name in ['frames-64.mp4','control-alias.mp4','control-endian.mp4']:
        add(source_name,small[source_name],48000,64,1,48000)
    changing=changes(pcm_only=True)
    for i in range(6):files[f'reference-{i+2}.f32le']=changing[f'pcm-{i}.f32le']
    queries=[]
    for ns in [0,1,20833,20834,19999999,20000000,20000001,39999999,40000001,59999999,60000001,79999999,80000001,99999999,100000001,119999999,120000000,18446744073709551615999999999]:
        parts=[]
        for i,(rate,ch) in enumerate([(48000,2),(32000,1),(44100,2)]*2):
            count=rate//50;first=min(count,max(0,((ns-i*20000000)*rate+10**9-1)//10**9))
            if first<count:parts.append(dict(control=i+2,rate=rate,channels=ch,first=first,last=count,start_ns=i*20000000))
        anchor=parts[0]['start_ns']+parts[0]['first']*10**9//parts[0]['rate']if parts else 120000000
        queries.append(dict(target_ns=str(ns),parts=parts,anchor_ns=anchor))
    for stem in ['pcm','pcm-fragmented','pcm-start-third','pcm-start-third-fragmented']:
        name='changing-'+stem+'.mp4';files[name]=changing[stem+'.mp4'];cases.append(dict(file=name,queries=queries,duration_ns=120000000,scale=1000000,video_frames=6))
    files['manifest.json']=(json.dumps(dict(provenance='Own8kHz mono integer ramp and48kHz constant0.125 PCM,compact native sample indexes and alias/endian descriptions,three neutral VP9 pictures each. No edit lists,private media,FFmpeg or network. Rational native ceil suffix oracle;additional classic/fragmented/reordered48k stereo16,32k mono float32,44.1k stereo24 changes with independent per-packet ramp references.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
