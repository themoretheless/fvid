// Optional external-reference driver. Production never links this executable.
// FVIDAAC1: magic, u32le ASC length, ASC, u32le AU count, then length/AU pairs.
#include "aacdecoder_lib.h"
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <memory>
#include <stdexcept>
#include <vector>

static uint32_t word(FILE *in) {
  unsigned char b[4];
  if (fread(b, 1, sizeof(b), in) != sizeof(b))
    throw std::runtime_error("truncated FVIDAAC1 record");
  return uint32_t(b[0]) | uint32_t(b[1]) << 8 |
         uint32_t(b[2]) << 16 | uint32_t(b[3]) << 24;
}
static std::vector<UCHAR> bytes(FILE *in, uint32_t size, uint32_t limit) {
  if (!size || size > limit)
    throw std::runtime_error("reference record exceeds admission bound");
  std::vector<UCHAR> result(size);
  if (fread(result.data(), 1, size, in) != size)
    throw std::runtime_error("truncated FVIDAAC1 payload");
  return result;
}
struct Decoder {
  HANDLE_AACDECODER value = aacDecoder_Open(TT_MP4_RAW, 1);
  ~Decoder() { if (value) aacDecoder_Close(value); }
};
static void check(AAC_DECODER_ERROR error, const char *stage, uint32_t frame) {
  if (error != AAC_DEC_OK) {
    fprintf(stderr, "%s error=0x%x AU=%u\n", stage, unsigned(error), frame);
    throw std::runtime_error("external FDK reference refused input");
  }
}
int main(int argc, char **argv) {
  if (argc != 3) return 2;
  try {
    static_assert(sizeof(INT_PCM) == 2, "probe requires PCM16 FDK build");
    using File = std::unique_ptr<FILE, int (*)(FILE *)>;
    File in(fopen(argv[1], "rb"), fclose), out(fopen(argv[2], "wb"), fclose);
    if (!in || !out) throw std::runtime_error("cannot open probe files");
    char magic[8];
    if (fread(magic, 1, 8, in.get()) != 8 || memcmp(magic, "FVIDAAC1", 8))
      throw std::runtime_error("invalid FVIDAAC1 magic");
    auto asc = bytes(in.get(), word(in.get()), 65536);
    Decoder decoder;
    if (!decoder.value) throw std::runtime_error("FDK allocation failed");
    UCHAR *pointer = asc.data();
    UINT size = UINT(asc.size());
    check(aacDecoder_ConfigRaw(decoder.value, &pointer, &size), "config", 0);
    check(aacDecoder_SetParam(decoder.value, AAC_PCM_OUTPUT_CHANNEL_MAPPING, 1),
          "WAVE channel order", 0);
    const uint32_t count = word(in.get());
    if (!count || count > 1000000) throw std::runtime_error("invalid AU count");
    std::vector<INT_PCM> pcm(32768);
    uint64_t total = 0;
    for (uint32_t i = 0; i < count; ++i) {
      auto packet = bytes(in.get(), word(in.get()), 1048576);
      pointer = packet.data();
      UINT valid = size = UINT(packet.size());
      check(aacDecoder_Fill(decoder.value, &pointer, &size, &valid), "fill", i);
      if (valid) throw std::runtime_error("FDK did not consume entire AU");
      // No concealment/flush flags or replacement output on decoder errors.
      check(aacDecoder_DecodeFrame(decoder.value, pcm.data(), INT(pcm.size()), 0),
            "decode", i);
      const auto *info = aacDecoder_GetStreamInfo(decoder.value);
      if (!info || info->frameSize <= 0 || info->numChannels <= 0 ||
          uint64_t(info->frameSize) * info->numChannels > pcm.size())
        throw std::runtime_error("invalid external PCM geometry");
      const size_t scalars = size_t(info->frameSize) * info->numChannels;
      if (i == 0)
        printf("rate=%d channels=%d frame=%d outputDelay=%u pcm_bits=16\n",
               info->sampleRate, info->numChannels, info->frameSize, info->outputDelay);
      std::vector<unsigned char> encoded(scalars * 2);
      for (size_t j = 0; j < scalars; ++j) {
        const uint16_t value = uint16_t(pcm[j]);
        encoded[2*j] = value & 255;
        encoded[2*j+1] = value >> 8;
      }
      if (fwrite(encoded.data(), 1, encoded.size(), out.get()) != encoded.size())
        throw std::runtime_error("external PCM write failed");
      total += scalars;
    }
    if (fgetc(in.get()) != EOF || ferror(in.get()))
      throw std::runtime_error("trailing/error bytes in FVIDAAC1 input");
    if (fflush(out.get())) throw std::runtime_error("external PCM flush failed");
    printf("packets=%u scalars=%llu\n", count, (unsigned long long)total);
    return 0;
  } catch (const std::exception &error) {
    fprintf(stderr, "%s\n", error.what());
    return 1;
  }
}
