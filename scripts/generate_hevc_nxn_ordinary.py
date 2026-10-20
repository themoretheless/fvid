"""Offline angular/DC and split ordinary-TU mixtures beside real DIM PBs."""
from generate_hevc_nxn_dim import generate,OUT
from generate_hevc_skip_mp4 import generate as movies
OPTIONS=[(1,True),(10,False),(10,True),(26,False),(26,True)]
SCHEDULES=['owco','cowo','wocw']
def generate_all():
    files={}
    for mode,split in OPTIONS:
        for schedule in SCHEDULES:
            for residual,label in [(False,'zero'),(True,'mixed')]:
                stem=f'nxn-mode{mode}-'+('split'if split else'whole')+f'-{schedule}-{label}'
                for n,b in generate(schedule,residual,ordinary_mode=mode,ordinary_split=split).items():
                    if n.endswith('.sps'):continue
                    files[f'{stem}-{n.removeprefix("contour-")}']=b
                for dlt in [False,True]:
                    files.update(movies(True,dlt,contour=True,nxn_schedule=schedule,nxn_residual=residual,nxn_ordinary_mode=mode,nxn_ordinary_split=split))
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate_all().items():(OUT/n).write_bytes(b)
