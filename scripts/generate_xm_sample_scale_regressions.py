"""Author equivalent signed8/signed16 XM sample pairs without an encoder."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_sample_index_regressions import generate as indices
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-sample-scale'
def generate():
    source=indices();base=source['single-zero.xm'];at=60+struct.unpack_from('<I',base,60)[0];instrument=at+9+struct.unpack_from('<H',base,at+7)[0];sample=instrument+struct.unpack_from('<I',base,instrument)[0]
    files={'video.mkv':source['video.mkv']};cases=[]
    for value in [-128,-1,0,1,127]:
        names={}
        for bits in [8,16]:
            width=bits//8;b=bytearray(base[:sample+40]);struct.pack_into('<III',b,sample,128*width,0,128*width);b[sample+14]=1+(16 if bits==16 else 0)
            delta=bytes([value%256]) if bits==8 else struct.pack('<h',value*256)
            b+=delta+bytes(127*width);name=f'constant-{value}-{bits}.xm';files[name]=bytes(b);names[str(bits)]=name
        cases.append(dict(value=value,files=names))
    files['manifest.json']=(json.dumps(dict(provenance='Five authored constant signed8bit sample loops and equivalent signed16bit value*256 controls, standard zero-based XM keymap. Negative fullscale,negativeLSB,zero,positiveLSB,positivefullscale. Separate authored native VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
