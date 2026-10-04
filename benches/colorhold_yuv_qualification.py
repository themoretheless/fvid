#!/usr/bin/env python3
"""Explicit conversion qualification, not an equivalence or ordinary-test gate."""
import argparse
from pathlib import Path
import subprocess
import tempfile
parser=argparse.ArgumentParser()
parser.add_argument('--fvid',type=Path,required=True)
parser.add_argument('--diagnostic',action='store_true',help='print intermediate synthetic RGB/UV samples')
parser.add_argument('--save-references',action='store_true',help='write synthetic reference sample planes')
args=parser.parse_args()
root=Path(__file__).resolve().parents[1]
def run(command):
    result=subprocess.run(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors='replace'))
    return result.stdout
with tempfile.TemporaryDirectory(prefix='fvid-colorhold-yuv-') as directory:
    for depth in (8,12,16):
        source=root/f'tests/fixtures/playback-errors/colorize-grid-{depth}.y4m'
        pix='yuv420p' if depth==8 else f'yuv420p{depth}le'
        for index,options in enumerate(('black:0.00001','red:0.2:0.5')):
            output=Path(directory)/f'output-{depth}-{index}.mkv'
            run([str(args.fvid.resolve()),'media','transcode-lossless',str(source),str(output),'--colorhold',options])
            own=run(['ffmpeg','-nostdin','-v','error','-i',str(output),'-an','-pix_fmt',pix,'-f','rawvideo','-'])
            ref=run(['ffmpeg','-nostdin','-v','error','-i',str(source),'-vf',f'format=rgba64le,colorhold={options}', '-an','-pix_fmt',pix,'-f','rawvideo','-'])
            if args.diagnostic and depth==8 and index==1:
                print('own first frame',list(own[:17]),'reference',list(ref[:17]))
                rgba=run(['ffmpeg','-nostdin','-v','error','-i',str(source),'-vf','format=rgba64le','-frames:v','1','-f','rawvideo','-'])
                print('reference RGB16',[int.from_bytes(rgba[i:i+2],'little') for i in range(0,len(rgba),2)])
                filtered=run(['ffmpeg','-nostdin','-v','error','-i',str(source),'-vf','format=rgba64le,colorhold=red:0.2:0.5','-frames:v','1','-f','rawvideo','-'])
                uv=[]
                for at in range(0,len(filtered),8):
                    r,g,b=[int.from_bytes(filtered[at+i:at+i+2],'little')/65535 for i in (0,2,4)]
                    l=.299*r+.587*g+.114*b
                    uv.append((round(128+224*(b-l)/(2*(1-.114))),round(128+224*(r-l)/(2*(1-.299)))))
                print('per-pixel UV',uv)

            assert len(own)==len(ref), 'geometry/frame count mismatch'
            if args.save_references:
                kind='black' if index==0 else 'blend'
                (root/f'tests/fixtures/playback-errors/colorhold-{kind}-reference-{depth}.raw').write_bytes(ref)
            size=1 if depth==8 else 2
            values=lambda data:[int.from_bytes(data[i:i+size],'little') for i in range(0,len(data),size)]
            differences=[abs(a-b) for a,b in zip(values(own),values(ref))]
            print(f'depth={depth} args={options} samples={len(differences)} different={sum(v!=0 for v in differences)} max_delta={max(differences)} mean_delta={sum(differences)/len(differences):.3f} conversion_parity={"exact" if not any(differences) else "not-established"}')
