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
#define YOAKE_MAX_FRAME_BYTES 8388608u
#define YOAKE_MAX_SETTINGS_BUTTONS 1024u
#define YOAKE_NET_LOGIN 1u
#define YOAKE_NET_CONFIGURATION 2u
#define YOAKE_NET_PLAY 3u
#define YOAKE_HAND_NONE 0u
#define YOAKE_HAND_MAIN 1u
#define YOAKE_HAND_OFF 2u
#define YOAKE_PLAYER_ON_GROUND (1u << 0)
#define YOAKE_PLAYER_SPRINTING (1u << 1)
#define YOAKE_PLAYER_SWIMMING (1u << 2)
#define YOAKE_PLAYER_FALL_FLYING (1u << 3)
#define YOAKE_PLAYER_HAS_VEHICLE (1u << 4)
#define YOAKE_PLAYER_MAIN_HAND_RIGHT (1u << 5)
#define YOAKE_PLAYER_IN_WATER (1u << 6)
#define YOAKE_PLAYER_EYE_IN_WATER (1u << 7)
#define YOAKE_PLAYER_FLYING (1u << 8)
#define YOAKE_PLAYER_HAS_MAX_HEALTH (1u << 9)
#define YOAKE_TEXTURE_RGBA8 1u
#define YOAKE_TEXTURE_BGRA8 2u
#define YOAKE_TEXTURE_RGBA16_FLOAT 3u
#define YOAKE_FILTER_NEAREST 1u
#define YOAKE_FILTER_LINEAR 2u
typedef struct YoakeFrameEvent { uint64_t elapsed_ns; } YoakeFrameEvent;
typedef struct YoakeNetFramesListenedEvent { uint32_t *listened; } YoakeNetFramesListenedEvent;
typedef struct YoakeNetFrameReceived {
    uint32_t state;
    int32_t packet_id;
    YoakeByteView body;
    uint64_t monotonic_ns, session;
} YoakeNetFrameReceived;
typedef struct YoakeNetSessionEndedEvent { uint64_t session; YoakeStrView reason; } YoakeNetSessionEndedEvent;
typedef struct YoakeGuiLayerEvent { YoakeStrView layer; YoakeHandle batch; } YoakeGuiLayerEvent;
typedef struct YoakeScreenView {
    YoakeStrView id;
    YoakeRect rect;
    YoakePoint mouse;
    float partial_tick;
} YoakeScreenView;
typedef struct YoakeScreenRenderEvent { YoakeScreenView screen; YoakeHandle batch; } YoakeScreenRenderEvent;
typedef struct YoakeScreenKeyEvent {
    YoakeScreenView screen;
    YoakeStrView key;
    uint32_t repeat, modifiers;
} YoakeScreenKeyEvent;
typedef struct YoakeScreenCharEvent {
    YoakeScreenView screen;
    uint32_t character, modifiers;
} YoakeScreenCharEvent;
typedef struct YoakeScreenMouseButtonEvent {
    YoakeScreenView screen;
    int32_t button;
    uint32_t modifiers;
} YoakeScreenMouseButtonEvent;
typedef struct YoakeScreenScrollEvent {
    YoakeScreenView screen;
    double scroll[2];
    uint32_t modifiers;
} YoakeScreenScrollEvent;
typedef YoakeStatus (*YoakeSettingsPush)(void *, YoakeStrView);
typedef struct YoakeSettingsEvent { YoakeStrView mod_id; YoakeSettingsPush push; void *user; } YoakeSettingsEvent;
typedef struct YoakeSettingsClickEvent { YoakeStrView mod_id; size_t button; } YoakeSettingsClickEvent;
typedef struct YoakeLocalPlayerSnapshot {
    int32_t entity_id;
    uint8_t uuid[16];
    double pos[3];
    float yaw, pitch, head_yaw, body_yaw;
    uint32_t flags, pose, swing_hand, swing_ticks, using_hand;
    int32_t use_ticks;
    YoakeByteView equipment[6];
    int32_t vehicle;
    uint32_t skin_parts;
    YoakeStrView dimension;
    uint32_t game_mode;
    float health, max_health;
    int32_t food_level;
} YoakeLocalPlayerSnapshot;
typedef struct YoakeSnapshotReport {
    size_t written;
    const YoakeStrView *missing;
    size_t missing_len;
} YoakeSnapshotReport;
typedef struct YoakeAccount { YoakeStrView name; uint8_t uuid[16]; uint32_t online; } YoakeAccount;
typedef struct YoakeGuiNineSlice { uint32_t source_size[2], border[4]; } YoakeGuiNineSlice;
typedef YoakeStatus (*YoakePlayerCallback)(void *, const YoakeLocalPlayerSnapshot *);
typedef YoakeStatus (*YoakeSnapshotFrameCallback)(void *, uint32_t, int32_t, YoakeByteView);
typedef YoakeStatus (*YoakeSnapshotReportCallback)(void *, const YoakeSnapshotReport *);
typedef YoakeStatus (*YoakeAccountCallback)(void *, const YoakeAccount *);
