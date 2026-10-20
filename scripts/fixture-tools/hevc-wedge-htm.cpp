// FVid validation harness; external HTM algorithm sources remain unchanged.
#include "TComRom.h"
#include "TComWedgelet.h"
#include <fstream>
#include <iostream>
int main(int argc, char **argv){
  if(argc!=2)return 2;
  const std::string directory=argv[1];
  initROM();
  for(unsigned size: {4,8,16}) {
    std::vector<TComWedgelet> list;
    std::vector<TComWedgeRef> refs;
    createWedgeList(size,size,list,refs,size<16?HALF_PEL:FULL_PEL);
    std::ofstream out(directory+"/"+std::to_string(size)+".bin",std::ios::binary);
    for(auto &w:list) for(unsigned i=0;i<size*size;i++)out.put(w.getPattern()[i]?1:0);
    std::cout<<size<<" "<<list.size()<<"\n";
    if(size==16) {
      std::ofstream large(directory+"/32.bin",std::ios::binary);
      for(auto &w:list) { Bool scaled[1024]; w.getPatternScaledCopy(32,scaled); for(auto bit:scaled)large.put(bit?1:0); }
    }

  }
}
