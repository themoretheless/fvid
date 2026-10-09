#!/usr/bin/env python3
"""Own late in-band PS packets in mono-core ADTS; no foreign codec or network."""
import json, subprocess
from functools import lru_cache
from pathlib import Path
from generate_he_aac_packet_fixtures import DEST, video_fixture
from generate_adts_layout_sbr_fixtures import fixed
from generate_adts_multiblock_fixtures import transport

@lru_cache(maxsize=1)
def region_helper():
    subprocess.run(['cargo','build','--offline','--quiet','--no-default-features','--example','adts_crc_regions'],check=True)
    meta=json.loads(subprocess.check_output(['cargo','metadata','--offline','--no-deps','--format-version','1']))
    return str(Path(meta['target_directory'])/'debug/examples/adts_crc_regions')

def generate(source, stem='adts-implicit-ps', provenance='existing authored late PS packets and independent scalar stereo PCM; own CRC regions and independent polynomial division', setup='1308', packet_file='he-aac-ps-absence-packets.bin', configuration=1, groups=None, channels=2):
    blob=(DEST/packet_file).read_bytes()
    frames=[dict(row,header=fixed(bytes.fromhex(setup),blob[row['offset']:row['offset']+row['bytes']],configuration)) for row in source['frames']]
    case=dict(asc=setup,slots=16,bands=64,frames=frames,sample_rate=48000,container_rate=48000,container_frame_samples=2048,samples=2048*len(frames),pcm_offset=0)
    video=video_fixture([case],blob,channels=channels,filename=stem+'-synthetic.mp4')
    helper=region_helper()
    inputs=[dict(asc=setup,payload=blob[r['offset']:r['offset']+r['bytes']].hex()) for r in frames]
    spans=json.loads(subprocess.run([helper],input=json.dumps(inputs),text=True,capture_output=True,check=True).stdout)
    for row,regions in zip(frames,spans): row['regions']=regions
    files=[]
    for counts in (groups or [[1,1,1],[3],[1,2]]):
        for crc in [False,True]:
            data=bytearray();at=0
            for count in counts:
                data+=transport(frames[at:at+count],blob,crc);at+=count
            name=stem+'-'+''.join(map(str,counts))+('-crc' if crc else '')+'-synthetic.aac'
            (DEST/name).write_bytes(data);files.append(name)
    result=dict(files=files,video=video,pcm=source.get('pcm',{}).get('Double'),provenance=provenance)
    if len(frames)!=3:result['samples']=2048*len(frames)
    (DEST/(stem+'.json')).write_text(json.dumps(result,indent=2)+'\n')
    return result
def main():
    manifest=json.loads((DEST/'aac-ps-inband-oracles.json').read_text())
    source=next(c for c in manifest['cases'] if c['kind']=='LC' and c['slots']==16)
    generate(source)
if __name__=='__main__':main()
