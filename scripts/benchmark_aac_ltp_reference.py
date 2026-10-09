#!/usr/bin/env python3
"""Explicit AAC LTP FFmpeg reference benchmark; ordinary tests never invoke it."""
import argparse,subprocess,time,json,hashlib
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--ffmpeg',default='ffmpeg');p.add_argument('--save-reference',action='store_true');a=p.parse_args()
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
version=subprocess.check_output([a.ffmpeg,'-version'],text=True).splitlines()[0];rows=[]
for name in ('inactive','active'):
 source=root/f'aac-ltp-{name}-synthetic.mp4';start=time.perf_counter()
 data=subprocess.check_output([a.ffmpeg,'-nostdin','-v','error','-i',str(source),'-map','0:a:0','-f','f32le','-c:a','pcm_f32le','pipe:1'])
 assert len(data)==12288*4
 row=dict(name=name,source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),reference_sha256=hashlib.sha256(data).hexdigest(),samples=len(data)//4,elapsed_ms=(time.perf_counter()-start)*1000)
 rows.append(row)
 if a.save_reference:(root/f'aac-ltp-{name}-external-reference.f32le').write_bytes(data)
 print(name,len(data)//4,'samples',round(row['elapsed_ms'],3),'ms including process startup')
if a.save_reference:(root/'aac-ltp-external-reference.json').write_text(json.dumps(dict(version=version,cases=rows,scope='Authored mono 24k/1024 long-window inactive and active ordinary LTP videos; explicit external-reference benchmark, not full-profile certification.'),indent=2)+'\n')
