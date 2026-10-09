#!/usr/bin/env python3
"""Own Main prediction spectral oracle and short MP4; no codec executables.
Oracle uses explicit scalar stage updates and generated inverse tables from
ISO/IEC 13818-7 clause 13, not a call to the production predictor.
"""
import json, struct, math
from generate_aac_ssr_fixtures import DEST, CODES, LENS, SC, SL, field, frequency, packed
from generate_he_aac_packet_fixtures import video_fixture

def f32(x): return struct.unpack('<f',struct.pack('<f',x))[0]
def bits(x): return struct.unpack('<I',struct.pack('<f',x))[0]
def number(x): return struct.unpack('<f',struct.pack('<I',x))[0]
def trunc(x): return number(bits(x)&0xffff0000)
def add(a,b): return f32(a+b)
def mul(a,b): return f32(a*b)
def nearest(x):
    # Scale normal mantissa into an integer lattice; Python ties-to-even.
    word=bits(x); exponent=((word>>23)&255)-127
    step=2.**(exponent-7)
    return f32(round(x/step)*step)
MANT=[nearest(f32((61/64)/(1+i/128))) for i in range(128)]
def inverse(x):
    word=bits(x); exponent=(word>>23)&255
    return 0. if exponent<=127 else f32(MANT[(word>>16)&127]*2.**(127-exponent))
def initial():return [0.,0.,0.,0.,1.,1.]
def step(state,residual,active):
    r0,r1,c0,c1,v0,v1=state
    k0=mul(c0,inverse(v0)); k1=mul(c1,inverse(v1))
    p0=mul(k0,r0); p1=mul(k1,r1); prediction=add(p0,p1)
    # Nearest seven-bit mantissa, ties away from zero, expressed as a lattice.
    if prediction:
        exponent=((bits(prediction)>>23)&255)-127
        unit=2.**(exponent-7)
        magnitude=int(abs(prediction)/unit+0.5)
        prediction=f32(magnitude*unit*(-1 if prediction<0 else 1))
    x=add(residual,prediction) if active else residual
    e1=add(x,-p0)
    next_state=[mul(61/64,x),mul(61/64,add(r0,-mul(k0,x))),
      add(mul(29/32,c0),mul(r0,x)),add(mul(29/32,c1),mul(r1,e1)),
      add(mul(29/32,v0),mul(.5,add(mul(r0,r0),mul(x,x)))),
      add(mul(29/32,v1),mul(.5,add(mul(r1,r1),mul(e1,e1))))]
    return x,list(map(trunc,next_state))
def main():
    states=[initial() for _ in range(64)];rows=[]
    for frame in range(36):
        if frame in (12,25):
            states=[initial() for _ in states];rows.append(dict(short=True));continue
        used=[] if frame<3 else ([True] if frame%3==0 else [False,True,frame%2==0])
        reset={7:1,8:30,19:2,35:30}.get(frame)
        source=[float(((i*11+frame*7)%23-11)*16) for i in range(64)]
        output=[]
        for band,(start,end) in enumerate(zip([0,4,34],[4,34,64])):
            for i in range(start,end):
                value,states[i]=step(states[i],source[i],used[band] if band<len(used) else False);output.append(bits(value))
        if reset:
            for i in range(reset-1,64,30):states[i]=initial()
        rows.append(dict(input=source,used=used,reset=reset,output_bits=output))
    blob=bytearray();cases=[];invalid=[]
    sequences=[0,0,0,0,1,2,2,3,0,0,0,0]
    for name,n,rate in [('mono-1024',1024,24000),('mono-960',960,48000)]:
        frames=[];pcm=bytearray();history=[initial() for _ in range(4)];overlap=[0.]*n
        for frame,seq in enumerate(sequences):
            active=frame>=3 and seq!=2;reset=1 if frame==10 else None
            prediction=('1'+('1'+field(reset,5) if reset else '0')+str(int(active))) if active else '0'
            info='0'+field(seq,2)+'0'+field(1,4 if seq==2 else 6)+('1111111' if seq==2 else prediction)
            indices=[1,-1,1,-1] if frame%2 else [-1,1,-1,1]
            index=0
            for value in indices:index=index*3+value+1
            spectral=field(CODES[index],LENS[index])*(8 if seq==2 else 1)
            packet=packed('0000000'+field(140,8)+info+field(1,4)+field(1,3 if seq==2 else 5)+field(SC[60],SL[60])+'000'+spectral+'111')
            frames.append(dict(offset=len(blob),bytes=len(packet),samples=n));blob.extend(packet)
            if seq==2:history=[initial() for _ in range(4)];coefficients=[float(v*1024) for v in indices]*8
            else:
                coefficients=[]
                for i,residual in enumerate(indices):
                    value,history[i]=step(history[i],float(residual*1024),active);coefficients.append(value)
                if reset:history[reset-1]=initial()
            # Sparse direct IMDCT with literal window placement and overlap-add.
            short=n//8;offset=(n-short)//2;block=[0.]*(2*n)
            def imdct(values,size):
                return [2/size*sum(value*math.cos(math.pi/size*(j+.5+size/2)*(k+.5)) for k,value in enumerate(values)) for j in range(2*size)]
            def sine(size,j):return math.sin(math.pi*(j+.5)/(2*size))
            if seq==2:
                for w in range(8):
                    transformed=imdct(coefficients[4*w:4*w+4],short)
                    for j,value in enumerate(transformed):block[offset+w*short+j]+=value*sine(short,j)
            else:
                block=imdct(coefficients,n)
                for j in range(2*n):
                    weight=sine(n,j)
                    if seq==1 and j>=n:
                        t=j-n;weight=1. if t<offset else (sine(short,short+t-offset) if t<offset+short else 0.)
                    if seq==3 and j<n:weight=0. if j<offset else (sine(short,j-offset) if j<offset+short else 1.)
                    block[j]*=weight
            for j in range(n):pcm+=struct.pack('<f',f32(overlap[j]+block[j])/65536)
            overlap=block[n:]
        asc=packed(field(1,5)+frequency(rate)+field(1,4)+str(int(n==960))+'00')
        case=dict(name=name,asc=asc.hex(),frames=frames,slots=16,bands=32,container_rate=rate,container_frame_samples=n,pcm_offset=0,samples=n*len(frames))
        filename='aac-main-prediction-synthetic.mp4' if n==1024 else 'aac-main-prediction-960-synthetic.mp4'
        case['video']=video_fixture([case],blob,filename=filename)
        case['pcm_file']='aac-main-prediction-'+name+'-pcm.bin';(DEST/case['pcm_file']).write_bytes(pcm);cases.append(case)
        if n==1024:
            for group in (0,31):
                bad_packet=packed('0000000'+field(140,8)+'0000'+field(1,6)+'11'+field(group,5)+'1'+field(1,4)+field(1,5)+field(SC[60],SL[60])+'000'+field(CODES[index],LENS[index])+'111')
                bad=dict(case,frames=[dict(offset=len(blob),bytes=len(bad_packet),samples=n)],samples=n);blob.extend(bad_packet)
                bad['video']=video_fixture([bad],blob,filename='aac-main-prediction-invalid-reset-'+str(group)+'-synthetic.mp4');invalid.append(bad)
    (DEST/'aac-main-prediction-packets.bin').write_bytes(blob)
    (DEST/'aac-main-prediction.json').write_text(json.dumps(dict(provenance='Own integer spectra, own bit writer and scalar ISO13818-7 clause13 oracle; own existing AVC seed/container templates. No private media or external codecs.',offsets=[0,4,34,64],oracle=rows,case=cases[0],cases=cases,invalid=invalid),indent=2)+'\n')
if __name__=='__main__':main()
