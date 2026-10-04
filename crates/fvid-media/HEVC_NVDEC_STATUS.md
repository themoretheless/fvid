# Native HEVC NVDEC adapter status

`owned_nvdec_hevc::configuration` translates FVid-owned SPS/PPS data into
the pinned NVDEC HEVC configuration structure for 8-bit and 10-bit 4:2:0.
Host tests use the checked-in synthetic Main/Main10 fixtures and verify
parameter mapping, scaling-list conversion, tile bounds and reference sentinels.
They require neither FFmpeg nor an NVIDIA GPU.

`HevcPicture` additionally owns slice bytes and driver offsets, supplies
slice-local RPS bit accounting from the FVid parser, and resolves short-term
references against caller-owned live slots. Host tests prepare IDR and inter
pictures from both synthetic files and reject missing or aliased reference
slots and oversized bitstreams. This does not verify hardware image output.

`HevcNvdecDecoder` now schedules POC and short-term references, retaining decode
slots with DPB references, display tickets and mappings. State commits only after
successful driver submission; driver failure requires reopening the decoder.
POC derivation is shared with the own software decoder. Host tests compare all
synthetic Main/Main10 I/P/B picture orders and verify backpressure and aborted
submission leave POC/DPB unchanged. In-band parameter changes are refused until
reconfiguration is implemented.

`HevcMp4Input` reads and qualifies the owned sample table, preserving raw
PTS/DTS/duration/sync flags and metadata. AVC and HEVC use the same edit-timeline
mapper and metadata builder. HEVC input tests cover Main/Main10 packet reads,
rewind after qualification and the synthetic blank/repeated-range fixture.
`MovieSource` dispatches AVC and HEVC into the shared `MovieReader` and
`MovieRenderer`. The production native route now accepts qualified eight-bit
HEVC MP4 into the own NV12 filter, NVENC H.264 and Matroska exporter, including
explicit blanks and repeated edits. Host tests prove route admission and syntax
qualification; they do not prove hardware image correctness or performance.
Hidden output/RASL pictures are removed from presentation events and remain
decoder preroll. Mid-stream prior-output suppression, in-band parameter changes
and field sequences remain explicitly unqualified.
Long-term references are still refused by the own slice parser. Main10 can now
render through owned P010 buffers/processors, including blank/repeated edits and
trusted shaders. NVENC Main10 initialization/registration and owned HEVC
bitstream/container export remain pending, so production export still refuses
Main10 before opening GPU resources. Driver acceptance and decoded image
correctness have not been verified on NVIDIA hardware.

The ignored Linux/Windows test
`synthetic_owned_hevc_idr_submits_and_maps_on_nvidia` submits and maps the first
IDR from each synthetic fixture. It is a driver smoke test, not a pixel
comparison or a complete inter-picture playback test, and has not been run here.
The additional ignored `owned_hevc_ipb_scheduler_submits_and_maps_on_nvidia`
test submits/maps all synthetic Main/Main10 pictures, including references and
reordered B pictures. It also does not compare GPU pixels with a reference.
Both hardware tests are required by the CUDA qualification runner.
The runner also requires `own_hevc_mp4_packets_decode_on_nvidia_without_libav`,
which connects the own MP4 input to the decoder and maps every synthetic picture.
`production_hw_filter_routes_hevc_movie_without_libav` checks full native export,
movie timestamps and decoding of all exported H.264 packets by FVid, using both
the Main I/P/B seed and the blank/repeated-range fixture. It is required by the
qualification runner but has not been executed here.

The production CUDA features still include the legacy libav backend. This
adapter alone does not remove that dependency.
