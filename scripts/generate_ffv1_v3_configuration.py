#!/usr/bin/env python3
"""Original RFC 9043 configuration/slice records and a small v3 pixel reproducer."""
import hashlib
import json
from generate_ffv1_rgb_fixtures import Range, TABLE, element, uint
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def crc(data):
    value=0
    for byte in data:
        value ^= byte<<24
        for _ in range(8):value=((value<<1)^(0x04c11db7 if value&0x80000000 else 0))&0xffffffff
    return value
def parity(data):
    data+=crc(data).to_bytes(4,'big');assert crc(data)==0;return data
def config(coder=1,depth=8,rgb=False,chroma=True,alpha=False,grid=(1,1),ec=1,intra=0,micro=4,version=3,count=2,zero_state=False,sx=0,sy=0,custom=None,quantization=None,initial_offset=0):
    r=Range();h=[128]*32
    r.integer(h,version);r.integer(h,micro);r.integer(h,coder)
    transitions=custom if custom is not None else [0]+[max(1,n) for n in TABLE[1:]]
    if coder==2:
        for i in range(1,256):r.integer(h,transitions[i]-TABLE[i],True)
    r.integer(h,int(rgb));r.integer(h,depth);r.bit(h,0,chroma);r.integer(h,sx);r.integer(h,sy);r.bit(h,0,alpha)
    r.integer(h,grid[0]-1);r.integer(h,grid[1]-1);r.integer(h,count)
    counts=[]
    for i in range(count):
        scale=1
        for table in range(5):
            model=[128]*32
            runs=quantization[i][table] if quantization is not None else ([0,126] if i==0 and table==0 else [127])
            for run in runs:r.integer(model,run)
            scale*=2*len(runs)-1
        counts.append((scale+1)//2)
    initial=[];states=[[128]*32 for _ in range(32)]
    for i in range(count):
        r.bit(h,0,True);models=[]
        for j in range(counts[i]):
            row=[]
            for k in range(32):
                value=0 if zero_state and i==0 and j==0 and k==0 else 144+(k%7)-j*3+initial_offset*i
                predictor=models[j-1][k] if j else 128
                r.integer(states[k],value-predictor,True);row.append(value)
            models.append(row)
        initial.append(models)
    r.integer(h,ec);r.integer(h,intra)
    return parity(r.finish()),initial,transitions
def sliced(payload,ec,status=0):
    data=payload+len(payload).to_bytes(3,'big')
    return parity(data+bytes([status])) if ec else data
def movie(private,packet):
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_FFV1')+element(0x63a2,private)+uint(0x23e383,20000000)+element(0xe0,uint(0xb0,4)+uint(0xba,2))
    cluster=uint(0xe7,0)+element(0xa3,b'\x81\0\0\x80'+packet)
    return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000))+element(0x1654ae6b,element(0xae,track))+element(0x1f43b675,cluster))
def main():
    cases=[];invalid=[];slices=[]
    for coder in [0,1,2]:
        for depth in [8,10,12,16]:
            for rgb in [False,True]:
                name=f'ffv1-v3-config-coder{coder}-depth{depth}-rgb{int(rgb)}'
                data,initial,transitions=config(coder=coder,depth=depth,rgb=rgb,alpha=rgb,grid=(3,2),micro=5 if depth==16 else 4)
                file=name+'.config';(D/file).write_bytes(data)
                cases.append(dict(file=file,coder=coder,depth=depth,rgb=rgb,alpha=rgb,grid=[3,2],micro=5 if depth==16 else 4,initial=initial,transitions=transitions if coder==2 else TABLE,sha256=hashlib.sha256(data).hexdigest()))
    for name,kwargs,error in [
        ('version',dict(version=2),'unsupported FFV1 capability: FFV1 configuration requires version 3'),
        ('experimental',dict(micro=3),'unsupported FFV1 capability: experimental FFV1 v3 micro-version'),
        ('set-count',dict(count=9),'invalid FFV1 quantizer set count'),
        ('rgb-chroma',dict(rgb=True,chroma=False),'invalid FFV1 RGB chroma configuration'),
        ('shift',dict(sx=5),'invalid FFV1 chroma shift'),
        ('ec',dict(ec=2),'unsupported FFV1 capability: unsupported FFV1 error correction mode'),
        ('intra',dict(intra=2),'unsupported FFV1 capability: unsupported FFV1 intra constraint'),
    ]:
        data=config(**kwargs)[0];file=f'ffv1-v3-config-invalid-{name}.config';(D/file).write_bytes(data);invalid.append(dict(file=file,error=error))
    zero=config(zero_state=True)[0];zero_file='ffv1-v3-config-zero-probability.config';(D/zero_file).write_bytes(zero)
    old=D/'ffv1-v3-config-invalid-zero-state.config'
    if old.exists() and old.read_bytes()==zero:old.unlink()
    data=bytearray(config()[0]);data[-1]^=1;file='ffv1-v3-config-invalid-crc.config';(D/file).write_bytes(data);invalid.append(dict(file=file,error='FFV1 configuration CRC mismatch'))
    for ec in [0,1]:
        private=config(ec=ec,grid=(3,2))[0];file=f'ffv1-v3-slices-ec{ec}.config';(D/file).write_bytes(private)
        payloads=[bytes([i,19+i,200-i]) for i in range(6)];packet=b''.join(sliced(p,ec) for p in payloads)
        packet_file=f'ffv1-v3-slices-ec{ec}.packet';(D/packet_file).write_bytes(packet)
        invalid_frames=[]
        for name,bad,error in [
            ('size',b'\xff\xff\xff' if not ec else sliced(b'\0\0',True)[:-8]+b'\xff\xff\xff'+bytes(5),'invalid FFV1 slice size'),
            ('truncated-footer',b'\0','truncated FFV1 slice footer'),
            ('empty',b'','FFV1 frame contains no slices'),
            ('count',packet+sliced(b'\0\0',ec),'FFV1 slice count exceeds raster'),
        ]:
            path=f'ffv1-v3-slices-ec{ec}-invalid-{name}.packet';(D/path).write_bytes(bad);invalid_frames.append(dict(file=path,error=error))
        if ec:
            bad=bytearray(packet);bad[0]^=1
            for name,data,error in [('crc',bad,'FFV1 slice CRC mismatch'),('status',sliced(b'\0\0',True,1),'FFV1 slice reports damaged content')]:
                path=f'ffv1-v3-slices-ec1-invalid-{name}.packet';(D/path).write_bytes(data);invalid_frames.append(dict(file=path,error=error))
        slices.append(dict(config=file,packet=packet_file,payloads=[p.hex() for p in payloads],invalid=invalid_frames))
    # A real all-zero 4x2 YUV444 v3 frame. Slice framing tests above intentionally
    # use arbitrary payloads and are not claimed as pixel decoding acceptance.
    private,initial,_=config(count=1);r=Range();r.bit([128],0,True);h=[128]*32
    for value in [0,0,0,0,0,0,3,1,1]:r.integer(h,value)
    models=[initial[0][0].copy(),initial[0][0].copy()]
    for p in range(3):
        for _ in range(8):r.integer(models[int(p!=0)],0,True)
    packet=sliced(r.finish(),True)
    (D/'ffv1-v3-zero-4x2.mkv').write_bytes(movie(private,packet))
    (D/'ffv1-v3-zero-4x2.yuv').write_bytes(bytes(24))
    for entry in invalid:
        entry['sha256']=hashlib.sha256((D/entry['file']).read_bytes()).hexdigest()
    for entry in slices:
        entry['sha256']=hashlib.sha256((D/entry['packet']).read_bytes()).hexdigest()
        for bad in entry['invalid']:bad['sha256']=hashlib.sha256((D/bad['file']).read_bytes()).hexdigest()
    (D/'ffv1-v3-configuration.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,slices=slices,zero_probability=zero_file,reproducer='ffv1-v3-zero-4x2.mkv',reference='ffv1-v3-zero-4x2.yuv',reproducer_sha256=hashlib.sha256((D/'ffv1-v3-zero-4x2.mkv').read_bytes()).hexdigest(),provenance='Original RFC 9043 range records, initial-state delta traces, polynomial CRC and original all-zero YUV444 frame; no private media, external codec or network. Configuration and framing acceptance is separate from v3 pixel decoding.'),indent=2)+'\n')
if __name__=='__main__':main()
