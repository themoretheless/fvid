#!/usr/bin/env python3
"""Owned single-layer VPS VUI on saved synthetic POC-reset video, offline."""
import json
from generate_hevc_vps_reset_updates import parts
from generate_hevc_colour_plane_fixtures import bits,packed,escape,unescape
from generate_hevc_poc_reset_fixtures import ue
from generate_he_aac_packet_fixtures import DEST,boxes
from hevc_fixture_mp4 import box


def main():
    source='hevc-poc-vps-aligned-msb-reset-synthetic.mp4'
    roots,vps,packets=parts((DEST/source).read_bytes())
    body=bits(unescape(vps[2:])); body=body[:body.rfind('1')]
    assert body[-2:]=='00'
    cases=[]
    labels=['basic','rates','bsp','bad-alignment','bad-rate','bad-index','bad-skip','bad-hrd-count','bad-sublayers']
    for label in labels:
        prefix=body[:-2]+'1'
        padding=-len(prefix)%8
        assert padding>0
        prefix+=('0' if label=='bad-alignment' else '1')+'1'*(padding-1)
        rate=label in ('rates','bad-rate')
        signal='1010'+'00000010'*3  # unspecified base-layer colour metadata
        vui='10'+('11' if rate else '00')
        if rate: vui+='11'+f'{1:016b}{100:016b}'+('11' if label=='bad-rate' else '10')+f'{256:016b}'
        if label=='bad-index': vui+='1'+'0001'+signal*2+'1111'
        else: vui+='0'+signal
        vui+='11'  # no tiles or WPP
        vui+=('01' if label=='bad-skip' else '00')+'0'
        bsp=label in ('bsp','bad-hrd-count','bad-sublayers')
        vui+=str(int(bsp))
        if label=='bsp': vui+=ue(2)+ue(0)+'00'+'111'+'0'+ue(0)+'111'
        elif label=='bad-hrd-count': vui+=ue(1025)
        elif label=='bad-sublayers': vui+=ue(1)+ue(1)
        replacement=vps[:2]+escape(packed(prefix+vui+'0'+'1'))
        def rewrite(tag,payload):
            if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):payload=b''.join(rewrite(t,p) for t,p in boxes(payload))
            elif tag==b'stsd':
                def config(data):
                    result=bytearray(data[:23]); at=23
                    for _ in range(data[22]):
                        header=data[at:at+3];at+=3;result+=header
                        for _ in range(int.from_bytes(header[1:],'big')):
                            n=int.from_bytes(data[at:at+2],'big');at+=2;nal=data[at:at+n];at+=n
                            if header[0]&63==32:nal=replacement
                            result+=len(nal).to_bytes(2,'big')+nal
                    assert at==len(data)
                    return bytes(result)
                payload=payload[:8]+b''.join(box(t,p[:78]+b''.join(box(k,config(v) if k==b'hvcC' else v) for k,v in boxes(p[78:]))) for t,p in boxes(payload[8:]))
            return box(tag,payload)
        name=f'hevc-vps-vui-{label}-synthetic.mp4'
        (DEST/name).write_bytes(box(b'ftyp',roots[b'ftyp'])+box(b'mdat',b''.join(packets))+rewrite(b'moov',roots[b'moov']))
        cases.append(dict(file=name,expectation='invalid' if label.startswith('bad-') else 'acceptance',failure_picture=0 if label.startswith('bad-') else None))
    (DEST/'hevc-vps-vui.json').write_text(json.dumps(dict(cases=cases,provenance='Owned single-layer VUI on original saved reset seed; no private media, external codec, FFmpeg or network.'),indent=2)+'\n')

if __name__=='__main__':main()
