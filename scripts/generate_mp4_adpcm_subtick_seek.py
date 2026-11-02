"""Own IMA4/IMA-WAV/MS-ADPCM native seek windows and predictor refusal."""
from pathlib import Path
from fractions import Fraction as F
import json,hashlib,struct
from generate_mp4_adpcm_transitions import generate as transitions
from generate_mp4_audio_packet_seek import movie,PREDICTORS
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-adpcm-subtick-seek'
def generate():
    source=transitions();manifest=json.loads(source['manifest.json']);files={};cases=[];references={}
    def ref(data):
        key=hashlib.sha256(data).hexdigest()
        if key not in references:
            identity=len(references);references[key]=identity;files[f'reference-{identity}.f32le']=data
        return references[key]
    names={'control-0','control-1','control-2','regular-2000','regular-48000','regular-96000','fragmented-48000','third-48000','trim-48000'}
    names.update(f'{variant}-{scale}-edits'for variant in ['regular','fragmented','third']for scale in [2000,48000])
    def add(name,data,parts,duration,scale):
        visible=[];targets={0,1,125001,18446744073709551615999999999};end=int(duration*10**9)
        for p in parts:
            rate,ch=p['rate'],p['channels'];first,last=p['first'],p['last'];count=last-first;start=F(*p['pts']);assert (start*10**9).denominator==1;start_ns=int(start*10**9)
            raw=bytes(count*ch*4)if p['oracle']is None else source[p['oracle']][first*ch*4:last*ch*4];control=ref(raw);visible.append((control,rate,ch,count,start_ns))
            for point in [start,start+F(count,2*rate),start+F(count,rate)]:
                ns=int(point*10**9);targets.update([max(0,ns-1),ns,ns+1])
        targets.update([end-1,end,end+1]);queries=[]
        for ns in sorted(targets):
            remaining=[]
            for control,rate,ch,count,start_ns in visible:
                first=min(count,max(0,(F(ns-start_ns,10**9)*rate).__ceil__()))
                if first<count:remaining.append(dict(control=control,rate=rate,channels=ch,first=first,last=count,start_ns=start_ns))
            anchor=remaining[0]['start_ns']+remaining[0]['first']*10**9//remaining[0]['rate']if remaining else end
            queries.append(dict(target_ns=str(ns),parts=remaining,anchor_ns=anchor))
        files[name]=data;cases.append(dict(file=name,queries=queries,duration_ns=end,scale=scale,video_frames=6))
    for c in manifest['cases']:
        if c['name']in names:add(c['name']+'.mp4',source[c['name']+'.mp4'],c['parts'],F(*c['duration']),c['scale'])
    raw=b''.join(struct.pack('<f',(PREDICTORS[i%3]+n)/32768)for i in range(6)for n in range(1,65));source['simple.f32le']=raw
    add('ima4-native-8000.mp4',movie(6),[dict(oracle='simple.f32le',rate=8000,channels=1,first=0,last=384,pts=[0,1])],F(384,8000),8000)
    # A complete MS ADPCM block with an invalid predictor. Packet size and all
    # container tables remain valid; active seeking must not turn it into EOF.
    top=dict(atoms(source['control-2.mp4']));payload=bytearray(top[b'mdat']);payload[-15]=7;files['invalid-ms-predictor.mp4']=atom(b'ftyp',top[b'ftyp'])+atom(b'mdat',payload)+atom(b'moov',top[b'moov'])
    files['manifest.json']=(json.dumps(dict(provenance='Only authored IMA4 ramps,IMA WAV stereo and MS ADPCM mono predictor plateaus;grouped blocks,three clocks,classic/fragmented/reordered/trimmed and explicit silence/repeated edits with independent exact PCM references. Invalid active MS predictor control is separate. No private input,FFmpeg or network.',cases=cases,refusal_file='invalid-ms-predictor.mp4',sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
