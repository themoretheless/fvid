#!/usr/bin/env python3
"""Authored FLAC/neutral VP9 Matroska companions, no external tools."""
from pathlib import Path
import hashlib,json,struct
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame as picture
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'flac-matroska'
def generate():
 files={};cases=[]
 for case in json.loads((ROOT/'flac-native/manifest.json').read_text())['cases']:
  name=case['name'];raw=(ROOT/'flac-native'/f'{name}.flac').read_bytes();info=raw[8:42];packet=raw[42:];word=int.from_bytes(info[10:18],'big');rate=word>>44;channels=((word>>41)&7)+1;bits=((word>>36)&31)+1
  head=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))
  video=ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(8000000).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
  audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_FLAC')+ebml('63a2',raw[:42])+ebml('e1',ebml('b5',struct.pack('>d',rate))+ebml('9f',bytes([channels]))))
  blocks=ebml('e7',b'\0')+ebml('a3',b'\x81\0\0\x80'+picture(8,'420',16,16))+ebml('a3',b'\x82\0\0\x80'+packet)
  files[name+'.mkv']=head+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
  shifted=ebml('e7',b'\0')+ebml('a3',b'\x81\0\0\x80'+picture(8,'420',16,16))+ebml('a3',b'\x82'+struct.pack('>h',50)+b'\x80'+packet)
  files[name+'-offset.mkv']=head+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',video+audio)+ebml('1f43b675',shifted))
  cases.append(dict(name=name,rate=rate,channels=channels,bits=bits))
 files['manifest.json']=(json.dumps(dict(cases=cases,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,v in generate().items():(OUT/n).write_bytes(v)
 print(len(generate())-1,'authored FLAC Matroska companions')
