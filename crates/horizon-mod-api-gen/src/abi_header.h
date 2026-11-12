#ifndef YOAKE_MOD_ABI_H
#define YOAKE_MOD_ABI_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef uint32_t YoakeStatus;
#define YOAKE_OK 0u
#define YOAKE_INVALID 1u
#define YOAKE_BUFFER_TOO_SMALL 2u
#define YOAKE_NOT_FOUND 3u
#define YOAKE_UNSUPPORTED 4u
#define YOAKE_DISABLED 5u
#define YOAKE_PANIC 6u
#define YOAKE_HOST_ERROR 7u
#define YOAKE_BUSY 8u
#define YOAKE_CONTINUE 0u
#define YOAKE_CANCEL 1u
#define YOAKE_REPLACE 2u
#define YOAKE_SIDE_CLIENT 2u
#define YOAKE_WORLD_SINGLEPLAYER 1u
#define YOAKE_WORLD_SERVER 2u
#define YOAKE_WORLD_LAN 3u
#define YOAKE_WORLD_FRIEND 4u
#define YOAKE_WORLD_REPLAY 5u
typedef struct YoakeTableHeader {
    uint32_t abi_version, size;
    uint64_t schema_hash;
} YoakeTableHeader;
typedef struct YoakeHandle { uint64_t id, generation; } YoakeHandle;
typedef struct YoakeByteView { const uint8_t *ptr; size_t len; } YoakeByteView;
typedef YoakeByteView YoakeStrView;
typedef struct YoakeOutputBuffer {
    uint8_t *ptr;
    size_t capacity, len;
} YoakeOutputBuffer;
typedef struct YoakePoint { float x, y; } YoakePoint;
typedef struct YoakeRect { float x, y, width, height; } YoakeRect;
typedef struct YoakeHookResult { YoakeStatus status; uint32_t flow; } YoakeHookResult;
typedef struct YoakeTickEvent { uint32_t reserved; } YoakeTickEvent;
typedef struct YoakeKeyEvent { YoakeStrView key; uint32_t pressed; } YoakeKeyEvent;
typedef struct YoakeHudEvent { YoakeHandle batch; } YoakeHudEvent;
typedef struct YoakeNetEvent { YoakeStrView channel; YoakeByteView data; } YoakeNetEvent;
