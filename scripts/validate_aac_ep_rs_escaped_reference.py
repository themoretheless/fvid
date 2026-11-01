#!/usr/bin/env python3
"""Explicit original RetrieveHeader component comparison, after header FEC recovery."""
import hashlib,json,subprocess,sys,tempfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def sha(b):return hashlib.sha256(b).hexdigest()
def run(source):
 source=Path(source);original=(source/'eptool.c').read_bytes();text=original.decode();a=text.index('int RetrieveHeader(');b=text.index('int RetrieveHeaderDeinter(',a);function=text[a:b]
 driver=r'''
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include "eptool.h"
#define MHDATA 255
/* Header FEC is outside this component: pass already recovered bits exactly. */
int FECDecodeHeader(int *out,int n,int *in,BCHTable *table,int ext,int rate,int crc) {
    memcpy(out,in,n*sizeof(int)); return n;
}
unsigned int bintoint(int *bits,int n) {int v=0;for(int i=0;i<n;i++)v=(v<<1)|bits[i];return v;}
#include "header-extract.c"
int main(void) {
 for(int fec=0;fec<3;fec++)for(int code=0;code<8;code++) {
  ClassAttribContent attr={0},esc={0};EPFrameAttrib frame={0};EPConfig cfg={0};
  frame.ClassCount=1;frame.TotalIninf=3;frame.cattr=&attr;frame.cattresc=&esc;
  attr.FECType=fec;esc.ClassCodeRate=1;cfg.NPred=1;cfg.NConcat=1;cfg.fattr=&frame;
  int bits[3]={(code>>2)&1,(code>>1)&1,code&1},pred=-1,stuff=-1;
  int consumed=RetrieveHeader(&cfg,&pred,bits,&stuff);
  if(consumed!=3||pred!=0||stuff!=0)return 3;
  printf("%d %d %d\n",fec,code,attr.ClassCodeRate);
 }
 return 0;
}
'''
 with tempfile.TemporaryDirectory(prefix='fvid-ep-rs-escape-')as tmp:
  p=Path(tmp);(p/'header-extract.c').write_text(function);(p/'probe.c').write_text(driver);exe=p/'probe';subprocess.run(['cc','-O2','-I',str(source),str(p/'probe.c'),'-o',str(exe)],check=True);output=subprocess.run([str(exe)],check=True,capture_output=True,text=True).stdout
 rows=[dict(fec=f,code=c,rate=r)for f,c,r in (map(int,line.split())for line in output.splitlines())];table=[0,3,4,6,8,12,16,24];assert len(rows)==24 and all(r['rate']==table[r['code']]for r in rows)
 report=dict(scope='Original unmodified RetrieveHeader after already recovered header bits; no foreign FEC or full audio decode acceptance',source_file='audio/natural/epTool/src/eptool.c',source_sha256=sha(original),function_sha256=sha(function.encode()),source_changes=[],driver_sha256=sha(driver.encode()),header_fec_stub='Identity for recovered bits; original FECDecodeHeader not exercised',compiler=subprocess.run(['cc','--version'],capture_output=True,text=True,check=True).stdout,cases=rows)
 (ROOT/'docs/validation/aac-ep-rs-escaped-reference-2026-11-02.json').write_text(json.dumps(report,indent=2)+'\n');print('Original RetrieveHeader:',len(rows),'rate mappings agree')
if __name__=='__main__':run(sys.argv[1])
