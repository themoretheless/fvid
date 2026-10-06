#!/usr/bin/env python3
"""Owned PAFF frame CAVLC intra slices spanning both raster macroblock rows."""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_field_pcm_samples import configuration,Writer
from avc_fixture_mp4 import mux,annexb

def frame_slice(address,mode,deblock,joined=False):
 b=Writer();b.ue(address);b.ue(2);b.ue(0);b.u(0,4);b.u(0);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(24);b.ue(deblock)
 if deblock!=1:b.se(6);b.se(6)
 for address in range(4) if joined else [address]:
  if mode.startswith('i4'):
   b.ue(0)
   for _ in range(16):b.u(1)
   b.ue(0);b.ue(2);b.se(0)
   for i in range(16):
    if mode=='i4-zero':b.u(1)
    else:b.u(1,2);b.u(0 if mode=='i4-bias' else (i+address)%2);b.u(3,3)
  else:
   b.ue(3);b.ue(0);b.se(0)
   if mode=='i16-zero':b.u(1)
   else:b.u(1,2);b.u(int(mode=='i16-negative'));b.u(1)
 return b.nal(0x65)

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);p.add_argument('--joined',action='store_true');args=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
 with tempfile.TemporaryDirectory(prefix='fvid-paff-intra-') as tmp:
  d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
  for depth,mode,deblock,aso in itertools.product([8,10],['i4-zero','i4-ac','i4-bias','i16-zero','i16-positive','i16-negative'],[0,1,2],[False] if args.joined else [False,True]):
   config=configuration(depth);ns=[frame_slice(0,mode,deblock,True)] if args.joined else [frame_slice(a,mode,deblock) for a in ([3,1,2,0] if aso else [0,1,2,3])];frames=[(0,True,b''.join(len(n).to_bytes(4,'big')+n for n in ns))];prefix='avc-paff-intra-'+('joined-' if args.joined else '');name=f'{prefix}{depth}bit-{mode}-filter{deblock}'+('-aso' if aso else '')
   coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames));subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
   pixels=oracle.read_bytes();assert len(pixels)==1536*(2 if depth==10 else 1),(name,len(pixels));data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels);records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
 (root/(prefix+'generated.json')).write_text(json.dumps(dict(generator='owned PAFF CAVLC intra raster slices',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
