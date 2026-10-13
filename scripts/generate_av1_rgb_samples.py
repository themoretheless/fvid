#!/usr/bin/env python3
"""Owned RGB AV1: reinterpret saved 4:4:4 samples as G/B/R, no color conversion."""
import hashlib,json,struct
from generate_av1_configuration_samples import D,units
from generate_audio_resample_window_fixture import atom
from generate_av1_show_existing_samples import obu
from generate_av1_spatial_container_samples import mp4,webm

def rgb_sequence(raw,depth):
    assert raw[0]==0x0a
    at=1
    while raw[at]&128:at+=1
    at+=1
    bits=[(v>>shift)&1 for v in raw[at:] for shift in range(7,-1,-1)]
    trailing=max(i for i,bit in enumerate(bits) if bit)
    count=8 if depth==12 else 5
    color=bits[trailing-count:trailing]
    # Existing owned YUV 4:4:4 headers: unspecified description, no grain.
    expected=([1,1,0,0,0,0,0,0] if depth==12 else [int(depth>8),0,0,0,0])
    assert color==expected,(depth,color)
    prefix=color[:3] if depth==12 else color[:1]
    values=prefix+[1]+[(v>>shift)&1 for v in [1,13,0] for shift in range(7,-1,-1)]+color[-2:]+[1]
    bits=bits[:trailing-count]+values
    bits += [0]*((-len(bits))%8)
    payload=bytes(sum(bits[i+j]<<(7-j) for j in range(8)) for i in range(0,len(bits),8))
    return obu(1,payload)

def rotated_mp4(data):
    output=b'';at=0
    while at<len(data):
        size=int.from_bytes(data[at:at+4],'big');kind=data[at+4:at+8];body=data[at+8:at+size]
        assert size>=8 and at+size<=len(data)
        if kind in [b'moov',b'trak']:body=rotated_mp4(body)
        elif kind==b'tkhd':
            body=bytearray(body);body[40:76]=struct.pack('>9i',0,65536,0,-65536,0,0,0,0,1<<30)
        output+=atom(kind,body);at+=size
    return output

def main():
    cases=[]
    for depth in [8,10,12]:
        seed=f'av1-chroma-layout444-depth{depth}-lossless1'
        parts=list(units((D/(seed+'.obu')).read_bytes()))
        packets=[];pending=b'';sequence=None
        for kind,piece in parts:
            if kind==2 and pending:packets.append(pending);pending=b''
            if kind==1:piece=rgb_sequence(piece,depth);sequence=piece
            pending+=piece
        if pending:packets.append(pending)
        assert len(packets)==2 and sequence
        raw=(D/(seed+'.yuv')).read_bytes();bytes_per_sample=1 if depth==8 else 2;plane=64*48*bytes_per_sample
        expected=bytearray()
        for at in range(0,len(raw),plane*3):
            for index in range(0,plane,bytes_per_sample):
                for component in [2,0,1]:
                    begin=at+component*plane+index;expected+=raw[begin:begin+bytes_per_sample]
        name=f'av1-rgb-depth{depth}'
        reference=name+'.rgb';(D/reference).write_bytes(expected)
        files={}
        for extension,data in [('obu',b''.join(packets)),('mp4',mp4(packets,sequence,(64,48))),('webm',webm(packets,sequence,(64,48))),('rotated',rotated_mp4(mp4(packets,sequence,(64,48))))]:
            file=name+('-rotate90.mp4' if extension=='rotated' else '.'+extension);(D/file).write_bytes(data)
            files[extension]=dict(file=file,sha256=hashlib.sha256(data).hexdigest())
        cases.append(dict(depth=depth,width=64,height=48,frames=2,files=files,reference=reference,
            reference_sha256=hashlib.sha256(expected).hexdigest(),seed=seed))
    (D/'av1-rgb.json').write_text(json.dumps(dict(cases=cases,provenance='Owned saved 4:4:4 coding and independent decoded samples. Authored identity-matrix color_config (1/13/0), preserving coding flags. G/B/R samples interleaved to R/G/B without conversion. No private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
