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
            format!(
                "{} allow-frame-reordering=false max-keyframe-interval=30 \
                 max-keyframe-interval-duration=1000000000 bitrate={} ! \
                 h264parse config-interval=-1",
                encoder,
                bitrate / 1000  // vtenc uses kbps
            )
        }
        "nvh264enc" => {
            // NVIDIA NVENC
            // preset=low-latency-hq for good quality with low latency
            // rc-mode=cbr for constant bitrate
            format!(
                "nvh264enc preset=low-latency-hq rc-mode=cbr bitrate={} zerolatency=true ! \
                 h264parse config-interval=-1",
                bitrate / 1000
            )
        }
        "vaapih264enc" => {
            // VAAPI encoder (Intel/AMD on Linux)
            format!(
                "vaapih264enc rate-control=cbr bitrate={} keyframe-period=60 ! \
                 h264parse config-interval=-1",
                bitrate / 1000
            )
        }
        "amfh264enc" => {
            // AMD AMF encoder (Windows)
            format!(
                "amfh264enc bitrate={} rate-control=cbr ! \
                 h264parse config-interval=-1",
                bitrate / 1000
            )
        }
        "qsvh264enc" => {
            // Intel QuickSync (Windows)
            format!(
                "qsvh264enc bitrate={} rate-control=cbr low-latency=true ! \
                 h264parse config-interval=-1",
                bitrate / 1000
            )
        }
        "mfh264enc" => {
            // MediaFoundation (Windows fallback)
            format!(
                "mfh264enc bitrate={} ! \
                 h264parse config-interval=-1",
                bitrate
            )
        }
        _ => {
            // Generic fallback format
            format!(
                "{} bitrate={} ! h264parse config-interval=-1",
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

        tracing::info!(
            "H.264 hardware encoder ({}) initialized for {}x{} @{}fps",
            encoder, width, height, fps
        );

        Ok(Self {
            pipeline,
            appsrc,
            appsink,
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
                tracing::debug!("H.264 encoded frame: {} bytes", data.len());
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
}

impl GstH264Decoder {
    /// Create a new H.264 hardware decoder
    pub fn new() -> Result<Self> {
        let decoder = detect_hw_decoder()?;

        let decoder_segment = build_decoder_pipeline(decoder);

        // Pipeline for RTP H.264: appsrc → rtph264depay → h264parse → hw_decoder → videoconvert → appsink
        let pipeline_str = format!(
            "appsrc name=src format=time is-live=true do-timestamp=true \
             caps=application/x-rtp,media=video,encoding-name=H264,clock-rate=90000,payload=96 ! \
             rtph264depay ! \
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
        })
    }

    /// Decode H.264 RTP data to an RGB frame
    pub fn decode(&self, data: &[u8]) -> Result<Option<VideoFrame>> {
        if !self.is_running.load(Ordering::SeqCst) {
            return Err(anyhow!("Decoder not running"));
        }

        if data.is_empty() {
            return Ok(None);
        }

        let mut buffer = gst::Buffer::with_size(data.len())?;
        {
            let buffer_ref = buffer.get_mut().ok_or_else(|| anyhow!("Failed to get buffer mut"))?;
            let mut map = buffer_ref.map_writable()?;
            map.copy_from_slice(data);
        }

        self.appsrc.push_buffer(buffer)?;

        match self.appsink.try_pull_sample(gst::ClockTime::from_mseconds(10)) {
            Some(sample) => {
                let buffer = sample.buffer().ok_or_else(|| anyhow!("No buffer in sample"))?;
                let caps = sample.caps().ok_or_else(|| anyhow!("No caps in sample"))?;

                let video_info = gst_video::VideoInfo::from_caps(caps)?;
                let width = video_info.width();
                let height = video_info.height();

                let map = buffer.map_readable()?;
                let data = map.as_slice().to_vec();

                Ok(Some(VideoFrame { width, height, data }))
            }
            None => Ok(None),
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
