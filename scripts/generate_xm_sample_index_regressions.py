"""Author standard zero-based XM sample maps with distinct constant loops."""
import json,hashlib,struct
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/xm-sample-index'
def generate():
    old=(ROOT/'tests/fixtures/xm/two-notes.xm').read_bytes();at=60+struct.unpack_from('<I',old,60)[0]
    instrument=at+9+struct.unpack_from('<H',old,at+7)[0];size=struct.unpack_from('<I',old,instrument)[0]
    header=bytearray(old[:at]);struct.pack_into('<H',header,68,1)
    cells=bytes([49,1,0,0,0,50,1,0,0,0,97,0,0,0,0]);pattern=struct.pack('<IBHH',9,0,3,len(cells))+cells
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for name,mapping,values in [('zero-one',[0,1],[32,-64]),('one-zero',[1,0],[32,-64]),('single-zero',[0,0],[32])]:
        inst=bytearray(old[instrument:instrument+size]);struct.pack_into('<H',inst,27,len(values));inst[33:129]=bytes(96);inst[33+48]=mapping[0];inst[33+49]=mapping[1]
        headers=[];payload=[]
        for value in values:
            h=bytearray(40);struct.pack_into('<III',h,0,128,0,128);h[12]=64;h[14]=1;h[15]=128;headers.append(bytes(h));payload.append(bytes([value%256])+bytes(127))
        files[name+'.xm']=bytes(header)+pattern+bytes(inst)+b''.join(headers)+b''.join(payload)
        cases.append(dict(name=name,file=name+'.xm',mapping=mapping,values=values))
    files['manifest.json']=(json.dumps(dict(provenance='Authored three-row XM with one/two distinct signed8bit constant loops, standard sample-map indices0/1 and one-based pattern instrument1. Header/instrument envelope derived only from repository-authored synthetic fixture; sample bytes and keymaps newly authored. Separate short native VP9/PCM video. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
