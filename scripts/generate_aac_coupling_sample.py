#!/usr/bin/env python3
"""Hand-authored AAC-LC independent CCE; FFmpeg only generates PCM oracle."""
from pathlib import Path
import argparse, re, subprocess
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--stereo', action='store_true')
parser.add_argument('--missing-target', action='store_true')
parser.add_argument('--short', action='store_true')
parser.add_argument('--band-gain', action='store_true')
parser.add_argument('--signed-gain', action='store_true')
parser.add_argument('--point', choices=['independent','before-tns','after-tns'], default='independent')
parser.add_argument('--selection', choices=['shared', 'right', 'left', 'separate'], default='separate')
args=parser.parse_args()
stereo=args.stereo
missing=args.missing_target
independent=args.point == 'independent'
if (args.band_gain or args.signed_gain) and independent: parser.error('band/signed gains require dependent coupling')
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
    fields=[(5,3),(0,4),(1,2),(3,4),(1,4),(0,4),(0,4),(0,2),(0,3),(1,4),(0,1),(0,1),(0,1),(int(stereo),1),(0,4),(int(independent),1),(1,4)]
    used=sum(w for _,w in fields)
    if used%8: fields += [(0,8-used%8)]
    fields += [(0,8)] # zero PCE comment bytes
    sequence=(1 if frame == 0 else 3 if frame == 5 else 2) if args.short else 0
    def ics_info(max_sfb):
        if sequence == 2:
            return [(0,1),(sequence,2),(0,1),(max_sfb,4),(127,7)] # one group, eight short windows
        return [(0,1),(sequence,2),(0,1),(max_sfb,6),(0,1)]
    def silent_channel(common):
        data=[(140,8)]
        if not common: data+=ics_info(0 if independent else 1)
        if not independent: data += [(0,4),(1,3 if sequence == 2 else 5)] # zero codebook, one band
        data += [(0,1)] # pulse absent
        if independent: data += [(0,1)]
        elif sequence == 2:
            data += [(1,1)]+[(1,1),(0,1),(14,4),(1,3),(0,1),(0,1),(1,3)]*8
        else: data += [(1,1),(1,2),(0,1),(49,6),(1,5),(0,1),(0,1),(1,3)] # active target TNS
        return data+[(0,1)] # gain control absent
    if stereo:
        fields += [(1,3),(0,4),(1,1)]+ics_info(0 if independent else 1)+[(0,2)]
        fields += silent_channel(True)+silent_channel(True)
    else:
        fields += [(0,3),(0,4)]+silent_channel(False)
    fields += [(2,3),(1,4),(int(independent),1),(0,3),(int(stereo),1),(int(missing),4)]
    if stereo: fields += [(selection,2)]
    fields += [(int(args.point == "after-tns"),1),(int(args.signed_gain),1),(0,2)]
    fields += [(140,8)]+ics_info(1)+[(1,4),(1,3 if sequence == 2 else 5),(sc[60],sl[60])]
    fields += [(0,1)]*3
    for window in range(8 if sequence == 2 else 1):
        index=80 if (frame+window)%2==0 else 0
        fields += [(codes[index],lens[index])]
    if stereo and selection == 3:
        if not independent: fields += [(int(not args.band_gain),1)] # common gain element
        index=65 if args.signed_gain and args.band_gain else 64
        fields += [(sc[index],sl[index])] # delta 4, or signed delta 5
    fields += [(7,3)]
    bits=''.join(f'{v:0{w}b}' for v,w in fields);bits+='0'*(-len(bits)%8)
    payload=int(bits,2).to_bytes(len(bits)//8,'big');n=len(payload)+7
    output+=bytes([255,241,76,(n>>11),(n>>3)&255,((n&7)<<5)|31,252])+payload
path=root/('tests/fixtures/playback-errors/aac-independent-coupling'+('-missing-target' if missing else '-stereo'+('' if selection == 3 else '-'+args.selection) if stereo else '')+('-short' if args.short else '')+('' if independent else '-'+args.point)+('-band-gain' if args.band_gain else '')+('-signed' if args.signed_gain else '')+'.aac');path.write_bytes(output)
if not missing: subprocess.run(['ffmpeg','-v','error','-i',str(path),'-f','f32le','-c:a','pcm_f32le','-y',str(path.with_suffix('.f32le'))],check=True)
