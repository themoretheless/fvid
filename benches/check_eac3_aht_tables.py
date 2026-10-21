"""Optional independent-reference numeric table audit; never runs in tests.
Pass an official FFmpeg libavcodec/eac3_data.c checkout as reference input.
Production code and fixture generators do not import the reference.
"""
import argparse,json,re,hashlib
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('reference',type=Path);a=p.parse_args()
source=(Path(__file__).resolve().parents[1]/'src/codec/eac3_aht_tables.rs').read_text();reference=a.reference.read_text();counts={}; differences={}
for book in range(1,8):
 own=re.search(rf'VQ{book}:.*?=\s*\[(.*?)\n\];',source,re.S).group(1)
 ref=re.search(rf'vq_hebap{book}\[.*?=\s*\{{(.*?)\n\}};',reference,re.S).group(1)
 x=[int(v)for v in re.findall(r'-?\d+',own)];y=[int(v)for v in re.findall(r'-?\d+',ref)]
 counts[book]=len(x)//6
 if x!=y:differences[book]=dict(owned_rows=len(x)//6,reference_rows=len(y)//6,reference_equals_owned_without_first_row=(x[6:]==y))
print(json.dumps(dict(rows=counts,total=sum(counts.values()),reference_sha256=hashlib.sha256(a.reference.read_bytes()).hexdigest(),differences=differences),indent=2))

raise SystemExit(1 if differences else 0)
