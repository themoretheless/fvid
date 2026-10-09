#!/usr/bin/env python3
"""Own late in-band PS packets in mono-core ADTS; no foreign codec or network."""
import json, subprocess
from pathlib import Path
from generate_he_aac_packet_fixtures import DEST, video_fixture
from generate_adts_layout_sbr_fixtures import fixed
from generate_adts_multiblock_fixtures import transport

def generate(source, stem='adts-implicit-ps', provenance='existing authored late PS packets and independent scalar stereo PCM; own CRC regions and independent polynomial division'):
    blob=(DEST/'he-aac-ps-absence-packets.bin').read_bytes()
    frames=[dict(row,header=fixed(bytes.fromhex('1308'),blob[row['offset']:row['offset']+row['bytes']],1)) for row in source['frames']]
    case=dict(asc='1308',slots=16,bands=64,frames=frames,sample_rate=48000,container_rate=48000,container_frame_samples=2048,samples=6144,pcm_offset=0)
    video=video_fixture([case],blob,channels=2,filename=stem+'-synthetic.mp4')
    subprocess.run(['cargo','build','--offline','--quiet','--no-default-features','--example','adts_crc_regions'],check=True)
    meta=json.loads(subprocess.check_output(['cargo','metadata','--offline','--no-deps','--format-version','1']))
    inputs=[dict(asc='1308',payload=blob[r['offset']:r['offset']+r['bytes']].hex()) for r in frames]
    spans=json.loads(subprocess.run([str(Path(meta['target_directory'])/'debug/examples/adts_crc_regions')],input=json.dumps(inputs),text=True,capture_output=True,check=True).stdout)
    for row,regions in zip(frames,spans): row['regions']=regions
    files=[]
    for counts in [[1,1,1],[3],[1,2]]:
        for crc in [False,True]:
            data=bytearray();at=0
            for count in counts:
                data+=transport(frames[at:at+count],blob,crc);at+=count
            name=stem+'-'+''.join(map(str,counts))+('-crc' if crc else '')+'-synthetic.aac'
            (DEST/name).write_bytes(data);files.append(name)
    (DEST/(stem+'.json')).write_text(json.dumps(dict(files=files,video=video,pcm=source['pcm']['Double'],provenance=provenance),indent=2)+'\n')
def main():
    manifest=json.loads((DEST/'aac-ps-inband-oracles.json').read_text())
    source=next(c for c in manifest['cases'] if c['kind']=='LC' and c['slots']==16)
    generate(source)
if __name__=='__main__':main()
