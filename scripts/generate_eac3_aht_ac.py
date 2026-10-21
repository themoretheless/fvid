"""Owned nonzero AHT AC spectra and conventional scalar controls.
All generation is in-process from analytic authored spectra, no external codec.
"""
import json
from pathlib import Path
from generate_eac3_aht import packet,spectrum,conventional
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-aht-ac'
def generate():
 files={};cases=[]
 for converted in [False,True]:
  for mask in [1,2,3]:
   for basis in [1,2,3,4,5,'mixed']:
    name=f'type{2 if converted else 0}-mask{mask}-ac{basis}'
    raw=packet(mask,converted,basis=basis);control=packet(0,converted,False,basis=basis)
    # A conventional 16-bit mantissa has at most 1/2-LSB rounding error.
    # Bound each output sample by the L1 spectral error in each overlap half
    # times the decoder's factor 2, with exponent scaling 2^-8.
    l1=max(sum(abs(conventional(spectrum(bin,ch,basis),b)-round(conventional(spectrum(bin,ch,basis),b)))for bin in range(73)) for ch in range(2) if mask&(1<<ch) for b in range(6))
    bound=4*l1/(32768*256)
    for suffix,data in [('.ec3',raw*3),('-control.ec3',control*3),('.mkv',movie([raw]*3)),('-control.mkv',movie([control]*3))]:files[name+suffix]=data
    cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',blocks=6,mask=mask,basis=basis,quantization_bound=bound,exact=basis==3))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
