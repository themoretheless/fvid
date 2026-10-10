#!/usr/bin/env python3
"""Own AAC Main stereo/PNS/order-20 TNS videos and scalar PCM oracles.
Offline bit writing and direct transforms only; no private or foreign media.
"""
import json, math, struct
from generate_aac_ssr_fixtures import DEST, CODES, LENS, SC, SL, field, frequency, packed, windows
from generate_aac_main_prediction_fixtures import initial, step, f32, add
from generate_he_aac_packet_fixtures import video_fixture
SEQUENCES=[0,0,0,0,1,2,2,3,0,0,0,0]

def sc(delta):return field(SC[60+delta],SL[60+delta])
def tuple_bits(values):
    index=0
    for value in values:index=index*3+value+1
    return field(CODES[index],LENS[index])
def ics(seq,bands,active,reset=None,shape=0):
    prediction='0' if not active else '1'+('1'+field(reset,5) if reset else '0')+'1'*bands
    return '0'+field(seq,2)+str(shape)+field(bands,4 if seq==2 else 6)+('1111111' if seq==2 else prediction)
def channel(seq,books,values,position=0,energy=50,info='',tns=None,er=False):
    sections=''.join(field(book,4)+field(1,3 if seq==2 else 5) for book in books)
    scales='';first_noise=True
    for book in books:
        if book==1:scales+=sc(0)
        elif book in (14,15):scales+=sc(position)
        elif book==13:
            scales+=field(energy-50+256,9) if first_noise else sc(0);first_noise=False
    # gain=140 => ordinary +/-1 maps to +/-1024; initial noise accumulator=50.
    payload=field(140,8)+info+sections+scales+'0'
    if tns:
        reverse,order=tns
        # length=47 reaches band zero, clipped to max_sfb=8 (32 coefficients).
        payload+='1'+('0' if er else '')+field(1,2)+'0'+field(47,6)+field(order,5)+field(reverse,1)+'0'
        payload+='000'*(order-1)+'001'
    else:payload+='0'
    if not (er and tns):payload+='0'
    for band,book in enumerate(books):
        if book==1:
            for window in range(8 if seq==2 else 1):payload+=tuple_bits(values[window][4*band:4*band+4])
    return payload

class Noise:
    def __init__(self):self.state=0x6d2b79f5
    def band(self,energy):
        out=[]
        for _ in range(4):
            self.state^=(self.state<<13)&0xffffffff;self.state^=self.state>>17;self.state^=(self.state<<5)&0xffffffff
            out.append(f32(((self.state>>16)+.5)/32768-1))
        factor=2**(energy/4)/math.sqrt(sum(v*v for v in out))
        return [f32(v*factor) for v in out]

class Filterbank:
    def __init__(self,n=1024):
        self.n=n;self.overlap=[0.]*n;self.previous_shape=0
        self.kbd={n:windows(n,4),n//8:windows(n//8,6)}
    def run(self,seq,spectra,shape=0):
        n=self.n;small=n//8;offset=(n-small)//2;block=[0.]*(2*n)
        def transform(values,size):
            sparse=[(k,v) for k,v in enumerate(values) if v]
            return [2/size*sum(v*math.cos(math.pi/size*(j+.5+size/2)*(k+.5)) for k,v in sparse) for j in range(2*size)]
        def weight(size,j,kind):return self.kbd[size][j] if kind else math.sin(math.pi*(j+.5)/(2*size))
        if seq==2:
            for w,values in enumerate(spectra):
                for j,value in enumerate(transform(values,small)):block[offset+w*small+j]+=value*weight(small,j,self.previous_shape if w==0 and j<small else shape)
        else:
            block=transform(spectra[0],n)
            for j in range(2*n):
                gain=weight(n,j,self.previous_shape if j<n else shape)
                if seq==1 and j>=n:
                    t=j-n;gain=1. if t<offset else (weight(small,small+t-offset,shape) if t<offset+small else 0.)
                if seq==3 and j<n:gain=0. if j<offset else (weight(small,j-offset,self.previous_shape) if j<offset+small else 1.)
                block[j]*=gain
        out=[f32(f32(self.overlap[j]+block[j])/65536) for j in range(n)];self.overlap=block[n:];self.previous_shape=shape;return out

def predict(history,values,active,books,reset=None):
    result=[]
    for i,value in enumerate(values):
        book=books[i//4]
        if book==13:
            # Noise never enters the Main predictor, and resets its history.
            history[i]=initial();result.append(value)
        else:
            output,history[i]=step(history[i],value,active and book not in (14,15));result.append(output)
    if reset:
        for i in range(reset-1,len(history),30):history[i]=initial()
    return result

def main():
    blob=bytearray();cases=[];invalid=[]
    names=['ms-explicit','ms-all','independent','intensity-explicit','intensity-all','pns-right-switch','pns-correlated-switch']
    for name,n in [(name,n) for n in (1024,960) for name in names]:
        history=[[initial() for _ in range(8)] for _ in range(2)]
        banks=[Filterbank(n),Filterbank(n)];noise=Noise();frames=[];pcm=bytearray()
        for frame,seq in enumerate(SEQUENCES):
            count=8 if seq==2 else 1;active=frame>=3 and seq!=2;reset=1 if frame==10 else None
            common=name!='independent';mode=2 if name.endswith('-all') else 1
            right_seq=seq if common else [0,0,1,2,2,3,0,0,1,2,3,0][frame]
            right_count=8 if right_seq==2 else 1
            mask=[True,(frame//2)%2==0] if mode==1 else [True,True]
            left_books=[1,1];right_books=[1,1]
            position=4 if frame%2 else 0;energy_left=50;energy_right=54
            if name.startswith('intensity') and 3<=frame<=8:right_books[1]=14 if frame%2 else 15
            if name.startswith('pns') and 3<=frame<=8:
                right_books[1]=13
                if name.startswith('pns-correlated'):left_books[1]=13;mask[1]=True
            left_wire=[];right_wire=[]
            for w in range(count):
                sign=-1 if (frame+w)%2 else 1
                left_wire.append([sign,-sign,sign,-sign]+[-sign,0,sign,0])
            for w in range(right_count):
                sign=-1 if (frame+w)%2 else 1
                right_wire.append([0,sign,0,-sign]+[sign,sign,0,-sign])
            shape=frame%2
            info=ics(seq,2,active,reset,shape)
            wire='0010000'+str(int(common))
            if common:wire+=info+field(mode,2)+(''.join(str(int(v)) for v in mask) if mode==1 else '')
            wire+=channel(seq,left_books,left_wire,energy=energy_left,info='' if common else info)
            right_active=(frame>=3 and right_seq!=2 and frame%2==0) if not common else active
            wire+=channel(right_seq,right_books,right_wire,position,energy_right,info='' if common else ics(right_seq,2,right_active,30 if frame==9 else None,shape if common else 1-shape))
            payload=packed(wire+'111');frames.append(dict(offset=len(blob),bytes=len(payload),samples=n,sequence=seq,right_sequence=right_seq,shape=shape,left_books=left_books,right_books=right_books,mask=mask if common else None));blob.extend(payload)
            spectra=[[[float(v*1024) for v in values] for values in left_wire],[[float(v*1024) for v in values] for values in right_wire]]
            for ch,books in enumerate([left_books,right_books]):
                for band,book in enumerate(books):
                    for w in range(count if ch==0 else right_count):
                        if book==13:spectra[ch][w][4*band:4*band+4]=noise.band(energy_left if ch==0 else energy_right)
                        elif book>=14:spectra[ch][w][4*band:4*band+4]=[0.]*4
            for band in range(2):
                if not common or not mask[band] or right_books[band]>=14:continue
                for w in range(count):
                    for i in range(4*band,4*band+4):
                        a,b=spectra[0][w][i],spectra[1][w][i]
                        if left_books[band]==13 or right_books[band]==13:
                            if left_books[band]==right_books[band]==13:spectra[1][w][i]=f32(a*2**((energy_right-energy_left)/4))
                        else:spectra[0][w][i]=add(a,b);spectra[1][w][i]=add(a,-b)
            if seq==2:history[0]=[initial() for _ in range(8)]
            else:spectra[0][0]=predict(history[0],spectra[0][0],active,left_books,reset)
            for band in range(2):
                if right_books[band]>=14:
                    sign=1 if (right_books[band]==15) != (mode==1 and mask[band]) else -1
                    for w in range(count):
                        for i in range(4*band,4*band+4):spectra[1][w][i]=f32(spectra[0][w][i]*sign*2**(-position/4))
            if right_seq==2:history[1]=[initial() for _ in range(8)]
            else:spectra[1][0]=predict(history[1],spectra[1][0],right_active,right_books,reset if common else (30 if frame==9 else None))
            output=[banks[ch].run(seq if ch==0 else right_seq,spectra[ch],shape if common or ch==0 else 1-shape) for ch in range(2)]
            for a,b in zip(*output):pcm+=struct.pack('<ff',a,b)
        rate=24000 if n==1024 else 48000
        asc=packed(field(1,5)+frequency(rate)+field(2,4)+str(int(n==960))+'00')
        if n==960:name+='-960'
        case=dict(name=name,asc=asc.hex(),channels=2,frames=frames,slots=16,bands=32,container_rate=rate,container_frame_samples=n,pcm_offset=0,samples=n*12)
        case['video']=video_fixture([case],blob,channels=2,filename='aac-main-tools-'+name+'-synthetic.mp4')
        case['pcm_file']='aac-main-tools-'+name+'-pcm.bin';(DEST/case['pcm_file']).write_bytes(pcm);cases.append(case)
    for reverse in [False,True]:
        history=[initial() for _ in range(32)];bank=Filterbank();control_bank=Filterbank();frames=[];control_frames=[];pcm=bytearray();control_pcm=bytearray()
        for frame in range(8):
            active=frame>=3;reset=2 if frame==6 else None;values=[0]*32
            start=28 if reverse else 0;sign=-1 if frame%2 else 1;values[start:start+4]=[sign,-sign,sign,-sign]
            info=ics(0,8,active,reset)
            for tns,target in [((reverse,20),frames),(None,control_frames)]:
                payload=packed('0000000'+channel(0,[1]*8,[values],info=info,tns=tns)+'111');target.append(dict(offset=len(blob),bytes=len(payload),samples=1024));blob.extend(payload)
            spectrum=predict(history,[float(v*1024) for v in values],active,[1]*8,reset)
            control=control_bank.run(0,[spectrum]);control_pcm+=struct.pack('<1024f',*control)
            # Only the twentieth LPC coefficient is nonzero. The scalar
            # recurrence retains full double precision values across samples.
            filtered=list(spectrum);pole=math.sin(math.pi/2/3.5);order=list(range(31,-1,-1)) if reverse else list(range(32))
            for i,position in enumerate(order):
                if i>=20:filtered[position]-=pole*filtered[order[i-20]]
            output=bank.run(0,[[f32(v) for v in filtered]]);pcm+=struct.pack('<1024f',*output)
        name='tns20-reverse' if reverse else 'tns20-forward';asc=packed(field(1,5)+frequency(24000)+field(1,4)+'000')
        case=dict(name=name,asc=asc.hex(),channels=1,frames=frames,slots=16,bands=32,container_rate=24000,container_frame_samples=1024,pcm_offset=0,samples=8192)
        case['video']=video_fixture([case],blob,filename='aac-main-tools-'+name+'-synthetic.mp4')
        case['control_video']=video_fixture([dict(case,frames=control_frames)],blob,filename='aac-main-tools-'+name+'-control-synthetic.mp4')
        case['pcm_file']='aac-main-tools-'+name+'-pcm.bin';case['control_pcm_file']='aac-main-tools-'+name+'-control-pcm.bin'
        (DEST/case['pcm_file']).write_bytes(pcm);(DEST/case['control_pcm_file']).write_bytes(control_pcm);cases.append(case)
        payload=packed('0000000'+channel(0,[1]*8,[values],info=ics(0,8,False),tns=(reverse,21))+'111')
        bad=dict(case,name=name+'-invalid21',frames=[dict(offset=len(blob),bytes=len(payload),samples=1024)],samples=1024);blob.extend(payload)
        bad['video']=video_fixture([bad],blob,filename='aac-main-tools-'+name+'-invalid21-synthetic.mp4');invalid.append(bad)
    (DEST/'aac-main-tools-packets.bin').write_bytes(blob)
    (DEST/'aac-main-tools.json').write_text(json.dumps(dict(provenance='Own spectra, bit writer, sparse direct IMDCT, scalar Main predictor and TNS recurrence. Own existing AVC/container seeds; no private media or foreign codec tools.',cases=cases,invalid=invalid),indent=2)+'\n')
if __name__=='__main__':main()
