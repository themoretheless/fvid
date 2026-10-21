#!/usr/bin/env python3
"""Author fixed-block AVI audio samples grouped into larger data chunks."""
import hashlib,json,struct
from pathlib import Path
from generate_avi_audio_eof_regressions import generate as owned,chunk,listing
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-adpcm-grouped'
def generate():
    original=owned();files={'video.mkv':original['video.mkv']};cases=[];controls=[];malformed=[]
    for codec,per_block in [('ima',9),('ms',4)]:
      for channels in [1,2]:
        key=codec if channels==1 else codec+'-stereo';base=original[codec+'.avi'];block_size=8*channels
        at=base.index(b'movi')-8;end=at+8+struct.unpack_from('<I',base,at+4)[0];cursor=at+12;blocks=[];video=None
        while cursor<end:
            tag=base[cursor:cursor+4];size=struct.unpack_from('<I',base,cursor+4)[0];payload=base[cursor+8:cursor+8+size]
            if tag==b'00dc':video=chunk(tag,payload)
            elif tag==b'01wb':assert size==8;blocks.append(payload)
            cursor+=8+size+(size&1)
        assert len(blocks)==3 and video is not None
        prefix_template=bytearray(base[:at]);audio_header=base.index(b'auds');assert struct.unpack_from('<I',base,audio_header+44)[0]==8
        expected=original[codec+'.f32le']
        if channels==2:
            blocks=[];expected=b''
            for predictor in [1000,2000,3000]:
                data=(struct.pack('<hBBhBB',predictor,0,0,-predictor,0,0)+bytes(8) if codec=='ima' else struct.pack('<BBHHhhhh',0,0,16,16,predictor,-predictor,predictor,-predictor)+bytes(2))
                assert len(data)==block_size;blocks.append(data);expected+=struct.pack('<ff',predictor/32768.,-predictor/32768.)*per_block
            fmt=prefix_template.index(b'strf',audio_header)+8
            struct.pack_into('<H',prefix_template,fmt+2,2);struct.pack_into('<I',prefix_template,fmt+8,block_size*8000//per_block);struct.pack_into('<H',prefix_template,fmt+12,block_size)
            struct.pack_into('<I',prefix_template,audio_header+44,block_size)
        def build(groups,nested=False,indexed=False,partial=False,declared=3):
            raw=[];pos=0
            for count in groups:
                data=b''.join(blocks[pos:pos+count]);pos+=count;raw.append(data)
            assert pos==3
            if partial:raw[-1]=raw[-1][:-1]
            children=[video]+[chunk(b'01wb',b) for b in raw]
            offset=4+(12 if nested else 0);entries=[]
            for child in children:
                if child[:4]==b'01wb':entries.append(struct.pack('<4sIII',b'01wb',16,offset,struct.unpack_from('<I',child,4)[0]))
                offset+=len(child)
            data=b''.join(children);data=listing(b'rec ',data) if nested else data
            prefix=bytearray(prefix_template);struct.pack_into('<I',prefix,audio_header+32,declared);struct.pack_into('<I',prefix,audio_header+36,max(map(len,raw)))
            main=prefix.index(b'avih')+8;struct.pack_into('<I',prefix,main+12,16 if indexed else 0)
            struct.pack_into('<I',prefix,main+28,max(len(video)-8,max(map(len,raw))))
            result=prefix+listing(b'movi',data)+(chunk(b'idx1',b''.join(entries)) if indexed else b'')
            struct.pack_into('<I',result,4,len(result)-8);return bytes(result)
        files[key+'.f32le']=expected;files[key+'-single.avi']=base if channels==1 else build([1,1,1])
        controls.append(dict(file=key+'-single.avi',expected=key+'.f32le',frames=3*per_block,block_frames=per_block,raw_records=3,channels=channels,block_bytes=block_size))
        profiles=[('two-one',[2,1],False,False),('one-two',[1,2],False,False),('three',[3],False,False),('nested',[2,1],True,False),('indexed',[2,1],False,True)] if channels==1 else [('three',[3],False,False),('indexed',[2,1],False,True)]
        for name,groups,nested,indexed in profiles:
            file=f'{key}-{name}.avi';files[file]=build(groups,nested,indexed);cases.append(dict(file=file,expected=key+'.f32le',frames=3*per_block,block_frames=per_block,raw_records=len(groups),control=key+'-single.avi',groups=groups,nested=nested,indexed=indexed,channels=channels,block_bytes=block_size))
        if channels==1:
          for name,kw in [('incomplete',dict(partial=True)),('wrong-count',dict(declared=2))]:
            file=f'{key}-{name}.avi';files[file]=build([2,1],**kw);malformed.append(dict(file=file,reason=name))
    files['manifest.json']=(json.dumps(dict(provenance='Authored mono/stereo fixed-size ADPCM:dwSampleSize and nBlockAlign8/16,dwLength3 blocks.14 grouped cases include2+1,1+2,3,nested rec and idx1. Four ungrouped controls and four malformed refusals. Stereo uses independent opposite-channel predictor PCM to expose block-byte stride or routing loss. Owned VP9/Matroska companion;no private media,encoder or network. Not a performance benchmark.',cases=cases,controls=controls,malformed=malformed,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
