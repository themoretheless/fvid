#!/usr/bin/env python3
"""Original lossless camera tile list and unmodified stock libaom golden."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--generator',type=Path,required=True);a=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    with tempfile.TemporaryDirectory(prefix='fvid-tile-list-') as tmp:
        prefix=Path(tmp)/'fixture';subprocess.run([str(a.generator),str(prefix)],check=True)
        reference=Path(str(prefix)+'-list.yuv').read_bytes();expected=bytearray()
        for plane in range(3):
            tile=64 if plane==0 else 32;dimension=tile*2
            for y in range(dimension):
                for x in range(dimension):
                    source=[3,0,2,1][(y//tile)*2+x//tile]
                    sx=(source%2)*tile+x%tile;sy=(source//2)*tile+y%tile
                    expected.append((71+3*sx+5*sy+23*plane)%192)
        assert reference==expected,'stock oracle must reproduce lossless authored tile permutation'
        records={}
        for suffix in ['anchor.obu','camera.obu','header.obu','list.obu','list.yuv']:
            data=Path(str(prefix)+'-'+suffix).read_bytes();name='av1-tile-list-'+suffix;(root/name).write_bytes(data)
            records[suffix]={'file':name,'sha256':hashlib.sha256(data).hexdigest()}
        (root/'av1-tile-list-generated.json').write_text(json.dumps({'size':[128,128],'tile_size':[64,64],'order':[3,0,2,1],'oracle':'stock libaom','artifacts':records},indent=2)+'\n')
if __name__=='__main__':main()
