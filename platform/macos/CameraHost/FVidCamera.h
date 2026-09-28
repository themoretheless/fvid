#ifndef FVID_CAMERA_H
#define FVID_CAMERA_H
#include <stddef.h>
#include <stdint.h>
// All operations on a handle must be serialized. Close invalidates the handle.
typedef struct CameraSource FVidCameraSource;
typedef struct { uint32_t width; uint32_t height; } FVidCameraSize;
FVidCameraSource *fvid_camera_open(const uint8_t *path, size_t length, size_t budget);
// Zero means duration unknown. Serialized with other source operations.
uint64_t fvid_camera_duration(const FVidCameraSource *source);
FVidCameraSize fvid_camera_size(const FVidCameraSource *source);
// 1 = copied, -1 = error (close/reopen). Output must have width*height*4 bytes.
int32_t fvid_camera_frame(FVidCameraSource *source, uint64_t media_ns, uint64_t host_ns,
                         uint64_t sequence, uint8_t *output, size_t length);
void fvid_camera_close(FVidCameraSource *source);
int32_t fvid_camera_fit(const uint8_t *input, size_t input_len, FVidCameraSize source,
                        uint8_t *output, size_t output_len, FVidCameraSize target);
typedef struct CameraClock FVidCameraClock;
typedef struct { uint64_t sequence; uint64_t host_ns; uint64_t media_ns; } FVidCameraTick;
FVidCameraClock *fvid_camera_clock_open(uint64_t now);
int32_t fvid_camera_clock_poll(FVidCameraClock *clock, uint64_t now, FVidCameraTick *output);
// 0 seek (ns), 1 pause (boolean), 2 loop duration (ns, 0 disables).
int32_t fvid_camera_clock_control(FVidCameraClock *clock, uint32_t command, uint64_t value, uint64_t now);
void fvid_camera_clock_close(FVidCameraClock *clock);
#endif
