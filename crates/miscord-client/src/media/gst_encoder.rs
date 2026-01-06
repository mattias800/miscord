//! GStreamer-based H.264 hardware encoder and decoder for WebRTC video streaming
//!
//! Uses hardware-accelerated H.264 encoding/decoding on all platforms.
//! Falls back is NOT supported - hardware encoding is mandatory for usable performance.
//!
//! Platform support:
//! - macOS: VideoToolbox (vtenc_h264 / vtdec_h264)
//! - Linux: VAAPI (Intel/AMD), NVENC (Nvidia)
//! - Windows: NVENC, AMF, QuickSync, MediaFoundation

use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::gst_video::VideoFrame;

/// Detect which hardware H.264 encoder is available on this system
fn detect_hw_encoder() -> Result<&'static str> {
    gst::init()?;

    // List of encoders to try in order of preference
    #[cfg(target_os = "macos")]
    let encoders = vec![
        ("vtenc_h264_hw", "VideoToolbox H.264 (hardware only)"),
        ("vtenc_h264", "VideoToolbox H.264"),  // Fallback that may use software
    ];

    #[cfg(target_os = "linux")]
    let encoders = vec![
        ("nvh264enc", "NVIDIA NVENC H.264"),
        ("vaapih264enc", "VAAPI H.264 (Intel/AMD)"),
    ];

    #[cfg(target_os = "windows")]
    let encoders = vec![
        ("nvh264enc", "NVIDIA NVENC H.264"),
        ("amfh264enc", "AMD AMF H.264"),
        ("qsvh264enc", "Intel QuickSync H.264"),
        ("mfh264enc", "MediaFoundation H.264"),
    ];

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let encoders: Vec<(&str, &str)> = vec![];

    for (element_name, description) in &encoders {
        if gst::ElementFactory::find(element_name).is_some() {
            tracing::info!("Found hardware encoder: {} ({})", description, element_name);
            return Ok(*element_name);
        }
    }

    Err(anyhow!(
        "No hardware H.264 encoder found. Hardware encoding is required for screen sharing. \
         Please ensure you have the appropriate GStreamer plugins installed:\n\
         - macOS: gst-plugins-bad (for vtenc_h264)\n\
         - Linux (NVIDIA): gst-plugins-bad (for nvh264enc)\n\
         - Linux (Intel/AMD): gstreamer-vaapi (for vaapih264enc)\n\
         - Windows: gst-plugins-bad (for nvh264enc, amfh264enc, qsvh264enc)"
    ))
}

/// Detect which hardware H.264 decoder is available on this system
fn detect_hw_decoder() -> Result<&'static str> {
    gst::init()?;

    #[cfg(target_os = "macos")]
    let decoders = vec![
        ("vtdec_hw", "VideoToolbox (hardware only)"),
        ("vtdec", "VideoToolbox (generic)"),
    ];

    #[cfg(target_os = "linux")]
    let decoders = vec![
        ("nvh264dec", "NVIDIA NVDEC H.264"),
        ("vaapih264dec", "VAAPI H.264 (Intel/AMD)"),
    ];

    #[cfg(target_os = "windows")]
    let decoders = vec![
        ("nvh264dec", "NVIDIA NVDEC H.264"),
        ("d3d11h264dec", "Direct3D 11 H.264"),
        ("mfh264dec", "MediaFoundation H.264"),
    ];

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let decoders: Vec<(&str, &str)> = vec![];

    for (element_name, description) in &decoders {
        if gst::ElementFactory::find(element_name).is_some() {
            tracing::info!("Found hardware decoder: {} ({})", description, element_name);
            return Ok(*element_name);
        }
    }

    Err(anyhow!(
        "No hardware H.264 decoder found. Hardware decoding is required for video playback."
    ))
}

/// Build encoder-specific pipeline segment
fn build_encoder_pipeline(encoder: &str, bitrate: u32) -> String {
    match encoder {
        "vtenc_h264_hw" | "vtenc_h264" => {
            // VideoToolbox encoder (macOS)
            // vtenc_h264_hw is hardware-only, vtenc_h264 may fall back to software
            // allow-frame-reordering=false disables B-frames for lower latency
            // max-keyframe-interval ensures keyframes every second
            // name=encoder allows us to find it later for force-keyframe
            // h264parse config-interval=-1 inserts SPS/PPS before every IDR
            // stream-format=byte-stream ensures Annex B output (start codes, not length prefixed)
            format!(
                "{} name=encoder allow-frame-reordering=false max-keyframe-interval=30 \
                 max-keyframe-interval-duration=1000000000 bitrate={} ! \
                 h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au",
                encoder,
                bitrate / 1000  // vtenc uses kbps
            )
        }
        "nvh264enc" => {
            // NVIDIA NVENC
            // preset=low-latency-hq for good quality with low latency
            // rc-mode=cbr for constant bitrate
            format!(
                "nvh264enc name=encoder preset=low-latency-hq rc-mode=cbr bitrate={} zerolatency=true ! \
                 h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au",
                bitrate / 1000
            )
        }
        "vaapih264enc" => {
            // VAAPI encoder (Intel/AMD on Linux)
            format!(
                "vaapih264enc name=encoder rate-control=cbr bitrate={} keyframe-period=60 ! \
                 h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au",
                bitrate / 1000
            )
        }
        "amfh264enc" => {
            // AMD AMF encoder (Windows)
            format!(
                "amfh264enc name=encoder bitrate={} rate-control=cbr ! \
                 h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au",
                bitrate / 1000
            )
        }
        "qsvh264enc" => {
            // Intel QuickSync (Windows)
            format!(
                "qsvh264enc name=encoder bitrate={} rate-control=cbr low-latency=true ! \
                 h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au",
                bitrate / 1000
            )
        }
        "mfh264enc" => {
            // MediaFoundation (Windows fallback)
            format!(
                "mfh264enc name=encoder bitrate={} ! \
                 h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au",
                bitrate
            )
        }
        _ => {
            // Generic fallback format
            format!(
                "{} name=encoder bitrate={} ! h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au",
                encoder,
                bitrate / 1000
            )
        }
    }
}

/// Build decoder-specific pipeline segment
fn build_decoder_pipeline(decoder: &str) -> String {
    match decoder {
        "vtdec_hw" | "vtdec" => {
            // VideoToolbox decoder (macOS)
            // vtdec_hw is hardware-only, vtdec may fall back to software
            decoder.to_string()
        }
        "nvh264dec" => {
            // NVIDIA NVDEC
            "nvh264dec".to_string()
        }
        "vaapih264dec" => {
            // VAAPI decoder
            "vaapih264dec".to_string()
        }
        "d3d11h264dec" => {
            // Direct3D 11 decoder (Windows)
            "d3d11h264dec".to_string()
        }
        "mfh264dec" => {
            // MediaFoundation decoder (Windows)
            "mfh264dec".to_string()
        }
        _ => decoder.to_string(),
    }
}

/// H.264 hardware encoder using GStreamer
pub struct GstH264Encoder {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
    encoder: gst::Element,
    width: u32,
    height: u32,
    fps: u32,
    encoder_name: String,
    is_running: Arc<AtomicBool>,
}

impl GstH264Encoder {
    /// Create a new H.264 hardware encoder for the given frame dimensions
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Self::new_with_fps(width, height, 30)
    }

    /// Create a new H.264 hardware encoder with custom framerate
    pub fn new_with_fps(width: u32, height: u32, fps: u32) -> Result<Self> {
        let encoder = detect_hw_encoder()?;

        // Calculate bitrate based on resolution
        let bitrate = Self::calculate_bitrate(width, height, fps);

        // Build the encoder segment
        let encoder_segment = build_encoder_pipeline(encoder, bitrate);

        // Full pipeline: appsrc → videoconvert → hw_encoder → h264parse → appsink
        // Note: Using I420 format which is widely supported by hardware encoders
        // and may have better hardware conversion paths than NV12 on some platforms
        let pipeline_str = format!(
            "appsrc name=src format=time is-live=true do-timestamp=true \
             caps=video/x-raw,format=RGB,width={},height={},framerate={}/1 ! \
             videoconvert ! video/x-raw,format=I420 ! \
             {} ! \
             appsink name=sink sync=false max-buffers=2 drop=true",
            width, height, fps, encoder_segment
        );

        tracing::info!(
            "Creating H.264 hardware encoder ({}) for {}x{} @{}fps, {}kbps: {}",
            encoder, width, height, fps, bitrate / 1000, pipeline_str
        );

        let pipeline = gst::parse::launch(&pipeline_str)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("Failed to downcast to Pipeline"))?;

        let appsrc = pipeline
            .by_name("src")
            .ok_or_else(|| anyhow!("Could not find appsrc"))?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| anyhow!("Failed to downcast to AppSrc"))?;

        let appsink = pipeline
            .by_name("sink")
            .ok_or_else(|| anyhow!("Could not find appsink"))?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| anyhow!("Failed to downcast to AppSink"))?;

        appsink.set_property("emit-signals", true);
        appsink.set_property("sync", false);

        // Start the pipeline
        pipeline.set_state(gst::State::Playing)?;

        // Wait for pipeline to initialize
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Check for errors
        if let Some(bus) = pipeline.bus() {
            while let Some(msg) = bus.pop() {
                use gst::MessageView;
                if let MessageView::Error(err) = msg.view() {
                    let _ = pipeline.set_state(gst::State::Null);
                    return Err(anyhow!(
                        "Hardware encoder ({}) failed: {} ({:?})",
                        encoder,
                        err.error(),
                        err.debug()
                    ));
                }
            }
        }

        // Get reference to the encoder element for force-keyframe support
        let encoder_element = pipeline
            .by_name("encoder")
            .ok_or_else(|| anyhow!("Failed to find encoder element in pipeline"))?;

        tracing::info!(
            "H.264 hardware encoder ({}) initialized for {}x{} @{}fps",
            encoder, width, height, fps
        );

        Ok(Self {
            pipeline,
            appsrc,
            appsink,
            encoder: encoder_element,
            width,
            height,
            fps,
            encoder_name: encoder.to_string(),
            is_running: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Calculate appropriate bitrate based on resolution and framerate
    fn calculate_bitrate(width: u32, height: u32, fps: u32) -> u32 {
        let pixels = width * height;
        let fps_factor = fps as f64 / 30.0;

        let base_bitrate = if pixels >= 3840 * 2160 {
            8_000_000  // 4K: 8 Mbps
        } else if pixels >= 2560 * 1440 {
            5_000_000  // 1440p: 5 Mbps
        } else if pixels >= 1920 * 1080 {
            3_000_000  // 1080p: 3 Mbps
        } else if pixels >= 1280 * 720 {
            1_500_000  // 720p: 1.5 Mbps
        } else {
            1_000_000  // Lower: 1 Mbps
        };

        (base_bitrate as f64 * fps_factor) as u32
    }

    /// Encode an RGB frame to H.264
    pub fn encode(&self, frame: &VideoFrame) -> Result<Vec<u8>> {
        if !self.is_running.load(Ordering::SeqCst) {
            return Err(anyhow!("Encoder not running"));
        }

        if frame.width != self.width || frame.height != self.height {
            return Err(anyhow!(
                "Frame size mismatch: expected {}x{}, got {}x{}",
                self.width, self.height, frame.width, frame.height
            ));
        }

        let expected_size = (self.width * self.height * 3) as usize;
        if frame.data.len() != expected_size {
            return Err(anyhow!(
                "Frame data size mismatch: expected {}, got {}",
                expected_size, frame.data.len()
            ));
        }

        let mut buffer = gst::Buffer::with_size(frame.data.len())?;
        {
            let buffer_ref = buffer.get_mut().ok_or_else(|| anyhow!("Failed to get buffer mut"))?;
            let mut map = buffer_ref.map_writable()?;
            map.copy_from_slice(&frame.data);
        }

        self.appsrc.push_buffer(buffer)?;

        match self.appsink.try_pull_sample(gst::ClockTime::from_mseconds(50)) {
            Some(sample) => {
                let buffer = sample.buffer().ok_or_else(|| anyhow!("No buffer in sample"))?;
                let map = buffer.map_readable()?;
                let data = map.as_slice().to_vec();

                // Log NAL types in the encoded frame for debugging
                static ENCODE_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let count = ENCODE_COUNT.fetch_add(1, Ordering::SeqCst);
                if count < 10 && !data.is_empty() {
                    // Find NAL units and log their types
                    let mut i = 0;
                    let mut nal_info = Vec::new();
                    while i < data.len().saturating_sub(4) {
                        // Look for start codes (00 00 00 01 or 00 00 01)
                        if data[i] == 0 && data[i+1] == 0 {
                            let start_code_len = if i + 3 < data.len() && data[i+2] == 0 && data[i+3] == 1 { 4 }
                                else if data[i+2] == 1 { 3 }
                                else { 0 };
                            if start_code_len > 0 {
                                let nal_start = i + start_code_len;
                                if nal_start < data.len() {
                                    let nal_header = data[nal_start];
                                    let nal_type = nal_header & 0x1f;
                                    nal_info.push(format!("type={}", nal_type));
                                }
                                i = nal_start;
                                continue;
                            }
                        }
                        i += 1;
                    }
                    tracing::info!("Encoded frame #{}: {} bytes, NALs: [{}]", count, data.len(), nal_info.join(", "));
                } else {
                    tracing::debug!("H.264 encoded frame: {} bytes", data.len());
                }
                Ok(data)
            }
            None => {
                tracing::debug!("H.264 encoder buffering");
                Ok(vec![])
            }
        }
    }

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn fps(&self) -> u32 { self.fps }
    pub fn encoder_name(&self) -> &str { &self.encoder_name }

    /// Force the encoder to generate a keyframe (IDR frame)
    /// This is needed when a new subscriber joins and needs to start decoding
    pub fn force_keyframe(&self) -> Result<()> {
        use gst::event::CustomDownstream;

        // Create a force-key-unit event
        // This tells the encoder to produce an IDR frame as soon as possible
        // The event must be sent downstream (towards the encoder) using CustomDownstream
        let structure = gst::Structure::builder("GstForceKeyUnit")
            .field("all-headers", true)
            .build();

        let event = CustomDownstream::new(structure);

        // Get the encoder's sink pad and send the event there
        if let Some(sink_pad) = self.encoder.static_pad("sink") {
            if sink_pad.send_event(event) {
                tracing::info!("Sent force-keyframe request to encoder via sink pad");
                return Ok(());
            }
        }

        // Fallback: try sending to encoder element directly
        let structure = gst::Structure::builder("GstForceKeyUnit")
            .field("all-headers", true)
            .build();
        let event = CustomDownstream::new(structure);

        if self.encoder.send_event(event) {
            tracing::info!("Sent force-keyframe request to encoder element");
            Ok(())
        } else {
            tracing::warn!("Failed to send force-keyframe event to encoder");
            Err(anyhow!("Failed to send force-keyframe event"))
        }
    }

    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        let _ = self.appsrc.end_of_stream();
        let _ = self.pipeline.set_state(gst::State::Null);
        tracing::info!("H.264 encoder stopped");
    }
}

impl Drop for GstH264Encoder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// H.264 hardware decoder using GStreamer
pub struct GstH264Decoder {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
    decoder_name: String,
    is_running: Arc<AtomicBool>,
    /// Buffer for reassembling FU-A fragmented NAL units
    fu_a_buffer: std::sync::Mutex<Vec<u8>>,
}

impl GstH264Decoder {
    /// Create a new H.264 hardware decoder
    pub fn new() -> Result<Self> {
        let decoder = detect_hw_decoder()?;

        let decoder_segment = build_decoder_pipeline(decoder);

        // Pipeline for RTP H.264: appsrc → rtph264depay → h264parse → hw_decoder → videoconvert → appsink
        // Note: We manually depayload RTP because GStreamer's rtph264depay has issues with
        // RTP header extensions used by webrtc-rs. Instead, we feed raw H.264 NAL units.
        let pipeline_str = format!(
            "appsrc name=src format=time is-live=true do-timestamp=true \
             caps=video/x-h264,stream-format=byte-stream,alignment=nal ! \
             h264parse ! \
             {} ! \
             videoconvert ! video/x-raw,format=RGB ! \
             appsink name=sink sync=false max-buffers=1 drop=true",
            decoder_segment
        );

        tracing::info!("Creating H.264 hardware decoder ({}): {}", decoder, pipeline_str);

        let pipeline = gst::parse::launch(&pipeline_str)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("Failed to downcast to Pipeline"))?;

        let appsrc = pipeline
            .by_name("src")
            .ok_or_else(|| anyhow!("Could not find appsrc"))?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| anyhow!("Failed to downcast to AppSrc"))?;

        let appsink = pipeline
            .by_name("sink")
            .ok_or_else(|| anyhow!("Could not find appsink"))?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| anyhow!("Failed to downcast to AppSink"))?;

        appsink.set_property("emit-signals", true);
        appsink.set_property("sync", false);
        appsink.set_property("max-buffers", 1u32);
        appsink.set_property("drop", true);

        pipeline.set_state(gst::State::Playing)?;

        std::thread::sleep(std::time::Duration::from_millis(100));

        if let Some(bus) = pipeline.bus() {
            while let Some(msg) = bus.pop() {
                use gst::MessageView;
                if let MessageView::Error(err) = msg.view() {
                    let _ = pipeline.set_state(gst::State::Null);
                    return Err(anyhow!(
                        "Hardware decoder ({}) failed: {} ({:?})",
                        decoder,
                        err.error(),
                        err.debug()
                    ));
                }
            }
        }

        tracing::info!("H.264 hardware decoder ({}) initialized", decoder);

        Ok(Self {
            pipeline,
            appsrc,
            appsink,
            decoder_name: decoder.to_string(),
            is_running: Arc::new(AtomicBool::new(true)),
            fu_a_buffer: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Depayload RTP H.264 packet to get raw NAL units
    /// Returns NAL units with Annex B start codes (00 00 00 01)
    fn depayload_rtp(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 12 {
            return Err(anyhow!("RTP packet too small"));
        }

        // Parse RTP header
        let extension = (data[0] >> 4) & 0x1;
        let cc = data[0] & 0xf;

        // Calculate payload offset: 12 + (cc * 4) + extension_length
        let mut payload_offset = 12 + (cc as usize * 4);
        if extension == 1 && data.len() > payload_offset + 4 {
            // Extension header: 2 bytes profile + 2 bytes length (in 32-bit words)
            let ext_len = u16::from_be_bytes([data[payload_offset + 2], data[payload_offset + 3]]) as usize;
            payload_offset += 4 + (ext_len * 4);
        }

        if payload_offset >= data.len() {
            return Err(anyhow!("RTP payload offset beyond packet length"));
        }

        let payload = &data[payload_offset..];
        if payload.is_empty() {
            return Err(anyhow!("Empty RTP payload"));
        }

        // Get NAL unit type from first byte of payload
        let nal_header = payload[0];
        let nal_type = nal_header & 0x1f;

        match nal_type {
            // Single NAL unit (types 1-23): pass through with start code
            1..=23 => {
                let mut result = vec![0x00, 0x00, 0x00, 0x01];
                result.extend_from_slice(payload);
                Ok(result)
            }

            // STAP-A (type 24): Aggregated NAL units
            24 => {
                let mut result = Vec::new();
                let mut offset = 1; // Skip STAP-A header

                while offset + 2 <= payload.len() {
                    let nal_size = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
                    offset += 2;

                    if offset + nal_size > payload.len() {
                        break;
                    }

                    // Add start code and NAL unit
                    result.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                    result.extend_from_slice(&payload[offset..offset + nal_size]);
                    offset += nal_size;
                }

                if result.is_empty() {
                    return Err(anyhow!("Empty STAP-A packet"));
                }

                Ok(result)
            }

            // FU-A (type 28): Fragmented NAL unit
            28 => {
                if payload.len() < 2 {
                    return Err(anyhow!("FU-A packet too small"));
                }

                let fu_header = payload[1];
                let start_bit = (fu_header >> 7) & 0x1;
                let end_bit = (fu_header >> 6) & 0x1;
                let original_nal_type = fu_header & 0x1f;

                // Reconstruct NAL header: F and NRI from FU indicator, type from FU header
                let reconstructed_nal_header = (nal_header & 0xe0) | original_nal_type;

                let mut fu_buffer = self.fu_a_buffer.lock().map_err(|e| anyhow!("FU-A buffer lock error: {}", e))?;

                if start_bit == 1 {
                    // Start of fragmented NAL unit
                    fu_buffer.clear();
                    fu_buffer.push(reconstructed_nal_header);
                    fu_buffer.extend_from_slice(&payload[2..]);
                } else {
                    // Continuation of fragmented NAL unit
                    fu_buffer.extend_from_slice(&payload[2..]);
                }

                if end_bit == 1 {
                    // End of fragmented NAL unit - output the complete NAL
                    let mut result = vec![0x00, 0x00, 0x00, 0x01];
                    result.append(&mut *fu_buffer);
                    Ok(result)
                } else {
                    // Not complete yet, return empty
                    Ok(Vec::new())
                }
            }

            // Other types (STAP-B, MTAP16, MTAP24, FU-B) are rarely used
            _ => {
                Err(anyhow!("Unsupported RTP NAL unit type: {}", nal_type))
            }
        }
    }

    /// Decode H.264 RTP data to an RGB frame
    pub fn decode(&self, data: &[u8]) -> Result<Option<VideoFrame>> {
        static DECODE_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let count = DECODE_COUNT.fetch_add(1, Ordering::SeqCst);

        if !self.is_running.load(Ordering::SeqCst) {
            return Err(anyhow!("Decoder not running"));
        }

        if data.is_empty() {
            return Ok(None);
        }

        // Log first few packets with RTP header analysis for debugging
        if count < 10 {
            if data.len() >= 12 {
                let version = (data[0] >> 6) & 0x3;
                let padding = (data[0] >> 5) & 0x1;
                let extension = (data[0] >> 4) & 0x1;
                let cc = data[0] & 0xf;
                let marker = (data[1] >> 7) & 0x1;
                let payload_type = data[1] & 0x7f;
                let seq = u16::from_be_bytes([data[2], data[3]]);

                // Calculate payload offset: 12 + (cc * 4) + extension_length
                let mut payload_offset = 12 + (cc as usize * 4);
                if extension == 1 && data.len() > payload_offset + 4 {
                    // Extension header: 2 bytes profile + 2 bytes length (in 32-bit words)
                    let ext_len = u16::from_be_bytes([data[payload_offset + 2], data[payload_offset + 3]]) as usize;
                    payload_offset += 4 + (ext_len * 4);
                }

                let nal_type = if data.len() > payload_offset {
                    data[payload_offset] & 0x1f
                } else {
                    0
                };

                tracing::info!(
                    "RTP packet #{}: {} bytes, v={} p={} x={} cc={} m={} pt={} seq={}, payload_offset={}, NAL_type={}",
                    count, data.len(), version, padding, extension, cc, marker, payload_type, seq, payload_offset, nal_type
                );

                // Log first few bytes of payload
                if data.len() > payload_offset {
                    let payload_preview = &data[payload_offset..data.len().min(payload_offset + 10)];
                    tracing::info!("  Payload preview: {:02x?}", payload_preview);
                }
            } else {
                tracing::warn!("RTP packet #{} too small: {} bytes", count, data.len());
            }
        }

        // Depayload RTP to get raw H.264 NAL units
        let nal_data = match self.depayload_rtp(data) {
            Ok(d) => d,
            Err(e) => {
                // Log depayload errors for first few packets
                if count < 20 {
                    tracing::warn!("RTP depayload error #{}: {}", count, e);
                }
                return Ok(None);
            }
        };

        // Skip if depayload returned empty (e.g., middle of FU-A fragment)
        if nal_data.is_empty() {
            return Ok(None);
        }

        if count < 10 {
            tracing::info!("Depayloaded NAL data: {} bytes, first 8: {:02x?}",
                nal_data.len(), &nal_data[..nal_data.len().min(8)]);
        }

        let mut buffer = gst::Buffer::with_size(nal_data.len())?;
        {
            let buffer_ref = buffer.get_mut().ok_or_else(|| anyhow!("Failed to get buffer mut"))?;
            let mut map = buffer_ref.map_writable()?;
            map.copy_from_slice(&nal_data);
        }

        if let Err(e) = self.appsrc.push_buffer(buffer) {
            tracing::warn!("Failed to push buffer to decoder: {}", e);
            return Err(anyhow!("Failed to push buffer: {}", e));
        }

        // Check for pipeline errors
        if let Some(bus) = self.pipeline.bus() {
            while let Some(msg) = bus.pop() {
                use gst::MessageView;
                match msg.view() {
                    MessageView::Error(err) => {
                        tracing::error!("Decoder pipeline error: {} ({:?})", err.error(), err.debug());
                    }
                    MessageView::Warning(warn) => {
                        if count < 10 {
                            tracing::warn!("Decoder pipeline warning: {} ({:?})", warn.error(), warn.debug());
                        }
                    }
                    _ => {}
                }
            }
        }

        match self.appsink.try_pull_sample(gst::ClockTime::from_mseconds(10)) {
            Some(sample) => {
                let buffer = sample.buffer().ok_or_else(|| anyhow!("No buffer in sample"))?;
                let caps = sample.caps().ok_or_else(|| anyhow!("No caps in sample"))?;

                let video_info = gst_video::VideoInfo::from_caps(caps)?;
                let width = video_info.width();
                let height = video_info.height();

                let map = buffer.map_readable()?;
                let data = map.as_slice().to_vec();

                tracing::info!("Decoder produced frame: {}x{}", width, height);
                Ok(Some(VideoFrame { width, height, data }))
            }
            None => {
                if count < 10 || count % 100 == 0 {
                    tracing::debug!("Decoder #{}: no output yet (buffering)", count);
                }
                Ok(None)
            }
        }
    }

    pub fn decoder_name(&self) -> &str { &self.decoder_name }

    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        let _ = self.appsrc.end_of_stream();
        let _ = self.pipeline.set_state(gst::State::Null);
        tracing::info!("H.264 decoder stopped");
    }
}

impl Drop for GstH264Decoder {
    fn drop(&mut self) {
        self.stop();
    }
}

// ============================================================================
// Legacy VP8 types (kept for backward compatibility during transition)
// These will be removed once H.264 is fully integrated
// ============================================================================

/// VP8 encoder - DEPRECATED, use GstH264Encoder instead
pub type GstVp8Encoder = GstH264Encoder;

/// VP8 decoder - DEPRECATED, use GstH264Decoder instead
pub type GstVp8Decoder = GstH264Decoder;

/// Screen encoder using H.264 hardware encoding
/// Optimized for screen content with no artificial resolution limits
pub type GstScreenEncoder = GstH264Encoder;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hw_encoder_detection() {
        let result = detect_hw_encoder();
        match result {
            Ok(encoder) => println!("Found hardware encoder: {}", encoder),
            Err(e) => println!("No hardware encoder: {}", e),
        }
    }

    #[test]
    fn test_hw_decoder_detection() {
        let result = detect_hw_decoder();
        match result {
            Ok(decoder) => println!("Found hardware decoder: {}", decoder),
            Err(e) => println!("No hardware decoder: {}", e),
        }
    }
}
