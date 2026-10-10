#!/usr/bin/env python3
"""Own SP deblocking fixtures; explicit optional JM reference capture."""
import argparse,json,subprocess,tempfile
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux,annexb
from avc_sp_chroma_reference import sequence
DEST=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path);args=parser.parse_args()
    config=bytes.fromhex(json.loads((DEST/'avc-switching-chroma.json').read_text())['video']['configuration']);cases=[]
    for mode in (0,2):
        b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(1);b.ue(25);b.align()
        source=[64+(i//16//4)*5+(i%16//4)*4 for i in range(256)]
        source += [96+(i//8//4)*3+(i%8//4)*3 for i in range(64)]
        source += [144-(i//8//4)*3+(i%8//4)*3 for i in range(64)]
        for v in source:b.u(v,8)
        nal=b.nal(0x65);first=len(nal).to_bytes(4,'big')+nal;frames=[(0,True,first)];packets=[first.hex()];secondary=[]
        for i,qs in enumerate([0,26,51],1):
            for switching in [False,True]:
                b=Writer();b.ue(0);b.ue(3);b.ue(0);b.u(i,4);b.u(2*i,4);b.u(0);b.u(0);b.u(0)
                b.se(24);b.u(int(switching));b.se(qs-26);b.ue(mode);b.se(0);b.se(0);b.ue(1)
                nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal
                if switching:secondary.append(packet.hex())
                else:packets.append(packet.hex());frames.append((i,False,packet))
        name=f'avc-primary-sp-filter{mode}'
        (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,16,16,30))
        reference=name+'-reference.yuv'
        secondary_file=name+'-secondary-refusal-synthetic.mp4'
        (DEST/secondary_file).write_bytes(mux(config,[(0,True,first)]+[(i+1,False,bytes.fromhex(packet)) for i,packet in enumerate(secondary)],16,16,30))
        if args.jm_decoder:
            with tempfile.TemporaryDirectory(prefix='fvid-sp-filter-') as tmp:
                d=Path(tmp);(d/'decoder.cfg').write_text('');(d/'owned.264').write_bytes(annexb(config,frames))
                with (d/'jm.log').open('w') as log:
                    subprocess.run([str(args.jm_decoder),'-d',str(d/'decoder.cfg'),'-p',f'InputFile={d}/owned.264','-p',f'OutputFile={d}/owned.yuv','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,stdout=log,stderr=subprocess.STDOUT,check=True)
                raw=(d/'owned.yuv').read_bytes();assert len(raw)==1536;(DEST/reference).write_bytes(raw)
        jm_reference=reference
        reference=name+'-normative-reference.yuv'
        jm=(DEST/jm_reference).read_bytes()
        planes=[source[:256],source[256:320],source[320:]]
        assert sequence(jm,planes,16,mode,sign_inside=False)==jm
        (DEST/reference).write_bytes(sequence(jm,planes,16,mode))
        cases.append(dict(file=name+'-synthetic.mp4',reference=reference,jm_reference=jm_reference,configuration=config.hex(),packets=packets,secondary_packets=secondary,secondary_file=secondary_file,source=source,mode=mode))
    (DEST/'avc-primary-sp-filter.json').write_text(json.dumps(dict(cases=cases,provenance='Own smooth block steps and primary SP skip at QPY50, QSY0/26/51; modes0/2. Optional explicit JM capture saves filtered YUV separately; ordinary generation and tests need no external process, FFmpeg or network. Secondary SP companions originally reproduced a refusal; now acceptance tests decode them. JM captures are preserved separately; normative chroma uses sign-inside-shift scalar reconstruction. Saved JM luma remains the cross-check.'),indent=2)+'\n')
if __name__=='__main__':main()
