"""Owned neutral VP9 plus native PCM/AAC transitions; analytic grid/DSP oracles."""
from pathlib import Path
from fractions import Fraction as F
import math,struct,json
from generate_mp4_audio_reconfiguration import generate as pcm
from generate_mp4_aac_transitions import generate as aac
from generate_mp4_variable_rate_edits import edit_movie
SCALE=14112000
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-geometry-normalization'
def ceil(x):return -(-x.numerator//x.denominator)
def generate():
    files={};cases=[]
    counts=[997,641,1001,577,479,853];rates=[48000,32000,44100];values=[(.125,-.25),(.375,),(.5,-.125)]
    data=pcm(pcm_only=True,pcm_frames=counts,pcm_values=values)
    runs=[];at=F(0)
    for i,n in enumerate(counts):
        identity=i%3;end=at+F(n,rates[identity]);runs.append((at,end,values[identity]));at=end
    def dc_oracle(edits,source_runs=None,width=32):
        out=bytearray();start=F(0)
        source_runs=source_runs or runs
        for duration,media in edits:
            end=start+duration
            for k in range(ceil(start*48000),ceil(end*48000)):
                if media is None:v=(0.,0.)
                else:
                    t=media+F(k,48000)-start
                    v=next(v for a,b,v in source_runs if a<=t<b)
                    if len(v)==1:v=v*2
                out+=struct.pack('<dd' if width==64 else '<ff',*v)
            start=end
        return bytes(out)
    edits=[(F(1,3000),None),(F(180,3000),F(17,SCALE)),(F(2,3000),None),(F(140,3000),F(SCALE*30//1000+23,SCALE))]
    for stem in ['pcm','pcm-fragmented','pcm-start-third','pcm-start-third-fragmented']:
        files[stem+'.mp4']=data[stem+'.mp4']
        cases.append(dict(name=stem,oracle='pcm.f32le',rate=48000,channels=2,frames=ceil(at*48000),precise=True))
        files[stem+'-edits.mp4']=edit_movie(data[stem+'.mp4'],[(1,-1),(180,17),(2,-1),(140,SCALE*30//1000+23)],clock=3000)
        cases.append(dict(name=stem+'-edits',oracle='pcm-edits.f32le',rate=48000,channels=2,frames=len(dc_oracle(edits))//8,precise=True))
    files['pcm.f32le']=dc_oracle([(at,F(0))]);files['pcm-edits.f32le']=dc_oracle(edits)
    from generate_mp4_fragment_description_regressions import atoms
    from generate_vp9_chroma import atom,word
    def patch_audio(body,kind,audio=False):
        out=b''
        for tag,payload in atoms(body):
            if tag==b'trak':payload=patch_audio(payload,kind,b'soun' in payload)
            elif tag in [b'moov',b'mdia',b'minf',b'stbl']:payload=patch_audio(payload,kind,audio)
            elif audio and kind=='trim' and tag==b'stts':payload=payload[:-4]+word(int.from_bytes(payload[-4:],'big')-13*(SCALE//44100))
            elif audio and kind=='trim' and tag==b'mdhd':payload=bytearray(payload);payload[16:20]=word(int.from_bytes(payload[16:20],'big')-13*(SCALE//44100))
            if audio and kind=='negative' and tag==b'stbl':payload+=atom(b'ctts',word(1<<24,1,6)+struct.pack('>i',-27))
            out+=atom(tag,payload)
        return out
    shift=F(27,SCALE);negative_end=at-shift
    files['pcm-negative.mp4']=edit_movie(patch_audio(data['pcm.mp4'],'negative'),[(int(negative_end*SCALE),0)],clock=SCALE)
    files['pcm-negative.f32le']=dc_oracle([(negative_end,F(0))],[(a-shift,b-shift,v)for a,b,v in runs])
    files['pcm-trim.mp4']=patch_audio(data['pcm.mp4'],'trim');trim_end=at-F(13,44100)
    files['pcm-trim.f32le']=dc_oracle([(trim_end,F(0))],runs[:-1]+[(runs[-1][0],trim_end,runs[-1][2])])
    for name in ['pcm-negative','pcm-trim']:cases.append(dict(name=name,oracle=name+'.f32le',rate=48000,channels=2,frames=len(files[name+'.f32le'])//8,precise=True))
    precise_values=[(.125+2**-30,-.25-2**-31),(.375+2**-32,),(.5-2**-30,-.125+2**-33)]
    precise_runs=[];cursor=F(0)
    for i,n in enumerate(counts):
        end=cursor+F(n,rates[i%3]);precise_runs.append((cursor,end,precise_values[i%3]));cursor=end
    precise_data=pcm([(b'fl64',48000,2,64),(b'fl64',32000,1,64),(b'fl64',44100,2,64)],True,counts,precise_values)
    for suffix,source_name,edited in [('', 'pcm',False),('-edits','pcm',True),('-fragmented','pcm-fragmented',False),('-third','pcm-start-third',False)]:
        name='pcm64'+suffix
        files[name+'.mp4']=edit_movie(precise_data[source_name+'.mp4'],[(1,-1),(180,17),(2,-1),(140,SCALE*30//1000+23)],clock=3000) if edited else precise_data[source_name+'.mp4']
        oracle_name='pcm64-edits' if edited else 'pcm64'
        selected_edits=edits if edited else [(cursor,F(0))]
        files[oracle_name+'.f32le']=dc_oracle(selected_edits,precise_runs)
        files[oracle_name+'.f64le']=dc_oracle(selected_edits,precise_runs,64)
        cases.append(dict(name=name,oracle=oracle_name+'.f32le',oracle64=oracle_name+'.f64le',rate=48000,channels=2,frames=len(files[oracle_name+'.f32le'])//8,precise=True))
    # AAC controls use separately authored spectral pulse/IMDCT oracles. Evaluate
    # the filter as literal convolution at exact global output-grid instants,
    # independently of production's bounded streaming queue and phase state.
    source=aac();groups=[]
    for identity,(rate,ch) in enumerate([(48000,2),(32000,1),(44100,2)]):
        raw=source[f'control-{identity}.f32le'];samples=list(struct.iter_unpack('<'+'f'*ch,raw));groups.append((rate,ch,samples))
    aruns=[];at=F(0)
    for identity in [0,1,2,0]:
        rate,ch,samples=groups[identity];end=at+F(len(samples),rate);aruns.append((at,end,rate,ch,samples));at=end
    def pulse_oracle(edits,output_rate=48000,output_channels=2):
        out=bytearray();start=F(0)
        for duration,media in edits:
            end=start+duration
            for k in range(ceil(start*output_rate),ceil(end*output_rate)):
                if media is None:result=[0.]*output_channels
                else:
                    t=media+F(k,output_rate)-start
                    a,b,rate,ch,samples=next(r for r in aruns if r[0]<=t<r[1])
                    position=(t-a)*rate;center=position.numerator//position.denominator;fraction=float(position-center)
                    if rate==output_rate and fraction==0:result=list(samples[center])
                    else:
                        radius=math.ceil(32/min(1.,output_rate/rate));cutoff=.94*min(1.,output_rate/rate);weights=[]
                        for j in range(center-radius,center+radius+1):
                            distance=float(j-center)-fraction
                            if abs(distance)>=radius:continue
                            w=cutoff*(math.sin(math.pi*cutoff*distance)/(math.pi*cutoff*distance) if abs(distance)>1e-12 else 1.)*(.42+.5*math.cos(math.pi*distance/radius)+.08*math.cos(2*math.pi*distance/radius))
                            weights.append((min(max(j,0),len(samples)-1),w))
                        norm=sum(w for _,w in weights);result=[sum(samples[j][c]*w for j,w in weights)/norm for c in range(ch)]
                    if output_channels==7 and ch==1:result=[0.,0.,result[0],0.,0.,0.,0.]
                    else:
                        if ch==1:result*=2
                        elif ch==4:
                            k=struct.unpack('<f',struct.pack('<f',math.sqrt(.5)))[0]
                            result=[result[0]*k+result[2],result[1]*k+result[3]]
                        if output_channels==7:result+= [0.]*5
                out+=struct.pack('<'+'f'*output_channels,*result)
            start=end
        return bytes(out)
    aedits=[(F(1,3000),None),(F(700,3000),F(SCALE//50,SCALE)),(F(2,3000),None),(F(700,3000),F(SCALE//50,SCALE))]
    files['aac.f32le']=pulse_oracle([(at,F(0))]);files['aac-edits.f32le']=pulse_oracle(aedits)
    for stem in ['changes','fragmented','third','equivalent']:
        files['aac-'+stem+'.mp4']=source[stem+'.mp4'];cases.append(dict(name='aac-'+stem,oracle='aac.f32le',rate=48000,channels=2,frames=len(files['aac.f32le'])//8,precise=False))
        files['aac-'+stem+'-edits.mp4']=edit_movie(source[stem+'.mp4'],[(1,-1),(700,SCALE//50),(2,-1),(700,SCALE//50)],clock=3000);cases.append(dict(name='aac-'+stem+'-edits',oracle='aac-edits.f32le',rate=48000,channels=2,frames=len(files['aac-edits.f32le'])//8,precise=False))
    from generate_aac_ssr_fixtures import packet as ssr_packet,oracle as ssr_oracle
    from generate_mp4_aac_transitions import packet,oracle
    sequences=[1,2,3];windows=[0,1,0]
    controls=[([ssr_packet(i,seq,shape,2,True) for i,(seq,shape) in enumerate(zip(sequences,windows))],bytes(ssr_oracle(windows,2,True,sequences=sequences)[0]))]
    for n,ch in [(960,1),(1024,2)]:
        specs=[[(150+f*3+c,(f+c)%4,4+f+c)for c in range(ch)]for f in range(3)]
        controls.append(([packet(s)for s in specs],oracle(n,specs)))
    source=aac([(24000,2,1024,3,6),(32000,1,960,1,5),(44100,2,1024,2,4)],controls)
    groups=[]
    for i,(rate,ch)in enumerate([(24000,2),(32000,1),(44100,2)]):groups.append((rate,ch,list(struct.iter_unpack('<'+'f'*ch,controls[i][1]))))
    aruns=[];at=F(0)
    for i in [0,1,2,0]:
        rate,ch,samples=groups[i];end=at+F(len(samples),rate);aruns.append((at,end,rate,ch,samples));at=end
    files['ssr.f32le']=pulse_oracle([(at,F(0))],24000);files['ssr-edits.f32le']=pulse_oracle(aedits,24000)
    for stem in ['changes','fragmented','third']:
        files['ssr-'+stem+'.mp4']=source[stem+'.mp4'];cases.append(dict(name='ssr-'+stem,oracle='ssr.f32le',rate=24000,channels=2,frames=len(files['ssr.f32le'])//8,precise=False))
        files['ssr-'+stem+'-edits.mp4']=edit_movie(source[stem+'.mp4'],[(1,-1),(700,SCALE//50),(2,-1),(700,SCALE//50)],clock=3000);cases.append(dict(name='ssr-'+stem+'-edits',oracle='ssr-edits.f32le',rate=24000,channels=2,frames=len(files['ssr-edits.f32le'])//8,precise=False))
    from generate_mp4_aac_transitions import individual
    from generate_mp4_audio_selection_regressions import pack_bits
    shapes=[(48000,4,1024,2,3),(48000,2,1024,2,3),(32000,1,960,1,5)]
    controls=[];configs=[]
    for rate,ch,n,profile,fi in shapes:
        specs=[[(150+f*3+c,(f+c)%4,4+f+c)for c in range(ch)]for f in range(3)]
        payloads=[pack_bits(''.join('000'+f'{tag:04b}'+individual(*row)for tag,row in enumerate(frame))+'111')for frame in specs] if ch==4 else [packet(frame)for frame in specs]
        controls.append((payloads,oracle(n,specs)))
        if ch==4:
            bits=f'{profile:05b}{fi:04b}'+'0000'+'000'+'0000'+f'{profile-1:02b}{fi:04b}'+'0100'+'0000'+'0000'+'00'+'000'+'0000'+'000'+''.join('0'+f'{tag:04b}'for tag in range(4))
            configs.append(pack_bits(bits+'0'*(-len(bits)%8)+'00000000'))
        else:configs.append(pack_bits(f'{profile:05b}{fi:04b}{ch:04b}'+('1'if n==960 else'0')+'00'))
    opaque=aac(shapes,controls,configs)
    files['opaque-control.mp4']=opaque['control-0.mp4'];files['opaque-control.f32le']=controls[0][1];files['opaque-changing.mp4']=opaque['changes.mp4']
    aruns=[];at=F(0)
    for i in [0,1,2,0]:
        rate,ch,n,_,_=shapes[i];samples=list(struct.iter_unpack('<'+'f'*ch,controls[i][1]));end=at+F(len(samples),rate);aruns.append((at,end,rate,ch,samples));at=end
    files['opaque-stereo.f32le']=pulse_oracle([(at,F(0))])
    files['opaque-stereo-32k.f32le']=pulse_oracle([(at,F(0))],32000)
    files['opaque-seven.f32le']=pulse_oracle([(at,F(0))],48000,7)
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned neutral VP9; arithmetic PCM constants; authored AAC spectra, scalar native IMDCT and literal output-grid convolution. No private media, external codec, process, FFmpeg or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
