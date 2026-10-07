#!/usr/bin/env python3
"""Owned AV1 separate headers/tile groups; no private media or FFmpeg."""
import argparse,hashlib,itertools,json,subprocess,tempfile,struct
from pathlib import Path
from generate_av1_scaled_reference_samples import CASES,stream
from generate_av1_show_existing_samples import webm,obu

def mp4(data,sequence,size):
 from generate_audio_resample_window_fixture import atom,word
 ftyp=atom(b'ftyp',b'isom'+word(0)+b'isomav01');mdat=atom(b'mdat',data)
 entry=bytearray(78);entry[6:8]=(1).to_bytes(2,'big');entry[24:28]=struct.pack('>HH',*size);entry[40:42]=(1).to_bytes(2,'big');entry[74:76]=(24).to_bytes(2,'big')
 stsd=atom(b'stsd',word(0,1)+atom(b'av01',entry+atom(b'av1C',bytes([0x81,0,0,0])+sequence)))
 stbl=atom(b'stbl',stsd+atom(b'stts',word(0,1,1,1))+atom(b'stsz',word(0,0,1,len(data)))+atom(b'stsc',word(0,1,1,1,1))+atom(b'stco',word(0,1,len(ftyp)+8))+atom(b'stss',word(0,1,1)))
 dinf=atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))
 tkhd=bytearray(84);tkhd[12:16]=word(1);tkhd[20:24]=word(1);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(0x40000000);tkhd[76:84]=word(size[0]<<16,size[1]<<16)
 mdhd=bytearray(24);mdhd[12:20]=word(50,1);mdhd[20:22]=(21956).to_bytes(2,'big')
 mdia=atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'vide'+bytes(12))+atom(b'minf',atom(b'vmhd',word(1)+bytes(8))+dinf+stbl))
 mvhd=bytearray(100);mvhd[12:20]=word(50,1)
 return ftyp+mdat+atom(b'moov',atom(b'mvhd',mvhd)+atom(b'trak',atom(b'tkhd',tkhd)+mdia))

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--writer',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);p.add_argument('--encoder',type=Path,required=True);a=p.parse_args()
 root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
 for case,logical,adaptive,redundant in itertools.product(range(12),[1,7],[False,True],[False,True]):
  ref,size=CASES[case];base=1 if case%2==0 else 64
  data,maps=stream(a.writer,ref,size,logical,base,1,False,adaptive,64,1,case%4,1,separate=True,redundant=redundant)
  name=f'av1-separate-frame-case{case}-ref{logical}-adapt{int(adaptive)}-redundant{int(redundant)}'
  file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
  (root/file).write_bytes(data);container=webm(data,size);(root/wrapped).write_bytes(container)
  subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
  pixels=(root/expected).read_bytes();assert len(pixels)==3*size[0]*size[1]
  records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(container).hexdigest(),reference_size=ref,current_size=size,case=case,logical=logical,adaptive=adaptive,redundant=redundant,maps=maps))
 refusals=[]
 def pieces(data):
  at=0;result=[]
  while at<len(data):
   kind=data[at]>>3;at+=1;size=0;shift=0
   while True:
    byte=data[at];at+=1;size|=(byte&127)<<shift;shift+=7
    if not byte&128:break
   payload=data[at:at+size];at+=size;result.append((kind,payload))
  return result
 initial=pieces((root/records[0]['file']).read_bytes());seq=obu(1,initial[0][1]);header=initial[1][1];tile=initial[2][1]
 def refuse(name,data,error):
  file='av1-separate-frame-invalid-'+name+'.obu';(root/file).write_bytes(data)
  refusals.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),error=error))
 refuse('orphan-group',seq+obu(4,tile),'AV1 tile group without frame header')
 refuse('duplicate-header',seq+obu(3,header)+obu(3,header),'AV1 new frame or delimiter before tile completion')
 refuse('incomplete-delimiter',seq+obu(3,header)+obu(2,b''),'AV1 new frame or delimiter before tile completion')
 refuse('orphan-redundant',seq+obu(7,header),'AV1 redundant header without frame header')
 bad_header=bytes([header[0]^16])+header[1:]
 refuse('redundant-mismatch',seq+obu(3,header)+obu(7,bad_header),'AV1 redundant frame header mismatch')
 bad=bytearray(header);bad[-1]&=~(bad[-1]&-bad[-1])
 refuse('missing-trailing',seq+obu(3,bytes(bad)),'missing AV1 frame header trailing one bit')
 refuse('bad-padding',seq+obu(3,header+b'\x01'),'nonzero AV1 frame header trailing padding')
 # Extended tile OBU with a different temporal_id.
 raw=obu(4,tile);refuse('layer-mismatch',seq+obu(3,header)+bytes([raw[0]|4,32])+raw[1:],'AV1 tile group layer mismatch')
 multitile=[];eof_refusals=[]
 for width,height,columns,rows,groups in [(128,128,1,1,2),(128,128,1,1,4),(256,128,2,1,4),(256,128,2,1,8)]:
  pixels=bytes((128+(x//16-y//16)*3)%256 for y in range(height) for x in range(width))+bytes([112])*(width*height//4)+bytes([144])*(width*height//4)
  name=f'av1-separate-tiles-w{width}-h{height}-groups{groups}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
  with tempfile.TemporaryDirectory(prefix='fvid-owned-tiles-') as tmp:
   input=Path(tmp)/'owned.y4m';input.write_bytes(f'YUV4MPEG2 W{width} H{height} F50:1 Ip A1:1 C420jpeg\nFRAME\n'.encode()+pixels)
   subprocess.run([str(a.encoder),'--obu','--cpu-used=8','--passes=1','--limit=1','--lossless=1',f'--tile-columns={columns}',f'--tile-rows={rows}',f'--num-tile-groups={groups}','--enable-palette=0','--enable-intrabc=0','--enable-cdef=0','--enable-restoration=0','--enable-ref-frame-mvs=0',f'--output={root/file}',str(input)],check=True)
  data=(root/file).read_bytes();parts=pieces(data);assert sum(k==4 for k,p in parts)==groups
  subprocess.run([str(a.oracle),str(root/file),'1',str(root/expected),'whole-packet'],check=True)
  assert (root/expected).read_bytes()==pixels
  container=webm(data,(width,height));(root/wrapped).write_bytes(container)
  sequence_obu=next(obu(k,payload) for k,payload in parts if k==1)
  mp4_file=name+'.mp4';mp4_data=mp4(data,sequence_obu,(width,height));(root/mp4_file).write_bytes(mp4_data)
  multitile.append(dict(mp4=mp4_file,mp4_sha256=hashlib.sha256(mp4_data).hexdigest(),file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(container).hexdigest(),size=[width,height],groups=groups,tiles=1<<(columns+rows)))
  prefix=[]
  for k,payload in parts:
   if k==4:break
   prefix.append(obu(k,payload))
  first_group=next(obu(k,payload) for k,payload in parts if k==4)
  incomplete=b''.join(prefix)+first_group
  partial_file=name+'-incomplete.obu';partial_webm=name+'-incomplete.webm'
  (root/partial_file).write_bytes(incomplete);partial_container=webm(incomplete,(width,height));(root/partial_webm).write_bytes(partial_container)
  partial_mp4=name+'-incomplete.mp4';partial_mp4_data=mp4(incomplete,sequence_obu,(width,height));(root/partial_mp4).write_bytes(partial_mp4_data)
  eof_refusals.append(dict(mp4=partial_mp4,mp4_sha256=hashlib.sha256(partial_mp4_data).hexdigest(),file=partial_file,sha256=hashlib.sha256(incomplete).hexdigest(),webm=partial_webm,webm_sha256=hashlib.sha256(partial_container).hexdigest(),error='AV1 end of stream before tile completion'))
  if width==128 and groups==4:
   before=[];tileparts=[]
   for k,payload in parts:
    if k==4:tileparts.append(obu(k,payload))
    else:before.append(obu(k,payload))
   prefix=b''.join(before)
   refuse('reordered-tiles',prefix+tileparts[1]+tileparts[0]+b''.join(tileparts[2:]),'AV1 tile groups out of order')
   refuse('repeated-tiles',prefix+tileparts[0]+tileparts[0]+b''.join(tileparts[1:]),'AV1 tile groups out of order')
 (root/'av1-separate-frame-generated.json').write_text(json.dumps(dict(fixtures=records,refusals=refusals,multitile=multitile,eof_refusals=eof_refusals),indent=2)+'\n')
if __name__=='__main__':main()
