# Screen Sharing Latency Analysis

This document records our investigation into screen sharing latency, aiming for Parsec-like game streaming performance.

## Goal

Achieve minimal latency (~10-25ms) between:
- Something happening on Alice's screen
- Bob seeing it in his client

## Current Architecture

```
Alice: Screen Capture → Encode (H.264) → RTP → WebRTC → Network
Server: WebRTC → RTP Forward → WebRTC
Bob: Network → WebRTC → RTP → Decode (H.264) → UI Display
```

## Optimizations Implemented

### Screen Capture (screen.rs)
- Leaky queue: `max-size-buffers=1 leaky=downstream`
- No sync: `sync=false` on appsink
- Drop old frames: `drop=true`, `max-buffers=1`
- Short timeout: 5ms `try_pull_sample`

### Encoder (gst_encoder.rs)
- Hardware encoding: `vtenc_h264_hw` (VideoToolbox)
- Low latency settings: `realtime=true`, `quality=0.5`
- No B-frames: `allow-frame-reordering=false`
- Keyframe interval: 30 frames
- Zero-timeout drain: Pull all available frames, return only latest
- No pipeline queues between elements

### Decoder (gst_encoder.rs)
- Hardware decoding: `vtdec_hw` (VideoToolbox)
- Minimal pipeline: `appsrc → h264parse → vtdec_hw → videoconvert → appsink`
- No pipeline clock: `pipeline.use_clock(None)`
- Zero-timeout drain: Pull all available frames, return only latest
- No sync: `sync=false`, `drop=true`, `max-buffers=1`

### WebRTC (sfu_client.rs)
- Empty interceptor registry (no NACK, no jitter buffer)
- Direct RTP forwarding on server (no transcoding)

### Server (track_router.rs)
- Direct RTP packet forwarding
- No buffering or processing

## Latency Measurement Points

Added `[LATENCY]` tagged logs at each stage:

| Tag | Location | Description |
|-----|----------|-------------|
| `CAPTURE` | screen.rs | Frame captured from screen |
| `ENCODE_IN` | gst_encoder.rs | Frame enters encoder |
| `ENCODE_OUT` | gst_encoder.rs | Encoded data ready |
| `RTP_SEND` | sfu_client.rs | Before RTP packet send |
| `SERVER_FWD` | track_router.rs | Server forwards packet |
| `RTP_RECV` | sfu_client.rs | Client receives RTP |
| `DECODE_OUT` | sfu_client.rs | Decoded frame ready |
| `UI_DISPLAY` | voice_channel_view.rs | Frame displayed in UI |

## Observed Latency (~1 second total)

### What's Fast
- **Hardware encode**: ~0ms (VideoToolbox is instant)
- **Server forwarding**: ~3ms (direct passthrough)
- **Hardware decode**: ~1ms (VideoToolbox)
- **Network** (localhost): negligible

### Suspected Bottlenecks

1. **UI Loop Coupling**
   - Video processing is tied to `show()` function which runs at ~30fps
   - Frames may wait in buffers until next UI poll
   - Expected: 33ms, but measurements suggest possibly more

2. **Async Task Scheduling**
   - `runtime.spawn()` for sending frames
   - `runtime.block_on()` calls in UI thread may cause contention
   - Could delay async task execution

3. **webrtc-rs Internal Buffering**
   - DTLS, ICE, RTP packetization layers
   - Not directly controllable
   - Library designed for general WebRTC, not game streaming

4. **VideoToolbox Internal Buffering**
   - Hardware codec may have frame buffers for quality
   - No exposed "ultra low latency" mode like NVENC
   - Cannot disable internal buffering

5. **Counter Mismatch in Measurements**
   - Each measurement point has independent frame counter
   - "frame=120" at CAPTURE vs ENCODE may not be same actual frame
   - Makes precise latency correlation difficult

## Sample Log Analysis

```
CAPTURE  frame=120 ts=1767963491455
ENCODE_IN  frame=120 ts=1767963491632  (+177ms?)
ENCODE_OUT frame=120 ts=1767963491632  (+0ms encode)
SERVER_FWD pkt=1501 ts=1767963491543
RTP_RECV   pkt=451 ts=1767963491546   (+3ms from server)
DECODE_OUT pkt=661 ts=1767963492464   (~instant)
UI_DISPLAY frame=30 ts=1767963492206
```

Note: Counter misalignment makes frame-to-frame correlation imprecise.

## What Would Be Needed for Parsec-Like Latency

1. **Dedicated Threads**
   - Separate capture thread (not tied to UI)
   - Separate encode thread
   - Separate network send thread
   - Avoid any UI loop coupling

2. **Custom RTP/UDP Implementation**
   - Bypass webrtc-rs for video data
   - Use raw UDP sockets
   - Implement minimal custom RTP (no jitter buffer)
   - Keep WebRTC only for signaling

3. **NVENC/AMF with Low-Latency Presets**
   - NVIDIA NVENC has explicit "ultra low latency" mode
   - AMD AMF has similar options
   - VideoToolbox doesn't expose such controls

4. **Frame Pacing and Prediction**
   - Client-side frame interpolation
   - Motion prediction to hide latency
   - Adaptive bitrate based on RTT

## Conclusion

The current ~1 second latency is likely coming from:
1. webrtc-rs internal buffering (largest contributor)
2. UI loop coupling (adds 33ms+ per stage)
3. VideoToolbox internal buffering (unknown amount)

Achieving Parsec-like latency would require architectural changes:
- Moving away from webrtc-rs for video transport
- Decoupling video processing from UI thread
- Using dedicated low-latency codec modes

These changes are beyond scope for a Discord clone but documented here for future reference.

## Files Modified for Low Latency

- `crates/miscord-client/src/media/screen.rs` - Screen capture pipeline
- `crates/miscord-client/src/media/gst_encoder.rs` - H.264 encoder/decoder
- `crates/miscord-client/src/media/sfu_client.rs` - WebRTC client, RTP handling
- `crates/miscord-server/src/sfu/track_router.rs` - Server-side RTP forwarding
- `crates/miscord-client/src/ui/voice_channel_view.rs` - UI display loop
