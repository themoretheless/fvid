"""Optional external HTM qualification. Never used by ordinary test execution."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('htm',type=Path,help='Existing HTM checkout; no download is performed')
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]
    checkout=args.htm.resolve();common=checkout/'source/Lib/TLibCommon'
    sources=[common/'TComWedgelet.cpp',common/'TComRom.cpp']
    revision=subprocess.check_output(['git','-C',str(checkout),'rev-parse','HEAD'],text=True).strip()
    subprocess.run(['git','-C',str(checkout),'diff','--quiet','HEAD','--','source/Lib'],check=True)
    rows=[]
    with tempfile.TemporaryDirectory(prefix='fvid-htm-wedge-') as directory:
        work=Path(directory);exe=work/'check'
        subprocess.run(['clang++','-std=c++11','-O2','-Wno-deprecated-register',
            '-I'+str(common.parent),'-I'+str(common),
            str(root/'scripts/fixture-tools/hevc-wedge-htm.cpp'),
            *map(str,sources),'-o',str(exe)],check=True)
        subprocess.run([str(exe),str(work)],check=True)
        for size in [4,8,16,32]:
            external=(work/f'{size}.bin').read_bytes()
            owned=(root/f'tests/fixtures/playback-errors/hevc-wedge/{size}.bin').read_bytes()
            assert external==owned,f'{size}: indexed masks differ'
            rows.append(dict(size=size,count=len(external)//(size*size),bytes=len(external),
                sha256=hashlib.sha256(external).hexdigest(),byte_identical=True))
    report=dict(reference='https://github.com/listenlink/3D-HEVC',revision=revision,
        reference_sources={p.name:hashlib.sha256(p.read_bytes()).hexdigest()for p in sources},
        scope='Every indexed wedge mask, including HTM 32x32 scaled copies; not full picture decoding.',
        tables=rows)
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(report,indent=2)+'\n')
    print('All four indexed tables match unchanged external HTM sources.')
if __name__=='__main__':main()
