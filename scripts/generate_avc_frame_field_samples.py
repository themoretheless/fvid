#!/usr/bin/env python3
"""Owned frame PCM references followed by explicit separate-field prediction."""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_field_pcm_samples import Writer,pcm_samples
from generate_avc_mbaff_direct_samples import config
from generate_avc_field_pcm_samples import configuration as paff_config
from generate_avc_field_gap_samples import prediction
from avc_fixture_mp4 import mux,annexb

def frame(depth,long_term=False,paff=False):
 b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.u(0);b.ue(0);b.u(0,4);b.u(0);b.u(int(long_term));b.se(0);b.ue(1)
 for address in range(4):
  if not paff and address%2==0:b.u(0)
  b.ue(25);b.align();b.bits.extend(pcm_samples(depth,address,address))
 return b.nal(0x65)

def skipped_frame():
 b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(0);b.u(4,4);b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1);b.ue(4);return b.nal(0x41)

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);p.add_argument('--paff',action='store_true');p.add_argument('--long-term',action='store_true');p.add_argument('--inter-source',action='store_true');args=p.parse_args();assert not (args.inter_source and (args.paff or args.long_term));root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
 with tempfile.TemporaryDirectory(prefix='fvid-frame-field-') as tmp:
  d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
  for depth,reverse,skip,deblock in ([(8,False,False,0)] if args.inter_source else itertools.product([8,10],[False,True],[False,True],[0,1,2])):
   configuration=paff_config(depth) if args.paff else config(depth,False,width_mbs=2);n=frame(depth,long_term=args.long_term,paff=args.paff);frames=[(0,True,len(n).to_bytes(4,'big')+n)]
   if args.inter_source:
    n=skipped_frame();frames.append((2,False,len(n).to_bytes(4,'big')+n))
   for i,bottom in enumerate([True,False] if reverse else [False,True]):
    ns=[prediction(bottom,a,2 if args.inter_source else 1,6+i if args.inter_source else 2+i,skip,deblock,long_target=args.long_term) for a in [0,1]];frames.append((4+i if args.inter_source else 2+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
   prefix=('avc-paff-frame-to-field-' if args.paff else 'avc-frame-to-field-')+('long-' if args.long_term else 'inter-' if args.inter_source else '');name=f'{prefix}{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-skip' if skip else '-coded')+f'-filter{deblock}'
   coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(configuration,frames));subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
   pixels=oracle.read_bytes();assert len(pixels)==(4608 if args.inter_source else 3072)*(2 if depth==10 else 1),(name,len(pixels));data=mux(configuration,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels);records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
 (root/(prefix+'generated.json')).write_text(json.dumps(dict(generator='owned PCM frame to separate P fields',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
