"""Exact rational component oracles for Annex I camera disparity, fully offline.

These are mathematical/API domain fixtures, not coded profile acceptance.
"""
import json,math,struct
from fractions import Fraction
from generate_hevc_wedge_tables import OUT
PARAMETERS=[('unit',[1,0,0,0]),('negative',[-3,-1,2,4]),
    ('asymmetric',[128,-64,7,-8]),
    ('extreme',[2147483647,-2147483647,-2147483647,2147483647]),
    ('min-input',[-2147483648,2147483647,2147483647,-2147483648])]
def oracle(values,precision,bits,d):
    scale,offset,sum_scale,sum_offset=values
    divisor=2**(bits-1+precision)
    b=Fraction(scale*d+offset*2**bits,divisor)+Fraction(1,2)
    f=Fraction((sum_scale-scale)*d+(sum_offset-offset)*2**bits,divisor)+Fraction(1,2)
    return math.floor(b),math.floor(f)
def generate():
    files={};cases=[]
    for name,values in PARAMETERS:
        for bits in range(8,17):
            count=2**bits
            points=sorted(set([0,1,2,count//4,count//2-1,count//2,count-2,count-1]))
            for precision in range(6):
                filename=None
                if bits==8:
                    filename=f'disparity-oracle-{name}-{precision}.i64'
                    files[filename]=b''.join(struct.pack('<qq',*oracle(values,precision,bits,d))for d in range(count))
                cases.append(dict(name=name,values=values,precision=precision,depth=bits,table=filename,
                    points=[[d,*oracle(values,precision,bits,d)]for d in points]))
    files['disparity-oracles.json']=(json.dumps(dict(provenance='Independent Python arbitrary-precision Fraction/floor oracle for I-38..I-43. Component/API ranges, not coded profile acceptance. No external codec, private source, network or test-time fixture generation.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
