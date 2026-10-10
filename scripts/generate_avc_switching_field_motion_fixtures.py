#!/usr/bin/env python3
"""Owned field SP fractional-motion matrix, no external decoder or encoder."""
import argparse
import json
import subprocess
import tempfile
from pathlib import Path
from avc_fixture_mp4 import annexb
from generate_avc_switching_field_fixtures import (
    DEST, configuration, pcm, switching, planes, reconstruct, weave, mux,
)


def compensate(plane, width, height, mv, chroma=False):
    """Scalar H.264 8.4.2.2 interpolation on compact field samples.

    Six-tap luma half samples keep unrounded intermediates for the diagonal.
    Chroma uses eighth-sample bilinear interpolation; reference parity agrees.
    """
    def pixel(x, y):
        return plane[max(0, min(height-1, y))*width+max(0, min(width-1, x))]
    taps = [1, -5, 20, 20, -5, 1]
    clip = lambda x: max(0, min(255, x))
    result = []
    for row in range(height):
        for col in range(width):
            scale = 8 if chroma else 4
            x, fx = divmod(col*scale+mv[0], scale)
            y, fy = divmod(row*scale+mv[1], scale)
            if chroma:
                result.append(((8-fx)*(8-fy)*pixel(x,y)+fx*(8-fy)*pixel(x+1,y)
                               +(8-fx)*fy*pixel(x,y+1)+fx*fy*pixel(x+1,y+1)+32)//64)
                continue
            horizontal = lambda yy: sum(t*pixel(x+i-2,yy) for i,t in enumerate(taps))
            vertical = lambda xx: sum(t*pixel(xx,y+i-2) for i,t in enumerate(taps))
            b=clip((horizontal(y)+16)//32);h=clip((vertical(x)+16)//32)
            j=clip((sum(t*horizontal(y+i-2) for i,t in enumerate(taps))+512)//1024)
            m=clip((vertical(x+1)+16)//32);s=clip((horizontal(y+1)+16)//32)
            avg=lambda a,b:(a+b+1)//2
            grid=[[pixel(x,y),avg(pixel(x,y),b),b,avg(b,pixel(x+1,y))],
                  [avg(pixel(x,y),h),avg(b,h),avg(b,j),avg(b,m)],
                  [h,avg(h,j),j,avg(j,m)],
                  [avg(h,pixel(x,y+1)),avg(h,s),avg(j,s),avg(m,s)]]
            result.append(grid[fy][fx])
    return result


def verify_jm(cases, decoder):
    """Optional luma-only syntax/interpolation cross-check, outside test execution."""
    with tempfile.TemporaryDirectory(prefix='fvid-sp-field-jm-') as tmp:
        directory=Path(tmp)
        (directory/'decoder.cfg').write_text('')
        for case in cases:
            frames=[(i,i==0,bytes.fromhex(p)) for i,p in enumerate(case['packets'])]
            (directory/'stream.264').write_bytes(annexb(bytes.fromhex(case['configuration']),frames))
            with (directory/'jm.log').open('w') as log:
                subprocess.run([str(decoder.resolve()),'-d',str(directory/'decoder.cfg'),
                    '-p',f'InputFile={directory}/stream.264',
                    '-p',f'OutputFile={directory}/decoded.yuv',
                    '-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],
                    cwd=directory,stdout=log,stderr=subprocess.STDOUT,check=True)
            actual=(directory/'decoded.yuv').read_bytes()
            expected=(DEST/case['reference']).read_bytes()
            assert len(actual)==len(expected),case['file']
            for start in range(0,len(expected),1536):
                assert actual[start:start+1024]==expected[start:start+1024],case['file']
    print(f'JM syntax and exact luma cross-check: {len(cases)} streams')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,help='Optional explicit luma cross-check only')
    args=parser.parse_args()
    config=configuration(8,max_refs=3);cases=[]
    # All sixteen fractional phases, with positive/negative integer displacements.
    vectors=[(fx-4 if fy%2 else fx+4,fy-4 if fx%2 else fy+4)
             for fy in range(4) for fx in range(4)]
    for kind in ['primary','secondary']:
        for coded in [False,True]:
            for reverse in [False,True]:
                for index,mv in enumerate(vectors):
                    qs=[0,26,51][index%3];order=[True,False] if reverse else [False,True]
                    fields={b:planes(b) for b in order};frames=[];packets=[]
                    def append(nals,idr):
                        packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                        frames.append((len(frames),idr,packet));packets.append(packet.hex())
                    for i,bottom in enumerate(order):append([pcm(bottom,i==0,reverse)],i==0)
                    raw=bytearray(weave(fields))
                    for bottom in order:
                        append([switching(bottom,1,qs,kind,coded,False,reverse,mb,mv)
                                for mb in range(2)],False)
                        predicted=[compensate(p,w,h,mv,c>0) for c,(p,w,h) in
                                   enumerate(zip(fields[bottom],[32,16,16],[16,8,8]))]
                        fields[bottom]=reconstruct(predicted,qs,kind,coded)
                    raw.extend(weave(fields))
                    name=f'avc-switching-field-motion-{kind}-'+('signed' if coded else 'zero')+'-'+('bottom-first' if reverse else 'top-first')+f'-x{mv[0]}-y{mv[1]}'
                    (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                    (DEST/(name+'-reference.yuv')).write_bytes(raw)
                    cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',
                                      configuration=config.hex(),packets=packets,frame_count=2,
                                      kind=kind,coded=coded,reverse=reverse,motion=mv,qs=qs))
    (DEST/'avc-switching-field-motion.json').write_text(json.dumps(dict(cases=cases,
        provenance='Original PCM field pair then primary/secondary SP pair; two slices per compact field. All 16 quarter-sample phases with signed displacements, zero/signed residual and both arrival parity orders. Independent scalar six-tap luma and eighth-sample chroma prediction before normative switching matrix. Same reference parity, deblocking disabled. Offline generator; no private media or external codecs.'),indent=2)+'\n')

    if args.jm_decoder:verify_jm(cases,args.jm_decoder)


if __name__=='__main__':main()
