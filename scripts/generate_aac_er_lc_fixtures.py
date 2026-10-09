#!/usr/bin/env python3
"""Own AOT17 epConfig0/no-resilience packets and scalar PCM, offline only."""
import json,struct,math
from generate_aac_main_tools_fixtures import channel,ics,Filterbank,SEQUENCES
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def f32(x):return struct.unpack('<f',struct.pack('<f',x))[0]
def er_channel(seq,q,info,reverse):
    body=channel(seq,[1,1],q,info=info,tns=(reverse,1) if seq!=2 else None)
    if seq==2:return body
    start=8+len(info)+18+2+1
    assert body[start]=='1' and body[start+20]=='0'
    return body[:start+1]+'0'+body[start+1:start+20]+body[start+21:]

def main():
    layouts={1:([0],[0]),2:([1],[0,1]),3:([0,1],[2,0,1]),6:([0,1,1,3],[2,0,1,4,5,3])}
    blob=bytearray();gold=bytearray();cases=[]
    for n in (960,1024):
        for config,(elements,mapping) in layouts.items():
            channels=len(mapping)
            for ms in ((0,) if channels==1 else (0,1,2)):
                banks=[Filterbank(n) for _ in mapping];rows=[];start=len(gold)
                for frame,seq in enumerate(SEQUENCES):
                    shape=frame%2;header=ics(seq,2,False,shape=shape);wire='';spectra=[];ch=0
                    for elem in elements:
                        width=2 if elem==1 else 1;wire+=field(0,4)
                        values=[[[(-1 if (frame+w+k+c)%3==0 else 1 if (frame+w+k+c)%3==1 else 0) for k in range(8)] for w in range(8 if seq==2 else 1)] for c in range(ch,ch+width)]
                        if width==2:wire+='1'+header+field(ms,2)+('10' if ms==1 else '')
                        for c,q in enumerate(values):wire+=er_channel(seq,q,'' if width==2 else header,bool((frame+ch+c)%2))
                        raw_spectra=[[[float(v*1024) for v in row] for row in q] for q in values]
                        if width==2:
                            for w in range(len(raw_spectra[0])):
                                for k in range(8):
                                    if ms==2 or (ms==1 and k<4):
                                        a=raw_spectra[0][w][k];b=raw_spectra[1][w][k];raw_spectra[0][w][k]=a+b;raw_spectra[1][w][k]=a-b
                        for c,spectrum in enumerate(raw_spectra):
                            if seq!=2:
                                order=list(range(8));previous=0.
                                if (frame+ch+c)%2:order.reverse()
                                for k in order:value=spectrum[0][k]-math.sin(math.pi/7)*previous;spectrum[0][k]=f32(value);previous=value
                            spectra.append(spectrum)
                        ch+=width
                    packet=packed(wire);rows.append(dict(offset=len(blob),bytes=len(packet),padding=(-len(wire)%8)));blob.extend(packet)
                    lanes=[banks[c].run(seq,spectra[c],shape) for c in range(channels)]
                    for i in range(n):
                        output=[0.]*channels
                        for coded,pcm in enumerate(mapping):output[pcm]=lanes[coded][i]
                        gold.extend(struct.pack('<'+str(channels)+'f',*output))
                for extension in (False,True):
                    asc=field(17,5)+frequency(24000)+field(config,4)+field(n==960,1)+'0'+field(extension,1)+('0000' if extension else '')+'00'
                    name=f'{n}-{config}-{ms}-{int(extension)}'
                    c=dict(name=name,n=n,channels=channels,asc=packed(asc).hex(),frames=rows,reference_offset=start,reference_bytes=12*n*channels*4,slots=n//64,bands=32,container_rate=24000,container_frame_samples=n,samples=12*n,pcm_offset=0)
                    c['video']=video_fixture([c],blob,channels=channels,filename=f'aac-er-lc-{name}-synthetic.mp4');cases.append(c)
    rejected=[];malformed=[];alignment=[]
    for n in (960,1024):
        base=next(c for c in cases if c['n']==n and c['channels']==1)
        for flags,ep,flag3 in [(flags,0,False) for flags in (1,2,3,5,6,7)]+[(0,ep,False) for ep in (1,2,3)]+[(0,0,True)]:
            asc=field(17,5)+frequency(24000)+'0001'+field(n==960,1)+'01'+field(flags,3)+field(flag3,1)+field(ep,2)
            c=dict(base,asc=packed(asc).hex())
            error='ER AAC LC resilience tools are not yet implemented' if flags else 'ER AAC LC epConfig is not yet implemented' if ep else 'AAC extensionFlag3 must be zero'
            video=video_fixture([c],blob,channels=1,filename=f'aac-er-lc-rejected-{n}-{flags}-{ep}-{int(flag3)}-synthetic.mp4')
            rejected.append(dict(asc=c['asc'],video=video,error=error))
        for kind in ('trailing','padding'):
            c=next(c for c in cases if c['n']==n and c['channels']==6 and (kind=='trailing' or c['frames'][7]['padding']>0))
            row=c['frames'][7];raw=bytearray(blob[row['offset']:row['offset']+row['bytes']])
            if kind=='trailing':raw.append(0)
            else:raw[-1]|=1
            bad=dict(row,offset=len(blob),bytes=len(raw));blob.extend(raw)
            video=video_fixture([dict(c,frames=c['frames'][:7]+[bad])],blob,channels=6,filename=f'aac-er-lc-{n}-{"alignment" if kind=="padding" else kind}-synthetic.mp4')
            if kind=='trailing':malformed.append(dict(video=video,error='trailing bytes after ER AAC block'))
            else:alignment.append(dict(video=video,reference_offset=c['reference_offset'],reference_bytes=8*n*6*4))
    (DEST/'aac-er-lc-packets.bin').write_bytes(blob)
    (DEST/'aac-er-lc-reference.f32le').write_bytes(gold)
    (DEST/'aac-er-lc.json').write_text(json.dumps(dict(cases=cases,rejected=rejected,malformed=malformed,alignment=alignment,provenance='Own ER-LC epConfig0 fixed-order tagged elements, deferred TNS syntax, 960/1024 transitions, sine/KBD, mono/stereo/3.0/5.1 and common MS modes0/1/2. Independent sparse IMDCT/window/TNS/PCM mapping; no private media, FFmpeg or network.'),indent=2)+'\n')
    print(len(cases),'own ER-LC videos')
if __name__=='__main__':main()
