#!/usr/bin/env python3
"""Original PS MP4 silence/repeated-range and incomplete-source regressions."""
import json,struct
from generate_he_aac_packet_fixtures import DEST,boxes,box

def edited(source,name,edits):
 raw=(DEST/source).read_bytes();outer=list(boxes(raw));assert [k for k,_ in outer].index(b'moov')>[k for k,_ in outer].index(b'mdat')
 out=[]
 for kind,body in outer:
  if kind==b'moov':
   children=list(boxes(body));header=next(p for k,p in children if k==b'mvhd');scale=struct.unpack_from('>I',header,12)[0]
   # Own requested sample windows use exact movie-clock durations.
   entries=[(samples*scale//48000,start) for samples,start in edits];assert all(d*48000==samples*scale for (d,_),(samples,_) in zip(entries,edits))
   elst=box(b'elst',bytes(4)+struct.pack('>I',len(entries))+b''.join(struct.pack('>IiI',d,start,65536) for d,start in entries))
   rebuilt=[]
   for child,p in children:
    if child==b'trak' and any(k==b'mdia' and any(a==b'hdlr' and b[8:12]==b'soun' for a,b in boxes(q)) for k,q in boxes(p)):
     p=b''.join(box(k,q) for k,q in boxes(p) if k!=b'edts')+box(b'edts',elst)
    rebuilt.append(box(child,p))
   body=b''.join(rebuilt)
  out.append(box(kind,body))
 (DEST/name).write_bytes(b''.join(out))

def main():
 cases=[]
 for size in [960,1024]:
  source=f'he-aac-sbr-ps-960-retain-synthetic.mp4' if size==960 else 'he-aac-ps-matrix-retain-synthetic.mp4'
  name=f'he-aac-ps-worker-repeat-{size}-synthetic.mp4';edited(source,name,[(1600,-1),(3200,720),(3200,720),(1600,-1)])
  gap=f'he-aac-ps-worker-source-gap-{size}-synthetic.mp4';edited(source,gap,[(6400,720),(1600,-1)])
  cases.append(dict(file=name,source=source,source_start=720,source_samples=3200,silence=1600,presentation_samples=9600,gap_file=gap,gap_error='AAC edit extends beyond source packets'))
 (DEST/'aac-ps-worker-oracles.json').write_text(json.dumps(dict(kind='own silence and repeated PS source ranges; source gap remains an explicit error',cases=cases),indent=2)+'\n')
 print('Four original edited PS worker MP4s')
if __name__=='__main__':main()
