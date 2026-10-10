#!/usr/bin/env python3
"""Own ER RVLC bits and independent scalar synthesis, offline only."""
import json,math,struct
from generate_aac_main_tools_fixtures import ics,Filterbank,Noise,SEQUENCES,tuple_bits,f32
from generate_aac_er_sections_fixtures import pair
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
# ISO/IEC 14496-3 tables4.113 and4.115: factual syntax, not decoder code.
RV=[(7,65),(9,257),(8,129),(6,33),(5,17),(4,9),(3,5),(1,0),(3,7),(5,27),(6,51),(7,107),(8,195),(9,427),(7,99)]
ESC=[(2,2),(2,0),(3,6),(3,2),(4,14),(5,31),(5,15),(5,13),(6,61),(6,29),(6,25),(6,24),(7,120),(7,56),(8,242),(8,114),(9,486),(9,230),(10,974),(10,463),(11,1950),(11,1951),(11,925),(12,1848),(14,7399),(13,3698),(15,14797)]+[(20,473455+i) for i in range(27,49)]+[(19,236687+i) for i in range(49,54)]
def rvlc(deltas):
    bits='';esc=''
    for d in deltas:
        base=max(-7,min(7,d));width,value=RV[base+7];bits+=field(value,width)
        if abs(d)>=7:
            width,value=ESC[abs(d)-7];esc+=field(value,width)
    return bits,esc

def scales(seq,books,values,gain=100,mutation=None):
    spectral=gain;noise=gain-90;intensity=0;deltas=[];first=None;absolute=[]
    for book,value in zip(books,values):
        absolute.append(value)
        if book==0:continue
        if book==13:
            if first is None:first=value-noise+256
            else:deltas.append(value-noise)
            noise=value
        elif book in (14,15):deltas.append(value-intensity);intensity=value
        else:deltas.append(value-spectral);spectral=value
    if any(b in (14,15) for b in books):deltas.append(intensity+(1 if mutation=='intensity-last' else 0))
    sf,esc=rvlc(deltas)
    if mutation=='forbidden':sf=field(50,6)+sf[1:]
    if mutation=='missing-escape':esc=''
    if mutation=='extra-escape':esc+='10'
    length=len(sf)+(9 if first is not None else 0)
    if mutation=='sf-length':length+=1
    if mutation=='noise-length':length=8
    header=field(1,1)+field(spectral+(1 if mutation=='reverse-gain' else 0),8)+field(length,11 if seq==2 else 9)
    if first is not None:header+=field(first,9)
    header+=field(bool(esc),1)+(field(len(esc)+(1 if mutation=='escape-length' else 0),8) if esc else '')
    if first is not None:header+=field(noise-spectral+90+256+(1 if mutation=='noise-last' else 0),9)
    return header,sf+esc,absolute

def channel(seq,books,values,q,section,info='',tns=False,mutation=None):
    sections=''.join(field(b,5 if section else 4)+('' if section and (b==11 or b>=16) else field(1,3 if seq==2 else 5)) for b in books)
    header,data,absolute=scales(seq,books,values,mutation=mutation)
    body=field(100,8)+info+sections+header+'0'+field(tns,1)+'0'+data
    if tns:body+=field(1,2)+'0'+field(47,6)+field(1,5)+'00'+'001'
    for band,book in enumerate(books):
        if book in (0,13,14,15):continue
        for row in q:
            v=row[4*band:4*band+4]
            body+=tuple_bits(v) if book==1 else pair(*v[:2])+pair(*v[2:])
    return body,absolute

def main():
    blob=bytearray();gold=bytearray();cases=[];malformed=[]
    sweep=[-60,-31,-7,-6,-1,0,1,6,7,31,59,60]
    for n in (960,1024):
      for section in (False,True):
       for mode in ('mono','stereo','intensity','pns'):
        channels=1 if mode=='mono' else 2;banks=[Filterbank(n) for _ in range(channels)];noise=Noise();frames=[];start=len(gold)
        for frame,seq in enumerate(SEQUENCES):
            shape=frame%2;d=sweep[frame];info=ics(seq,3,False,shape=shape);wire='0000';common=channels==2
            if common:wire+='1'+info+'00'
            spectra=[];books0=[16 if section else 1,1,11]
            for ch in range(channels):
                books=books0.copy();values=[100+d,100,100-d]
                if ch and mode=='intensity':books=[books[0],14 if frame%2 else 15,15 if frame%2 else 14];values=[100+d,d,d+(7 if d<=53 else -7)]
                if mode=='pns':books=[books[0],13,13];values=[100+d,20+ch,20+ch+d]
                q=[[(-1 if (frame+ch+w+k)%3==0 else 1 if (frame+ch+w+k)%3==1 else 0) for k in range(12)] for w in range(8 if seq==2 else 1)]
                tns=mode=='mono' and seq!=2
                body,absolute=channel(seq,books,values,q,section,info='' if common else info,tns=tns);wire+=body
                spectrum=[[0.]*12 for _ in q]
                for band,book in enumerate(books):
                    for w,row in enumerate(q):
                        if book==13:spectrum[w][4*band:4*band+4]=noise.band(absolute[band])
                        elif book not in (0,14,15):spectrum[w][4*band:4*band+4]=[f32(v*2**((absolute[band]-100)/4)) for v in row[4*band:4*band+4]]
                if mode=='intensity' and ch:
                    for band in (1,2):
                        sign=1 if books[band]==15 else -1
                        for w in range(len(q)):
                            spectrum[w][4*band:4*band+4]=[f32(v*sign*2**(-absolute[band]/4)) for v in spectra[0][w][4*band:4*band+4]]
                if tns:
                    previous=0.
                    for k in range(12):
                        v=f32(spectrum[0][k]-math.sin(math.pi/7)*previous);spectrum[0][k]=v;previous=v
                spectra.append(spectrum)
            raw=packed(wire);frames.append(dict(offset=len(blob),bytes=len(raw),padding=(-len(wire)%8)));blob.extend(raw)
            lanes=[banks[ch].run(seq,spectra[ch],shape) for ch in range(channels)]
            for i in range(n):gold.extend(struct.pack('<'+str(channels)+'f',*(lane[i] for lane in lanes)))
        asc=packed(field(17,5)+frequency(24000)+field(channels,4)+field(n==960,1)+'01'+field(6 if section else 2,3)+'000').hex()
        c=dict(name=f'{n}-{int(section)}-{mode}',n=n,channels=channels,asc=asc,frames=frames,reference_offset=start,reference_bytes=12*n*channels*4,slots=n//64,bands=32,container_rate=24000,container_frame_samples=n,samples=12*n,pcm_offset=0)
        c['video']=video_fixture([c],blob,channels=channels,filename=f'aac-rvlc-{c["name"]}-synthetic.mp4');cases.append(c)
        if mode=='intensity':
            q=[[1,-1,0,1]*3]
            left,_=channel(0,[1,1,11],[100,100,100],q,section)
            right,_=channel(0,[1,14,15],[100,1,2],q,section,mutation='intensity-last')
            raw=packed('0000'+'1'+ics(0,3,False)+'00'+left+right)
            row=dict(offset=len(blob),bytes=len(raw),padding=0);blob.extend(raw)
            bad=dict(c,frames=[row],samples=n)
            video=video_fixture([bad],blob,channels=2,filename=f'aac-rvlc-bad-{n}-{int(section)}-intensity-last-synthetic.mp4')
            malformed.append(dict(video=video,frame=row,asc=asc,error='AAC RVLC reverse intensity mismatch',kind='intensity-last'))
        if mode!='mono':continue
        for kind,error in [('forbidden','invalid AAC RVLC codeword'),('missing-escape','truncated AAC RVLC escape'),('extra-escape','AAC RVLC escape length mismatch'),('sf-length','AAC RVLC scalefactor length mismatch'),('reverse-gain','AAC RVLC reverse gain mismatch'),('escape-length','AAC RVLC escape length mismatch'),('noise-length','AAC RVLC noise length is below nine bits'),('noise-last','AAC RVLC reverse noise mismatch')]:
            books=[1,13,13] if kind.startswith('noise') else [1,1,11];values=[100,20,21] if kind.startswith('noise') else [100,107,100]
            if kind=='sf-length':values=[100,100,100]
            body,_=channel(0,books,values,[[1,-1,0,1]*3],section,info=ics(0,3,False),mutation=kind)
            raw=packed('0000'+body);row=dict(offset=len(blob),bytes=len(raw),padding=0);blob.extend(raw)
            bad=dict(c,frames=[row],samples=n)
            video=video_fixture([bad],blob,channels=1,filename=f'aac-rvlc-bad-{n}-{int(section)}-{kind}-synthetic.mp4')
            malformed.append(dict(video=video,frame=row,asc=asc,error=error,kind=kind))
    (DEST/'aac-rvlc-packets.bin').write_bytes(blob);(DEST/'aac-rvlc-reference.f32le').write_bytes(gold)
    (DEST/'aac-rvlc.json').write_text(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own ER RVLC header/class2 separation, scalar PNS/intensity/TNS/IMDCT/window synthesis, signed deltas and escapes, 960/1024 transitions with and without virtual books. No private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
