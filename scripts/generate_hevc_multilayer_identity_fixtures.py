#!/usr/bin/env python3
"""Own Annex F identity PPS extension on synthetic I/P/B seeds; offline."""
import json
import struct
from generate_he_aac_packet_fixtures import DEST, boxes
from hevc_fixture_mp4 import box
from generate_hevc_colour_plane_fixtures import bits, unescape, escape, packed


def extension(nal,body,future):
    value=bits(unescape(nal[2:]));syntax=value[:value.rfind('1')]
    assert syntax[-1]=='0','seed PPS must have no extension'
    flags='0100'+('0001' if future else '0000')
    return nal[:2]+escape(packed(syntax[:-1]+'1'+flags+body+('0100101' if future else '')+'1'))


def main():
    cases=[]
    for seed in ['main-ipb','main10-ipb']:
        data=(DEST.parent/'hevc'/f'{seed}.mp4').read_bytes();roots=list(boxes(data));assert roots[-1][0]==b'moov'
        for kind,body,future in [('identity','0010',False),('identity-tail','0010',True),('poc-reset','1010',False),('truncated','00',False),('colour-mapping','0011',False)]:
            def config(data):
                out=bytearray(data[:23]);at=23;changed=0
                for _ in range(data[22]):
                    header=data[at:at+3];at+=3;out+=header
                    for _ in range(int.from_bytes(header[1:],'big')):
                        n=int.from_bytes(data[at:at+2],'big');at+=2;nal=data[at:at+n];at+=n
                        if header[0]&63==34:nal=extension(nal,body,future);changed+=1
                        out+=struct.pack('>H',len(nal))+nal
                assert at==len(data) and changed==1
                return bytes(out)
            def rewrite(tag,payload):
                if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):payload=b''.join(rewrite(t,p) for t,p in boxes(payload))
                elif tag==b'stsd':payload=payload[:8]+b''.join(box(t,p[:78]+b''.join(box(k,config(v) if k==b'hvcC' else v) for k,v in boxes(p[78:]))) for t,p in boxes(payload[8:]))
                return box(tag,payload)
            name=f'hevc-multilayer-pps-{seed}-{kind}-synthetic.mp4'
            (DEST/name).write_bytes(b''.join(rewrite(t,p) for t,p in roots));cases.append(dict(file=name,seed=seed,kind=kind,expectation=('invalid' if kind=='truncated' else ('acceptance' if kind.startswith('identity') else ('configuration' if kind=='poc-reset' else 'refusal')))))
    (DEST/'hevc-multilayer-pps-identity.json').write_text(json.dumps(dict(cases=cases,provenance='Original synthetic testsrc2 I/P/B seeds documented in fixtures/hevc/README.md. Own offline rewriting of PPS Annex F multilayer syntax, identity operation and future tails; POC-reset cases probe configuration admission only; colour-mapping remains refusal, not playback acceptance; truncated is malformed syntax. No private media, external tool, FFmpeg or network used by this generator or ordinary tests.'),indent=2)+'\n')
    print('Multilayer PPS cases:',len(cases))

if __name__=='__main__':main()
