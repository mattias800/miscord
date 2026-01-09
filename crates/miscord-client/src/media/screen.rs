//! GStreamer-based screen capture
//!
//! Uses platform-specific GStreamer sources for screen capture.
//! - macOS: avfvideosrc with capture-screen=true
//! - Linux: ximagesrc or pipewiresrc
//! - Windows: d3d11screencapturesrc

use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::gst_video::VideoFrame;

/// Information about a monitor
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
}

/// Information about a window (placeholder - GStreamer doesn't easily enumerate windows)
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub app_name: String,
    pub width: u32,
    pub height: u32,
}

/// Type of capture source
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureType {
    None,
    Monitor,
    Window,
}

/// Screen frame data
pub struct ScreenFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA data
}

/// GStreamer-based screen capture
pub struct ScreenCapture {
    pipeline: Option<gst::Pipeline>,
    appsink: Option<gst_app::AppSink>,
    capture_type: CaptureType,
    is_running: Arc<AtomicBool>,
    /// Max output width (for scaling down)
    max_width: Option<u32>,
    /// Max output height (for scaling down)
    max_height: Option<u32>,
}

impl ScreenCapture {
    /// Create a new screen capture instance
    pub fn new() -> Result<Self> {
        gst::init()?;
        Ok(Self {
            pipeline: None,
            appsink: None,
            capture_type: CaptureType::None,
            is_running: Arc::new(AtomicBool::new(false)),
            max_width: None,
            max_height: None,
        })
    }

    /// Create a new screen capture instance with scaling
    /// The output will be scaled down to fit within max_width x max_height
    /// while maintaining aspect ratio
    pub fn new_with_scaling(max_width: u32, max_height: u32) -> Result<Self> {
        gst::init()?;
        Ok(Self {
            pipeline: None,
            appsink: None,
            capture_type: CaptureType::None,
            is_running: Arc::new(AtomicBool::new(false)),
            max_width: Some(max_width),
            max_height: Some(max_height),
        })
    }

    /// List available monitors
    pub fn list_monitors() -> Result<Vec<MonitorInfo>> {
        #[cfg(target_os = "macos")]
        {
            Self::list_monitors_macos()
        }

        #[cfg(target_os = "linux")]
        {
            // On Linux, return primary display - proper enumeration would need X11/Wayland APIs
            Ok(vec![MonitorInfo {
                id: 0,
                name: "Primary Display".to_string(),
                width: 1920,
                height: 1080,
                is_primary: true,
            }])
        }

        #[cfg(target_os = "windows")]
        {
            // On Windows, return primary display - proper enumeration would need Win32 APIs
            Ok(vec![MonitorInfo {
                id: 0,
                name: "Primary Display".to_string(),
                width: 1920,
                height: 1080,
                is_primary: true,
            }])
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Ok(vec![MonitorInfo {
                id: 0,
                name: "Primary Display".to_string(),
                width: 1920,
                height: 1080,
                is_primary: true,
            }])
        }
    }

    #[cfg(target_os = "macos")]
    fn list_monitors_macos() -> Result<Vec<MonitorInfo>> {
        use std::process::Command;

        // Use system_profiler to get display info on macOS
        let output = Command::new("system_profiler")
            .args(["SPDisplaysDataType", "-json"])
            .output();

        match output {
            Ok(output) => {
                if let Ok(json_str) = String::from_utf8(output.stdout) {
                    // Parse the JSON to extract display info
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        let mut monitors = Vec::new();
                        let mut id = 0u32;

                        if let Some(displays) = json.get("SPDisplaysDataType")
                            .and_then(|v| v.as_array())
                        {
                            for gpu in displays {
                                if let Some(ndrvs) = gpu.get("spdisplays_ndrvs")
                                    .and_then(|v| v.as_array())
                                {
                                    for display in ndrvs {
                                        let name = display.get("_name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Unknown Display")
                                            .to_string();

                                        // Parse resolution from _spdisplays_resolution
                                        let (width, height) = display.get("_spdisplays_resolution")
                                            .and_then(|v| v.as_str())
                                            .and_then(|res| {
                                                // Format: "3840 x 2160" or similar
                                                let parts: Vec<&str> = res.split(" x ").collect();
                                                if parts.len() >= 2 {
                                                    let w = parts[0].trim().parse::<u32>().ok()?;
                                                    let h = parts[1].split_whitespace().next()?.parse::<u32>().ok()?;
                                                    Some((w, h))
                                                } else {
                                                    None
                                                }
                                            })
                                            .unwrap_or((1920, 1080));

                                        let is_primary = display.get("spdisplays_main")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s == "spdisplays_yes")
                                            .unwrap_or(id == 0);

                                        monitors.push(MonitorInfo {
                                            id,
                                            name,
                                            width,
                                            height,
                                            is_primary,
                                        });
                                        id += 1;
                                    }
                                }
                            }
                        }

                        if !monitors.is_empty() {
                            return Ok(monitors);
                        }
                    }
                }
                // Fallback if parsing fails
                Ok(vec![MonitorInfo {
                    id: 0,
                    name: "Primary Display".to_string(),
                    width: 1920,
                    height: 1080,
                    is_primary: true,
                }])
            }
            Err(_) => {
                // Fallback if command fails
                Ok(vec![MonitorInfo {
                    id: 0,
                    name: "Primary Display".to_string(),
                    width: 1920,
                    height: 1080,
                    is_primary: true,
                }])
            }
        }
    }

    /// List available windows
    /// Note: Window capture is platform-specific and not fully supported via GStreamer
    pub fn list_windows() -> Result<Vec<WindowInfo>> {
        // GStreamer doesn't easily enumerate windows
        // Return empty list - window capture requires platform-specific APIs
        Ok(vec![])
    }

    /// Start capturing from a monitor
    pub fn start_monitor(&mut self, monitor_id: u32, fps: u32) -> Result<()> {
        if self.is_running.load(Ordering::SeqCst) {
            self.stop();
        }

        let pipeline_str = Self::build_monitor_pipeline(monitor_id, fps, self.max_width, self.max_height)?;

        tracing::info!("Starting screen capture with pipeline: {}", pipeline_str);

        let pipeline = gst::parse::launch(&pipeline_str)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("Failed to downcast to Pipeline"))?;

        let appsink = pipeline
            .by_name("sink")
            .ok_or_else(|| anyhow!("Could not find appsink"))?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| anyhow!("Failed to downcast to AppSink"))?;

        // Configure appsink for ULTRA LOW LATENCY
        appsink.set_property("emit-signals", true);
        appsink.set_property("sync", false);
        appsink.set_property("max-buffers", 1u32);  // Only keep latest frame
        appsink.set_property("drop", true);

        // Start the pipeline
        pipeline.set_state(gst::State::Playing)?;

        // Wait for pipeline state change (async, no fixed sleep for low latency)
        let (state_result, _current, _pending) =
            pipeline.state(gst::ClockTime::from_mseconds(500));
        if state_result == Err(gst::StateChangeError) {
            if let Some(bus) = pipeline.bus() {
                while let Some(msg) = bus.pop() {
                    use gst::MessageView;
                    if let MessageView::Error(err) = msg.view() {
                        let _ = pipeline.set_state(gst::State::Null);
                        return Err(anyhow!(
                            "GStreamer screen capture error: {} ({:?})",
                            err.error(),
                            err.debug()
                        ));
                    }
                }
            }
            let _ = pipeline.set_state(gst::State::Null);
            return Err(anyhow!("Screen capture pipeline failed to reach Playing state"));
        }

        self.pipeline = Some(pipeline);
        self.appsink = Some(appsink);
        self.capture_type = CaptureType::Monitor;
        self.is_running.store(true, Ordering::SeqCst);

        tracing::info!("Screen capture started for monitor {}", monitor_id);
        Ok(())
    }

    /// Start capturing from a window (not fully supported)
    pub fn start_window(&mut self, _window_id: u64, _fps: u32) -> Result<()> {
        Err(anyhow!(
            "Window capture is not yet supported via GStreamer. Use monitor capture instead."
        ))
    }

    /// Build platform-specific pipeline string for monitor capture
    fn build_monitor_pipeline(monitor_id: u32, fps: u32, max_width: Option<u32>, max_height: Option<u32>) -> Result<String> {
        // Build scaling element if dimensions are specified
        let scale_element = if let (Some(w), Some(h)) = (max_width, max_height) {
            // videoscale with caps to limit output size
            format!(
                "videoscale ! video/x-raw,width={},height={} ! ",
                w, h
            )
        } else {
            String::new()
        };

        #[cfg(target_os = "macos")]
        {
            // macOS: Use avfvideosrc with capture-screen=true
            // device-index selects which display to capture
            // Low-latency optimizations:
            // - No videorate (source framerate is stable enough)
            // - Capture in NV12 (native), convert to RGBA for UI preview
            // - Encoder will convert RGBA→NV12 but this is fast on modern hardware
            // - appsink with sync=false, max-buffers=1, drop=true
            Ok(format!(
                "avfvideosrc capture-screen=true capture-screen-cursor=true device-index={} ! \
                 video/x-raw,format=NV12,framerate={}/1 ! \
                 {}videoconvert ! video/x-raw,format=RGBA ! \
                 appsink name=sink sync=false max-buffers=1 drop=true",
                monitor_id, fps, scale_element
            ))
        }

        #[cfg(target_os = "linux")]
        {
            // Linux: Use ximagesrc for X11
            // For Wayland, pipewire would be needed
            Ok(format!(
                "ximagesrc display-name=:0 show-pointer=true use-damage=false ! \
                 videorate ! video/x-raw,framerate={}/1 ! \
                 {}videoconvert ! video/x-raw,format=RGB ! \
                 appsink name=sink",
                fps, scale_element
            ))
        }

        #[cfg(target_os = "windows")]
        {
            // Windows: Use d3d11screencapturesrc
            Ok(format!(
                "d3d11screencapturesrc monitor-index={} show-cursor=true ! \
                 videorate ! video/x-raw,framerate={}/1 ! \
                 {}videoconvert ! video/x-raw,format=RGB ! \
                 appsink name=sink",
                monitor_id, fps, scale_element
            ))
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Err(anyhow!("Screen capture not supported on this platform"))
        }
    }

    /// Get the next captured frame
    /// Ultra low latency: minimal timeout to avoid blocking
    pub fn get_frame(&self) -> Option<ScreenFrame> {
        if !self.is_running.load(Ordering::SeqCst) {
            return None;
        }

        let appsink = self.appsink.as_ref()?;

        // Ultra low latency: very short timeout (5ms) - if no frame ready, return None
        match appsink.try_pull_sample(gst::ClockTime::from_mseconds(5)) {
            Some(sample) => {
                let buffer = sample.buffer()?;
                let caps = sample.caps()?;

                // Get video info from caps
                let video_info = gstreamer_video::VideoInfo::from_caps(caps).ok()?;
                let width = video_info.width();
                let height = video_info.height();

                // Map buffer to read data
                let map = buffer.map_readable().ok()?;
                let data = map.as_slice().to_vec();

                Some(ScreenFrame {
                    width,
                    height,
                    data,
                })
            }
            None => None,
        }
    }

    /// Convert ScreenFrame to VideoFrame for encoding
    pub fn frame_to_video_frame(frame: &ScreenFrame) -> VideoFrame {
        VideoFrame {
            width: frame.width,
            height: frame.height,
            data: frame.data.clone(),
        }
    }

    /// Stop capturing
    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);

        if let Some(pipeline) = self.pipeline.take() {
            let _ = pipeline.set_state(gst::State::Null);
        }

        self.appsink = None;
        self.capture_type = CaptureType::None;

        tracing::info!("Screen capture stopped");
    }

    /// Check if currently capturing
    pub fn is_capturing(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Get the current capture type
    pub fn capture_type(&self) -> CaptureType {
        self.capture_type
    }
}

impl Default for ScreenCapture {
    fn default() -> Self {
        Self::new().expect("Failed to initialize screen capture")
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_monitors() {
        let monitors = ScreenCapture::list_monitors();
        assert!(monitors.is_ok());
        // Should have at least one monitor entry
        assert!(!monitors.unwrap().is_empty());
    }

    #[test]
    fn test_screen_capture_creation() {
        let capture = ScreenCapture::new();
        assert!(capture.is_ok());
    }
}
