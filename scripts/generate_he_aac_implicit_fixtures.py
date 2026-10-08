#!/usr/bin/env python3
"""Original implicit SBR ASC with declared 48kHz container output, offline."""
import json,hashlib
from generate_he_aac_packet_fixtures import DEST,packed,field,video_fixture

def main():
    source=json.loads((DEST/'he-aac-sbr-packets.json').read_text());blob=(DEST/'he-aac-sbr-packets.bin').read_bytes()
    cases=[]
    for original in source['cases']:
        if original['bands']!=64 or original['signalling']!='explicit':continue
        case=dict(original);case['signalling']='implicit'
        case['asc']=packed(field(2,5)+field(6,4)+'0001'+field(case['slots']==15,1)+'00').hex()
        cases.append(case)
    video=video_fixture(cases,blob,1,'he-aac-implicit-sbr.mp4')
    case=next(c for c in cases if c['slots']==16)
    adts=bytearray()
    for record in case['frames']:
        raw=blob[record['offset']:record['offset']+record['bytes']];length=7+len(raw)
        adts.extend(bytes([0xff,0xf1,0x58,0x40|((length>>11)&3),(length>>3)&255,((length&7)<<5)|31,0xfc])+raw)
    (DEST/'he-aac-implicit-sbr.aac').write_bytes(adts)
    adts_info=dict(file='he-aac-implicit-sbr.aac',sha256=hashlib.sha256(adts).hexdigest(),pcm_offset=case['pcm_offset'],samples=case['samples'])
    (DEST/'he-aac-implicit-sbr.json').write_text(json.dumps(dict(kind='original SBR packets with implicit ASC and declared dual-rate output',video=video,adts=adts_info,cases=cases),separators=(',',':'))+'\n')
    print(len(cases),'implicit 24-to-48 kHz sequences')
if __name__=='__main__':main()
