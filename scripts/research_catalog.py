#!/usr/bin/env python3
"""Build a transparent, manually screened landscape; no repository code is executed."""
import json, pathlib, csv, collections, datetime
ROOT=pathlib.Path(__file__).resolve().parents[1]
rows=json.load(open(ROOT/'research/candidates.json'))
# Indices refer to the frozen, alphabetically ordered candidates.json snapshot.
EXCLUDE={}
def reject(ids,reason):
 for i in map(int,ids.split()): EXCLUDE[i]=reason
reject('12 21 22 26 34 35 49 67 71 83 89 104 107 108 125 129 130 136 137 157 159 160 167 168 170 171 184 190 192 197 201 208 216 217 221 223 224 235 250 262 267 268 279 284 285 286 287 299 300 302 305 306 313 319 330 331 333 334 353 355 357 358 364 370 373 374 375 377 378 379 382 392 401 408 411 413 414 421 423 429 430 431 432 434 443 451 453 454 457 464 475 480 481 482 484 485 490 493 497 498 499 501 505 506 509 513 514 519 524 533 534 536 542 545 546 548 549 553 554 555 556 557 558 565 569 575 584 588 589 591 597 600 601 603 610 611 612 613 621 634 635 638 643 648 650 655 659 661 662 663 665 667 672 682 687 688 700 721 723 724 726 727 731 737 739 743 746 751 755 764 774 775 778 783 786 790 796 797 799 802 806 810 813 817 823 826 828 833 837 838 843 847 850 856 861 869 872 874 875 877 880 883 890 893 900 903 904 907 913 915 916 918 921 922 923 924 934 935 937 944 954 955 956 960 962 963 970 971 973 976 978 984 990 991 993 994 998 999 1002 1004 1005 1008 1010 1012 1014 1015 1017 1019 1029 1030', 'outside_engine_scope_or_weak_relevance')
reject('0 1 17 20 24 25 28 32 36 39 42 44 56 61 65 68 72 75 78 79 81 82 96 102 103 111 124 127 128 131 132 135 141 144 146 147 154 163 164 169 173 178 179 180 182 187 189 193 207 210 212 218 220 229 232 238 240 244 247 248 254 257 258 265 272 289 301 304 308 311 318 321 322 337 338 339 340 341 342 343 345 346 348 349 351 361 366 372 387 389 393 404 409 412 424 428 444 466 467 477 488 489 504 517 537 543 550 551 552 559 562 568 570 579 592 596 599 602 607 619 620 624 625 626 627 630 631 642 644 647 654 657 669 679 684 695 703 717 729 733 738 742 749 756 759 766 767 777 784 801 809 814 819 820 829 834 852 853 857 859 864 868 882 884 887 889 896 897 899 910 911 912 919 927 930 932 939 940 948 949 958 959 961 972 981 986 987 997 1020 1021 1023 1024 1027 1028', 'demo_build_packaging_duplicate_lineage_or_peripheral_application')
# Metadata screening only; architecture facts require individual source inspection.
def category(r):
 s=(r['full_name']+' '+(r['description'] or '')).lower()
 if any(x in s for x in ['binding','wrapper','interface to','based on ffmpeg','ffmpeg gui','gui for ffmpeg','ffmpeg-based']): return 'integration_wrapper'
 if any(x in s for x in ['editor','editing','composit','timeline']): return 'editor_compositor'
 if any(x in s for x in ['mux','mp4','matroska','metadata','tag reader','tag reading','format demux']): return 'container_metadata'
 if any(x in s for x in ['transcod','convert','compression tool']): return 'transcoding_product'
 if any(x in s for x in ['codec','encoder','decoding','encoding']): return 'codec_hardware'
 if any(x in s for x in ['audio','sound','dsp','synthesi','music']): return 'audio_dsp'
 if any(x in s for x in ['webrtc','stream','rtsp','rtmp','media server','sfu']): return 'streaming_transport'
 if any(x in s for x in ['player','playback']): return 'playback'
 return 'processing_framework'
selected=[]; excluded=[]
for i,r in enumerate(rows):
 entry={k:r.get(k) for k in ['full_name','html_url','description','language','stargazers_count','archived','pushed_at','default_branch']}
 entry.update(candidate_index=i,queries=r['queries'],license=(r.get('license') or {}).get('spdx_id','UNKNOWN'),category=category(r),review_level='metadata_screened',evidence='research/candidates.json',inclusion_basis=r.get('description') or r['full_name'])
 if i in EXCLUDE: entry['exclusion_reason']=EXCLUDE[i];excluded.append(entry)
 else: selected.append(entry)
json.dump(selected,open(ROOT/'research/catalog.json','w'),ensure_ascii=False,indent=2)
json.dump(excluded,open(ROOT/'research/excluded.json','w'),ensure_ascii=False,indent=2)
with open(ROOT/'research/catalog.csv','w') as f:
 w=csv.DictWriter(f,fieldnames=list(selected[0]));w.writeheader();w.writerows(selected)
print('selected',len(selected),'excluded',len(excluded));print(collections.Counter(r['category'] for r in selected))
