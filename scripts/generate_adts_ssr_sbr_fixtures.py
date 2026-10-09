#!/usr/bin/env python3
"""Owned SSR/SBR ADTS framing; only owned syntax helper and polynomial CRC."""
import json,subprocess
from generate_aac_ssr_fixtures import DEST,field,frequency,packed
from generate_adts_ps_fixtures import region_helper
from generate_adts_multiblock_fixtures import transport

def main():
    source=json.loads((DEST/'aac-ssr-sbr.json').read_text());blob=(DEST/'aac-ssr-sbr-packets.bin').read_bytes();cases=[]
    selected=[c for c in source['cases'] if c['name'] in ('implicit','implicit-active','core-control')]+[dict(source['active_core_control'],name='active-core-control')]
    for c in selected:
        rows=[]
        for r in c['frames']:
            size=r['bytes']+9
            h=packed(field(0xfff,12)+'0'+'00'+'0'+'10'+frequency(24000)+'0'+'001'+'0000'+field(size,13)+field(0x7ff,11)+'00')
            rows.append(dict(r,header=h.hex()))
        spans=json.loads(subprocess.run([region_helper()],input=json.dumps([dict(asc=c['asc'],payload=blob[r['offset']:r['offset']+r['bytes']].hex()) for r in rows]),capture_output=True,text=True,check=True).stdout)
        for row,regions in zip(rows,spans):row['regions']=regions
        files=[]
        for counts in ([1]*6,[3,3],[1,2,3]):
            for crc in (False,True):
                at=0;data=bytearray()
                for n in counts:data+=transport(rows[at:at+n],blob,crc);at+=n
                name='adts-ssr-sbr-'+c['name']+'-'+''.join(map(str,counts))+('-crc' if crc else '')+'-synthetic.aac';(DEST/name).write_bytes(data);files.append(name)
        cases.append(dict(c,files=files))
    (DEST/'adts-ssr-sbr.json').write_text(json.dumps(dict(cases=cases,provenance='Own SSR window transitions, nonzero spectra and active gain, existing authored SBR; owned protected-region reader and independent polynomial division. No private media or foreign codec.'),indent=2)+'\n')
if __name__=='__main__':main()
