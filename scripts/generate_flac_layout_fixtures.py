#!/usr/bin/env python3
"""FLAC comment-mask regressions from authored frames, no external codec tools."""
import json,hashlib,struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame as picture
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';OUT=ROOT/'flac-layout'
def wrap(raw,channels):
 offset=46+int.from_bytes(raw[43:46],'big')
 video=ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(8000000).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
 audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_FLAC')+ebml('63a2',raw[:offset])+ebml('e1',ebml('b5',struct.pack('>d',8000))+ebml('9f',bytes([channels]))))
 head=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))
 blocks=ebml('e7',b'\0')+ebml('a3',b'\x81\0\0\x80'+picture(8,'420',16,16))+ebml('a3',b'\x82\0\0\x80'+raw[offset:])
 return head+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))

def generate():
 files={};cases=[]
 for name,source,channels,mask in [('mono-lfe','16-0-verbatim-0',1,8),('mono-unassigned','16-0-verbatim-0',1,0),('stereo-unassigned','16-1-verbatim-0',2,0),('stereo-rear','16-1-verbatim-0',2,0x30),('four-fc-bc','extended-lpc-4-1-0-0',4,0x107),('four-height','extended-lpc-4-1-0-0',4,0x5003),('four-centers','extended-lpc-4-1-0-0',4,0x12104),('four-unassigned','extended-lpc-4-1-0-0',4,0),('four-stereo-prefix','extended-lpc-4-1-0-0',4,3)]+[(f'speaker-{speaker}','16-0-verbatim-0',1,1<<speaker)for speaker in range(18)]:
  original=(ROOT/'flac-native'/f'{source}.flac').read_bytes();field=f'waveformatextensible_channel_mask=0X{mask:08X}'.encode();comment=struct.pack('<I',4)+b'fvid'+struct.pack('<I',1)+struct.pack('<I',len(field))+field
  raw=original[:4]+bytes([0])+original[5:42]+bytes([132])+len(comment).to_bytes(3,'big')+comment+original[42:]
  files[name+'.flac']=raw;files[name+'.mkv']=wrap(raw,channels);cases.append(dict(name=name,source=source,channels=channels,mask=mask))
 files['manifest.json']=(json.dumps(dict(cases=cases,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,v in generate().items():(OUT/n).write_bytes(v)
