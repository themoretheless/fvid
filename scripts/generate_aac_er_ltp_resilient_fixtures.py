#!/usr/bin/env python3
"""Own ER AAC-LTP resilience combinations and scalar prediction/TNS/PCM."""
import json, struct, math
from functools import partial
from generate_aac_ltp_transition_fixtures import Oracle as BaseOracle, SEQUENCES, GAINS
from generate_aac_main_tools_fixtures import ics, sc, f32
from generate_aac_hcr_fixtures import payload, cw
from generate_aac_rvlc_fixtures import scales
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture

def prediction(active,lag,coef,used):
    return str(int(active))+(field(lag,11)+field(coef,3)+''.join(str(int(x)) for x in used) if active else '')

class Oracle(BaseOracle):
    def run(self,seq,shape,q,active,lag,coefficient,used,tns=None):
        spectra=[r[:] for r in q]
        order=list(range(8))
        if tns:order.reverse()
        a=math.sin(math.pi/7)
        if active:
            estimate=[v*GAINS[coefficient]*self.long_weight(i,seq,shape) for i,v in enumerate(self.history[2*self.n-lag:4*self.n-lag])]
            prediction=[sum(v*c for v,c in zip(estimate,self.cos[self.n][k])) for k in range(8)]
            if tns is not None:
                prior=0.
                for k in order:
                    original=prediction[k];prediction[k]=original+a*prior;prior=original
            spectra[0]=[f32(v+prediction[k]) if used[k//4] else v for k,v in enumerate(spectra[0])]
        if tns is not None:
            prior=0.
            for k in order:
                value=spectra[0][k]-a*prior;spectra[0][k]=f32(value);prior=value
        return super().run(seq,shape,[[v/1024 for v in row] for row in spectra],False,lag,coefficient,used)

def channel(seq,books,values,info='',flags=0,scale_values=None,tns=None,mutation=None):
    section=bool(flags&4);rvlc=bool(flags&2);hcr=bool(flags&1)
    q=[r[:] for r in values]
    if mutation=='virtual-lav':q[0][0]=32
    sections=''.join(field(b,5 if section else 4)+('' if section and (b==11 or b>=16) else field(1,3 if seq==2 else 5)) for b in books)
    if rvlc:
        header,rvdata,_=scales(seq,books,scale_values,gain=140,mutation=mutation)
    else:
        prior=140;header='';rvdata=''
        for book,value in zip(books,scale_values):
            if book:header+=sc(value-prior);prior=value
    if hcr:
        data,hint,_=payload(seq,[books],[8 if seq==2 else 1],q)
        if mutation=='zero-longest':hint=0
        spectral_header=field(len(data),14)+field(hint,6)
    else:
        spectral_header='';data=''
        for band,book in enumerate(books):
            if not book:continue
            dim=4 if book<=4 else 2
            for row in q:
                for k in range(4*band,4*band+4,dim):data+=cw(book,row[k:k+dim])
    body=field(140,8)+info+sections+header+'0'+field(tns is not None,1)+'0'+spectral_header+rvdata
    if tns is not None:body+=field(1,2)+'0'+field(47,6)+field(1,5)+field(tns,1)+'0'+'001'
    return body+data

def large_cpe_cases(blob,gold):
    from generate_aac_hcr_fixtures import reorder,PRIORITY
    from generate_aac_main_tools_fixtures import Filterbank
    result=[];off=[0,4,8,12,16,20,24,28,36,44,52]
    for n,large_channel in [(n,c) for n in (960,1024) for c in (0,1)]:
        banks=[Filterbank(n),Filterbank(n)];rows=[];start=len(gold)
        for frame,seq in enumerate((0,1,2)):
            wire='00000';spectra=[];regions=[]
            for ch in range(2):
                if seq!=2:
                    wire+=field(72,8)+ics(seq,0,False,shape=frame%2)+'000'+field(0,14)+field(0,6);spectra.append([[0.]])
                elif ch!=large_channel:
                    q=[[1,-1,0,1] for _ in range(8)]
                    data,hint,_=payload(2,[[1]],[8],q)
                    regions.append(len(data))
                    wire+=field(72,8)+ics(seq,1,False,shape=frame%2)+field(1,4)+field(1,3)+sc(0)+'000'+field(len(data),14)+field(hint,6)+data
                    spectra.append([[v/128 for v in row] for row in q])
                else:
                    q=[[8191 if (k+w+ch)%2 else -8191 for k in range(off[-1])] for w in range(8)]
                    words=[]
                    for band in range(10):
                        for w in range(8):
                            for line in range(off[band],off[band+1],4):
                                for half in range(2):
                                    values=q[w][line+half*2:line+half*2+2]
                                    words.append(dict(key=(PRIORITY[11],line,w,half),book=11,bits=cw(11,values)))
                    words.sort(key=lambda word:word['key']);data,hint,_=reorder(words)
                    assert 6144<len(data)<=12288
                    regions.append(len(data))
                    wire+=field(72,8)+ics(seq,10,False,shape=frame%2)+field(11,4)+field(7,3)+field(3,3)+sc(0)*10+'000'+field(len(data),14)+field(hint,6)+data
                    spectra.append([[f32(math.copysign(abs(v)**(4/3)/128,v)) for v in row] for row in q])
            assert len(wire)<=12288
            raw=packed(wire);rows.append(dict(offset=len(blob),bytes=len(raw),sequence=seq,active=[False,False],hcr_lengths=regions,reference_offset=len(gold)));blob.extend(raw)
            pcm=[banks[ch].run(seq,spectra[ch],frame%2) for ch in range(2)]
            for i in range(n):gold.extend(struct.pack('<ff',pcm[0][i],pcm[1][i]))
        name=f'{n}-large-independent-{large_channel}';asc=packed(field(19,5)+frequency(24000)+field(2,4)+field(n==960,1)+'01'+'001'+'000').hex()
        case=dict(name=name,n=n,large_channel=large_channel,flags=1,channels=2,asc=asc,frames=rows,reference_offset=start,reference_bytes=len(gold)-start,container_rate=24000,container_frame_samples=n,samples=3*n,pcm_offset=0,slots=n//64,bands=32)
        case['video']=video_fixture([case],blob,channels=2,filename=f'aac-er-ltp-resilient-{name}-synthetic.mp4');result.append(case)
    return result

def main():
    blob=bytearray();gold=bytearray();cases=[];malformed=[]
    for n,flags in [(n,f) for n in (960,1024) for f in range(1,8)]:
        body=partial(channel,flags=flags)
        for mode in ('mono','independent','common-0','common-1','common-2'):
            width=1 if mode=='mono' else 2;common=mode.startswith('common');ms=int(mode[-1]) if common else 0
            banks=[Oracle(n) for _ in range(width)];rows=[];start=len(gold)
            for frame,seq in enumerate(SEQUENCES):
                shape=frame%2;header=ics(seq,2,False,shape=shape);pred=[];values=[];active=[];used=[];lags=[];coefs=[]
                for c in range(width):
                    enabled=seq!=2 and frame>=1 and (frame+c)%4!=0
                    lag=n-13*((frame+c)%3);coef=(frame+3*c)%8;bands=[True,(frame+c)%3!=1]
                    active.append(enabled);lags.append(lag);coefs.append(coef);used.append(bands)
                    pred.append(prediction(enabled,lag,coef,bands))
                    values.append([[(-1 if (frame+w+k+c)%3==0 else 1 if (frame+w+k+c)%3==1 else 0) for k in range(8)] for w in range(8 if seq==2 else 1)])
                books=[[17 if flags&4 else 1,1] for _ in range(width)]
                if frame==0:
                    books=[[0,0] for _ in range(width)];values=[[[0]*8] for _ in range(width)]
                for c in range(width):
                    if books[c][0]==17:
                        for row in values[c]:row[:4]=[v*17 for v in row[:4]]
                delta=[0,-8,8,7,-7,6,-6,1,-1,0,8,-8][frame]
                scale_values=[[140+delta,140] for _ in range(width)]
                tns=[bool((frame+c)%2) if seq!=2 else None for c in range(width)]
                kwargs=[dict(scale_values=scale_values[c],tns=tns[c]) for c in range(width)]
                kind,error=('zero-longest','AAC HCR nonempty region has zero longest codeword') if flags&1 else ('reverse-gain','AAC RVLC reverse gain mismatch') if flags&2 else ('virtual-lav','AAC virtual codebook magnitude exceeds section limit')
                present=seq!=2 and any(active)
                wire='0000'
                if width==2:wire+=str(int(common))
                badwire=None
                if common:
                    wire+=header if seq==2 else header[:-1]+str(int(present))
                    wire+=field(ms,2)+('10' if ms==1 else '')
                    if present:
                        wire+=pred[0]
                    prefix=wire;wire+=body(seq,books[0],values[0],**kwargs[0])
                    if frame==2:badwire=prefix+body(seq,books[0],values[0],mutation=kind,**kwargs[0])
                    suffix=(pred[1] if present else '')+body(seq,books[1],values[1],**kwargs[1]);wire+=suffix
                    if frame==2:badwire+=suffix
                else:
                    for c in range(width):
                        info=header if seq==2 else header[:-1]+str(int(active[c]))+(pred[c] if active[c] else '')
                        packet_body=body(seq,books[c],values[c],info=info,**kwargs[c])
                        if frame==2:
                            if c==0:badwire=wire+body(seq,books[c],values[c],info=info,mutation=kind,**kwargs[c])
                            else:badwire+=packet_body
                        wire+=packet_body
                raw=packed(wire);row=dict(offset=len(blob),bytes=len(raw),sequence=seq,active=active,reference_offset=len(gold));blob.extend(raw);rows.append(row)
                mixed=[[[f32(math.copysign(abs(v)**(4/3)*2**((scale_values[c][k//4]-100)/4),v)) if v else 0. for k,v in enumerate(row)] for row in q] for c,q in enumerate(values)]
                if common:
                    for w in range(len(mixed[0])):
                        for k in range(8):
                            if ms==2 or (ms==1 and k<4):
                                a,b=mixed[0][w][k],mixed[1][w][k];mixed[0][w][k]=f32(a+b);mixed[1][w][k]=f32(a-b)
                pcm=[banks[c].run(seq,shape,mixed[c],active[c],lags[c],coefs[c],used[c],tns[c]) for c in range(width)]
                for i in range(n):gold.extend(struct.pack('<'+'f'*width,*(p[i] for p in pcm)))
                if frame==2:
                    bad=packed(badwire);row['malformed']=dict(offset=len(blob),bytes=len(bad),error=error);blob.extend(bad)
            asc=packed(field(19,5)+frequency(24000)+field(width,4)+field(n==960,1)+'01'+field(flags,3)+'000')
            name=f'{n}-{flags}-{mode}';case=dict(name=name,flags=flags,n=n,asc=asc.hex(),channels=width,frames=rows,reference_offset=start,reference_bytes=len(gold)-start,container_rate=24000,container_frame_samples=n,samples=12*n,pcm_offset=0,slots=n//64,bands=32)
            case['video']=video_fixture([case],blob,channels=width,filename=f'aac-er-ltp-resilient-{name}-synthetic.mp4');cases.append(case)
            bad=rows[2]['malformed'];badcase=dict(case,frames=rows[:2]+[bad]);malformed.append(dict(video=video_fixture([badcase],blob,channels=width,filename=f'aac-er-ltp-resilient-{name}-malformed-synthetic.mp4'),error=bad['error']))
    large=large_cpe_cases(blob,gold)
    (DEST/'aac-er-ltp-resilient-packets.bin').write_bytes(blob);(DEST/'aac-er-ltp-resilient-reference.f32le').write_bytes(gold)
    (DEST/'aac-er-ltp-resilient.json').write_text(json.dumps(dict(cases=cases,large_cpe=large,malformed=malformed,provenance='Own AOT19 ep0 all resilience combinations, virtual books, RVLC escapes, HCR, deferred stereo LTP and bidirectional TNS; scalar prediction/FIR/AR/cosine/window/float-history oracle. No private media or foreign decoder.'),indent=2)+'\n')
    print(f'generated {len(cases)} acceptance and {len(malformed)} malformed videos')
if __name__=='__main__':main()
