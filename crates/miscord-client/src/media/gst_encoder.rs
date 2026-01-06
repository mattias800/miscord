//! GStreamer-based VP8 encoder and decoder for WebRTC video streaming
//!
//! Uses GStreamer pipelines to encode RGB frames to VP8 and decode VP8 to RGB.

use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::gst_video::VideoFrame;

/// VP8 encoder using GStreamer
/// Pipeline: appsrc → videoconvert → vp8enc → appsink
pub struct GstVp8Encoder {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
    width: u32,
    height: u32,
    is_running: Arc<AtomicBool>,
}

impl GstVp8Encoder {
    /// Create a new VP8 encoder for the given frame dimensions
    pub fn new(width: u32, height: u32) -> Result<Self> {
        gst::init()?;

        // Build encoding pipeline
        // Scale down large resolutions for faster encoding
        // VP8 encoding at 1080p is very CPU intensive
        let (enc_width, enc_height) = if width > 1280 || height > 720 {
            // Scale down to 720p max, maintaining aspect ratio
            let scale = (1280.0 / width as f64).min(720.0 / height as f64);
            ((width as f64 * scale) as u32, (height as f64 * scale) as u32)
        } else {
            (width, height)
        };

        // vp8enc settings for low latency:
        // - deadline=1: realtime encoding
        // - cpu-used=16: maximum speed (range 0-16)
        // - keyframe-max-dist=10: more frequent keyframes for faster startup and recovery
        // - target-bitrate: 500 kbps for reasonable quality at 720p
        // - threads=4: use multiple threads for encoding
        // - lag-in-frames=0: no lookahead for lower latency
        // - error-resilient=1: better recovery from packet loss
        let pipeline_str = format!(
            "appsrc name=src format=time is-live=true do-timestamp=true \
             caps=video/x-raw,format=RGB,width={},height={},framerate=30/1 ! \
             videoconvert ! \
             videoscale ! video/x-raw,width={},height={} ! \
             vp8enc deadline=1 cpu-used=16 keyframe-max-dist=10 target-bitrate=500000 threads=4 lag-in-frames=0 error-resilient=1 ! \
             appsink name=sink sync=false max-buffers=2 drop=true",
            width, height, enc_width, enc_height
        );

        tracing::info!("Creating VP8 encoder pipeline: {}", pipeline_str);

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

        // Configure appsink
        appsink.set_property("emit-signals", true);
        appsink.set_property("sync", false);

        // Start the pipeline
        pipeline.set_state(gst::State::Playing)?;

        // Brief wait for pipeline to initialize
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Check for errors
        if let Some(bus) = pipeline.bus() {
            while let Some(msg) = bus.pop() {
                use gst::MessageView;
                if let MessageView::Error(err) = msg.view() {
                    let _ = pipeline.set_state(gst::State::Null);
                    return Err(anyhow!(
                        "GStreamer encoder error: {} ({:?})",
                        err.error(),
                        err.debug()
                    ));
                }
            }
        }

        tracing::info!("VP8 encoder initialized for {}x{}", width, height);

        Ok(Self {
            pipeline,
            appsrc,
            appsink,
            width,
            height,
            is_running: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Encode an RGB frame to VP8
    pub fn encode(&self, frame: &VideoFrame) -> Result<Vec<u8>> {
        if !self.is_running.load(Ordering::SeqCst) {
            return Err(anyhow!("Encoder not running"));
        }

        // Verify frame dimensions match
        if frame.width != self.width || frame.height != self.height {
            return Err(anyhow!(
                "Frame size mismatch: expected {}x{}, got {}x{}",
                self.width,
                self.height,
                frame.width,
                frame.height
            ));
        }

        let expected_size = (self.width * self.height * 3) as usize;
        if frame.data.len() != expected_size {
            return Err(anyhow!(
                "Frame data size mismatch: expected {}, got {}",
                expected_size,
                frame.data.len()
            ));
        }

        // Create buffer from frame data
        let mut buffer = gst::Buffer::with_size(frame.data.len())?;
        {
            let buffer_ref = buffer.get_mut().ok_or_else(|| anyhow!("Failed to get buffer mut"))?;
            let mut map = buffer_ref.map_writable()?;
            map.copy_from_slice(&frame.data);
        }

        // Push to appsrc
        self.appsrc.push_buffer(buffer)?;

        // Try to pull encoded data with longer timeout for high-res encoding
        match self.appsink.try_pull_sample(gst::ClockTime::from_mseconds(50)) {
            Some(sample) => {
                let buffer = sample.buffer().ok_or_else(|| anyhow!("No buffer in sample"))?;
                let map = buffer.map_readable()?;
                let data = map.as_slice().to_vec();
                tracing::debug!("Encoded frame: {} bytes", data.len());
                Ok(data)
            }
            None => {
                // No output yet (encoder buffering)
                tracing::debug!("Encoder buffering, no output yet");
                Ok(vec![])
            }
        }
    }

    /// Get the configured width
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get the configured height
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Stop the encoder
    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        let _ = self.appsrc.end_of_stream();
        let _ = self.pipeline.set_state(gst::State::Null);
        tracing::info!("VP8 encoder stopped");
    }
}

impl Drop for GstVp8Encoder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// VP8 decoder using GStreamer
/// Pipeline: appsrc → rtpvp8depay → vp8dec → videoconvert → appsink
/// Note: Input is RTP VP8 payload (with VP8 payload descriptor)
pub struct GstVp8Decoder {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
    is_running: Arc<AtomicBool>,
}

impl GstVp8Decoder {
    /// Create a new VP8 decoder
    pub fn new() -> Result<Self> {
        gst::init()?;

        // Build decoding pipeline with rtpvp8depay to depacketize RTP VP8 payload
        // Low-latency settings:
        // - queue with leaky=downstream drops old frames when backed up
        // - max-size-buffers=1 keeps minimal buffering
        // - appsink with max-buffers=1 drop=true keeps only latest frame
        let pipeline_str =
            "appsrc name=src format=time is-live=true do-timestamp=true \
             caps=application/x-rtp,media=video,encoding-name=VP8,clock-rate=90000,payload=96 ! \
             rtpvp8depay ! \
             queue max-size-buffers=1 max-size-bytes=0 max-size-time=0 leaky=downstream ! \
             vp8dec ! \
             videoconvert ! \
             video/x-raw,format=RGB ! \
             appsink name=sink sync=false max-buffers=1 drop=true";

        tracing::info!("Creating VP8 decoder pipeline: {}", pipeline_str);

        let pipeline = gst::parse::launch(pipeline_str)?
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

        // Configure appsink for low latency
        appsink.set_property("emit-signals", true);
        appsink.set_property("sync", false);
        appsink.set_property("max-buffers", 1u32);
        appsink.set_property("drop", true);

        // Start the pipeline
        pipeline.set_state(gst::State::Playing)?;

        // Wait for pipeline to start (reduced from 100ms)
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Check for errors
        if let Some(bus) = pipeline.bus() {
            while let Some(msg) = bus.pop() {
                use gst::MessageView;
                if let MessageView::Error(err) = msg.view() {
                    let _ = pipeline.set_state(gst::State::Null);
                    return Err(anyhow!(
                        "GStreamer decoder error: {} ({:?})",
                        err.error(),
                        err.debug()
                    ));
                }
            }
        }

        tracing::info!("VP8 decoder initialized");

        Ok(Self {
            pipeline,
            appsrc,
            appsink,
            is_running: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Decode VP8 data to an RGB frame
    /// Returns the most recent decoded frame (older frames are dropped for low latency)
    pub fn decode(&self, data: &[u8]) -> Result<Option<VideoFrame>> {
        if !self.is_running.load(Ordering::SeqCst) {
            return Err(anyhow!("Decoder not running"));
        }

        if data.is_empty() {
            return Ok(None);
        }

        // Create buffer from VP8 data
        let mut buffer = gst::Buffer::with_size(data.len())?;
        {
            let buffer_ref = buffer.get_mut().ok_or_else(|| anyhow!("Failed to get buffer mut"))?;
            let mut map = buffer_ref.map_writable()?;
            map.copy_from_slice(data);
        }

        // Push to appsrc
        self.appsrc.push_buffer(buffer)?;

        // Try to pull decoded frame with short timeout for low latency
        // The appsink is configured with drop=true so it keeps only the latest frame
        match self.appsink.try_pull_sample(gst::ClockTime::from_mseconds(10)) {
            Some(sample) => {
                let buffer = sample.buffer().ok_or_else(|| anyhow!("No buffer in sample"))?;
                let caps = sample.caps().ok_or_else(|| anyhow!("No caps in sample"))?;

                // Get video info from caps
                let video_info = gst_video::VideoInfo::from_caps(caps)?;
                let width = video_info.width();
                let height = video_info.height();

                // Map buffer to read data
                let map = buffer.map_readable()?;
                let data = map.as_slice().to_vec();

                Ok(Some(VideoFrame {
                    width,
                    height,
                    data,
                }))
            }
            None => {
                // No output yet (decoder buffering or still processing)
                Ok(None)
            }
        }
    }

    /// Stop the decoder
    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        let _ = self.appsrc.end_of_stream();
        let _ = self.pipeline.set_state(gst::State::Null);
        tracing::info!("VP8 decoder stopped");
    }
}

impl Drop for GstVp8Decoder {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        // This test requires GStreamer to be installed with VP8 plugins
        let result = GstVp8Encoder::new(640, 480);
        // May fail if GStreamer VP8 plugins not installed
        if let Err(e) = &result {
            eprintln!("Encoder creation failed (may be expected): {}", e);
        }
    }

    #[test]
    fn test_decoder_creation() {
        let result = GstVp8Decoder::new();
        if let Err(e) = &result {
            eprintln!("Decoder creation failed (may be expected): {}", e);
        }
    }
}
