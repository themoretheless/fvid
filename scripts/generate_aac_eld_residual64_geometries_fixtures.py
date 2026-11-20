#!/usr/bin/env python3
"""Own residual64 extended integral geometries and parameter grids,offline."""
import generate_aac_eld_residual64_multicore_fixtures as base
if __name__=='__main__':
 base.main(base.FIX/'aac-eld-residual64-geometries',((480,2),(480,4),(512,2),(512,3),(512,4)),(4,5,7,9,12,15,23),48)
