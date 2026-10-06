#!/usr/bin/env python3
"""Owned MMCO5 field reset followed by a frame-number gap and reordered B fields."""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_field_b_longterm_samples import prediction,cavlc_config,cabac_config,intra,field
from avc_fixture_mp4 import mux,annexb

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);p.add_argument('--poc-type',type=int,choices=[0,1,2],default=0);args=p.parse_args()
 root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
 with tempfile.TemporaryDirectory(prefix='fvid-reset-gap-') as tmp:
  d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
  for depth,reverse,init,spatial,skip,deblock,initial_long in itertools.product([8,10],[False,True],[None,0,1,2],[False,True],[False,True],[0,1,2],[False,True]):
   order=[True,False] if reverse else [False,True];frames=[]
   for stage,number in enumerate([0,15,0]):
    for i,bottom in enumerate(order):
     pts=2*stage+i;poc=(8 if i==0 else 1) if stage==2 else stage*4+i
     kw=dict(long_term=initial_long and stage==0,reset=stage==2 and i==0,poc_type=args.poc_type,poc_delta=i if args.poc_type==1 else 0)
     ns=[intra(bottom,depth,number,stage==0 and i==0,poc,**kw)] if init is None else [field(bottom,poc,a,'positive' if (a==0)^bottom else 'negative',deblock,biased=stage>0,frame_num=number,**kw) for a in [0,1]]
     frames.append((pts,stage==0 and i==0,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
   for is_b in [False,True]:
    for i,bottom in enumerate(order):
     index=(6 if is_b else 8)+i;n=prediction(bottom,index,spatial,'source',False,skip,deblock,init,is_b,poc_type=args.poc_type)
     frames.append((index,False,len(n).to_bytes(4,'big')+n))
   config=(cavlc_config if init is None else cabac_config)(depth,max_refs=5,gaps=True,poc_type=args.poc_type)
   prefix='avc-field-b-reset-gap-'+(f'poc{args.poc_type}-' if args.poc_type else '');name=f'{prefix}{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-spatial' if spatial else '-temporal')+('-skip' if skip else '-coded')+('-cavlc' if init is None else f'-cabac-init{init}')+f'-filter{deblock}-initial'+('long' if initial_long else 'short')
   coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
   subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
   pixels=oracle.read_bytes();assert len(pixels)==5*1536*(2 if depth==10 else 1),(name,len(pixels))
   data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels)
   records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
 (root/(prefix+'generated.json')).write_text(json.dumps(dict(generator='owned MMCO5 reset gap B fields',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
