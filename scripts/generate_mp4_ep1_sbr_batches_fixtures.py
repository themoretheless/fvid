"""Own consecutive LD-SBR pairs and cross-element AU boundaries; offline."""
import json
from generate_mp4_ep1_fixtures import SOURCE
from generate_mp4_ep1_epochs_fixtures import eld_control
from generate_mp4_ep1_grouped_sbr_fixtures import grouped
OUT=SOURCE/'mp4-ep1-sbr-batches'
def generate():
    files={};cases=[]
    for n in [480,512]:
      for configuration in range(3,8):
       for dual in [False,True]:
        for crc in [False,True]:
            name=f'n{n}-c{configuration}-x{2 if dual else 1}-crc{int(crc)}'
            control,_,count,channels=eld_control(n,configuration,crc,dual)
            groups={3:2,4:3,5:3,6:3,7:4}[configuration]
            for mode in ['batch','bridge']:
                movie,width=grouped(control,groups,mode);video=name+'-'+mode+'.mp4';files[video]=movie
                malformed=[]
                mutations=([('truncate','missing AAC ESC instance')]if mode=='batch'else[])+[('extra','unconsumed AAC ESC information bits')]+([('crc','low-delay SBR CRC mismatch')]if crc else[])
                for mutation,error in mutations:
                    bad=name+'-'+mode+'-'+mutation+'.mp4';files[bad]=grouped(control,groups,mode,mutation)[0];malformed.append(dict(video=bad,error=error))
                cases.append(dict(name=name+'-'+mode,video=video,control='../mp4-ep1-grouped-sbr/'+name+'-control.mp4',golden='../mp4-ep1-grouped-sbr/'+name+'.f32le',rate=48000 if dual else 24000,channels=channels,n=n,streams=width,original_streams=count,groups=groups,mode=mode,malformed=malformed))
    assert len(cases)==80
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own AOT39 EP1 contiguous LD-SBR 9/10 pairs in one AU, and data-of-element plus prefix-of-next-element sharing an AU. Independent scalar references and separate transport controls reused from own grouped-sbr corpus. 480/512, configurations 3..7, core/double rates, CRC off/on. Exact bits, canonical dependencies. No private media, external codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
