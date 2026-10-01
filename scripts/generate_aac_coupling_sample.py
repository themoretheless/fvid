#!/usr/bin/env python3
"""Hand-authored AAC-LC independent CCE; FFmpeg only generates PCM oracle."""
from pathlib import Path
import argparse, re, subprocess
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--stereo', action='store_true')
parser.add_argument('--missing-target', action='store_true')
parser.add_argument('--short', action='store_true')
parser.add_argument('--selection', choices=['shared', 'right', 'left', 'separate'], default='separate')
args=parser.parse_args()
stereo=args.stereo
missing=args.missing_target
selection={'shared':0, 'right':1, 'left':2, 'separate':3}[args.selection]
if not stereo and args.selection != 'separate':
    parser.error('--selection requires --stereo')
root=Path(__file__).resolve().parents[1]
tables=(root/'src/codec/aac_huffman_tables.rs').read_text()
def table(name):
    text=re.search(r'const '+name+r':[^=]+ = \[(.*?)\];',tables,re.S)[1]
    return [int(n,0) for n in re.findall(r'0x[0-9a-fA-F]+|\d+',text)]
codes=table('SPECTRUM_CODEBOOK1_CODES');lens=table('SPECTRUM_CODEBOOK1_LENS')
sc=table('SCF_CODEBOOK_CODES');sl=table('SCF_CODEBOOK_LENS')
output=bytearray()
for frame in range(6):
    fields=[(5,3),(0,4),(1,2),(3,4),(1,4),(0,4),(0,4),(0,2),(0,3),(1,4),(0,1),(0,1),(0,1),(int(stereo),1),(0,4),(1,1),(1,4)]
    used=sum(w for _,w in fields)
    if used%8: fields += [(0,8-used%8)]
    fields += [(0,8)] # zero PCE comment bytes
    sequence=(1 if frame == 0 else 3 if frame == 5 else 2) if args.short else 0
    def ics_info(max_sfb):
        if sequence == 2:
            return [(0,1),(sequence,2),(0,1),(max_sfb,4),(127,7)] # one group, eight short windows
        return [(0,1),(sequence,2),(0,1),(max_sfb,6),(0,1)]
    if stereo:
        fields += [(1,3),(0,4),(1,1)]+ics_info(0)+[(0,2)]
        for _ in range(2): fields += [(140,8),(0,1),(0,1),(0,1)]
    else:
        fields += [(0,3),(0,4),(140,8)]+ics_info(0)+[(0,1)]*3
    fields += [(2,3),(1,4),(1,1),(0,3),(int(stereo),1),(int(missing),4)]
    if stereo: fields += [(selection,2)]
    fields += [(0,1),(0,1),(0,2)]
    fields += [(140,8)]+ics_info(1)+[(1,4),(1,3 if sequence == 2 else 5),(sc[60],sl[60])]
    fields += [(0,1)]*3
    for window in range(8 if sequence == 2 else 1):
        index=80 if (frame+window)%2==0 else 0
        fields += [(codes[index],lens[index])]
    if stereo and selection == 3: fields += [(sc[64],sl[64])] # right gain 2^(-4/8)
    fields += [(7,3)]
    bits=''.join(f'{v:0{w}b}' for v,w in fields);bits+='0'*(-len(bits)%8)
    payload=int(bits,2).to_bytes(len(bits)//8,'big');n=len(payload)+7
    output+=bytes([255,241,76,(n>>11),(n>>3)&255,((n&7)<<5)|31,252])+payload
path=root/('tests/fixtures/playback-errors/aac-independent-coupling'+('-missing-target' if missing else '-stereo'+('' if selection == 3 else '-'+args.selection) if stereo else '')+('-short' if args.short else '')+'.aac');path.write_bytes(output)
if not missing: subprocess.run(['ffmpeg','-v','error','-i',str(path),'-f','f32le','-c:a','pcm_f32le','-y',str(path.with_suffix('.f32le'))],check=True)
