#!/usr/bin/env python3
"""Owned MBAFF reproducers and independent JM pixels; explicit generation, no FFmpeg."""
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path
from avc_fixture_mp4 import read_mkv, mux, annexb

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--x264', type=Path, required=True)
    p.add_argument('--jm-decoder', type=Path, required=True)
    a=p.parse_args()
    output=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-avc-mbaff-') as temporary:
        directory=Path(temporary)
        source=directory/'owned.yuv'
        source.write_bytes(bytes(24+(x*3+y*5+frame*17+component*29)%160
            for frame in range(3) for component in range(3)
            for y in range(64 if component==0 else 32)
            for x in range(64 if component==0 else 32)))
        for entropy in ['cabac','cavlc','field-cabac','field-cavlc','field-intra-unfiltered-cavlc',
                        'frame-intra-unfiltered-cavlc','mixed-intra-unfiltered-cavlc','mixed-reverse-intra-unfiltered-cavlc','field-multislice-intra-unfiltered-cavlc',
                        'field-intra-filtered-cavlc','frame-intra-filtered-cavlc',
                        'mixed-intra-filtered-cavlc','mixed-reverse-intra-filtered-cavlc',
                        'field-multislice-intra-filtered-cavlc',
                        'mixed-vertical-intra-filtered-cavlc',
                        'mixed-vertical-reverse-intra-filtered-cavlc',
                        'field-intra-filtered-high10-cavlc','frame-intra-filtered-high10-cavlc',
                        'mixed-intra-filtered-high10-cavlc','mixed-reverse-intra-filtered-high10-cavlc',
                        'mixed-vertical-intra-filtered-high10-cavlc',
                        'mixed-vertical-reverse-intra-filtered-high10-cavlc',
                        'field-multislice-intra-filtered-high10-cavlc',
                        'field-intra-filtered-cabac','frame-intra-filtered-cabac',
                        'mixed-intra-filtered-cabac','mixed-reverse-intra-filtered-cabac',
                        'mixed-vertical-intra-filtered-cabac','mixed-vertical-reverse-intra-filtered-cabac',
                        'field-multislice-intra-filtered-cabac',
                        'field-intra-filtered-high10-cabac','frame-intra-filtered-high10-cabac',
                        'mixed-intra-filtered-high10-cabac','mixed-reverse-intra-filtered-high10-cabac',
                        'mixed-vertical-intra-filtered-high10-cabac',
                        'mixed-vertical-reverse-intra-filtered-high10-cabac',
                        'field-multislice-intra-filtered-high10-cabac']:
            depth=10 if 'high10' in entropy else 8
            def sample(x,y,frame,component):
                width=64 if component==0 else 32
                coordinate=y if 'vertical' in entropy else x
                field=entropy.startswith('field') or (entropy.startswith('mixed') and
                    ((coordinate<width//2) if 'reverse' in entropy else (coordinate>=width//2)))
                if field:
                    return 24+(x*3+(y//2)*5+frame*17+component*29)%80+(y%2)*128
                return 24+(x*3+y*5+frame*17+component*29)%160
            samples=(sample(x,y,frame,component)
                for frame in range(3) for component in range(3)
                for y in range(64 if component==0 else 32)
                for x in range(64 if component==0 else 32))
            source.write_bytes(bytes(samples) if depth==8 else b''.join(
                (value*4+(index%4)).to_bytes(2,'little') for index,value in enumerate(samples)))
            stream=directory/f'{entropy}.mkv'
            command=[str(a.x264), '--demuxer','raw','--input-csp','i420',
                '--input-res','64x64','--fps','25','--frames','3','--threads','1',
                '--keyint','30','--bframes','0','--tff','--profile','high10' if depth==10 else 'main',
                '--muxer','mkv','-o',str(stream)]
            if depth==10: command+=['--input-depth','10','--output-depth','10']
            if entropy.endswith('cavlc'): command+=['--no-cabac']
            if 'intra' in entropy: command+=['--keyint','1']
            if 'unfiltered' in entropy: command+=['--no-deblock']
            if 'multislice' in entropy: command+=['--slices','2']
            subprocess.run(command+[str(source)],check=True)
            configuration,frames=read_mkv(stream.read_bytes(),25)
            coded=directory/f'{entropy}.264'
            oracle=directory/f'{entropy}.yuv'
            cfg=directory/'decoder.cfg'
            cfg.write_text('')
            coded.write_bytes(annexb(configuration,frames))
            subprocess.run([str(a.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}',
                '-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],
                cwd=directory,check=True)
            pixels=oracle.read_bytes()
            assert len(pixels)==3*64*64*3//2*(2 if depth>8 else 1)
            (output/f'avc-mbaff-{entropy}.yuv').write_bytes(pixels)
            data=mux(configuration,frames,64,64)
            name=f'avc-mbaff-{entropy}.mp4'
            (output/name).write_bytes(data)
            records.append(dict(file=name,sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (output/'avc-mbaff-generated.json').write_text(json.dumps(dict(
        generator='owned synthetic pattern; x264 CLI',
        jm_decoder_sha256=hashlib.sha256(a.jm_decoder.read_bytes()).hexdigest(),
        x264_sha256=hashlib.sha256(a.x264.read_bytes()).hexdigest(),
        fixtures=records),indent=2)+'\n')
if __name__=='__main__': main()
