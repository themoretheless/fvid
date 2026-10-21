"""Fixture-only full CABAC context bank, retaining the independent integer interval writer."""
import re
from pathlib import Path
from generate_avc_mbaff_direct_samples import CabacWriter
class Cabac444Writer(CabacWriter):
    def __init__(self,init_idc=-1,qp=26):
        super().__init__(init_idc,qp)
        text=(Path(__file__).resolve().parents[1]/'src/codec/avc_cabac_444_init.rs').read_text()
        pairs=[tuple(map(int,p))for p in re.findall(r'\((-?\d+), (-?\d+)\)',text)]
        assert len(pairs)==4*564
        for m,n in pairs[(init_idc+1)*564:(init_idc+2)*564]:
            pre=max(1,min(126,(m*max(0,min(51,qp))>>4)+n))
            self.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
        assert len(self.contexts)==1024
