use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct VideoDeviceInfo {
    pub index: u32,
    pub name: String,
    pub device_path: String,
}

#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA data
}

pub struct GstVideoCapture {
    pipeline: Option<gst::Pipeline>,
    appsink: Option<gst_app::AppSink>,
    is_running: Arc<AtomicBool>,
}

impl GstVideoCapture {
    pub fn new() -> Result<Self> {
        // Initialize GStreamer
        gst::init()?;

        Ok(Self {
            pipeline: None,
            appsink: None,
            is_running: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn list_devices() -> Result<Vec<VideoDeviceInfo>> {
        gst::init()?;

        let mut devices = Vec::new();

        // Use device monitor to find video sources
        let monitor = gst::DeviceMonitor::new();
        monitor.add_filter(Some("Video/Source"), None);

        if let Err(e) = monitor.start() {
            tracing::warn!("Failed to start device monitor: {}", e);
            // Return default device
            devices.push(VideoDeviceInfo {
                index: 0,
                name: "Default Camera".to_string(),
                device_path: String::new(),
            });
            return Ok(devices);
        }

        for (index, device) in monitor.devices().iter().enumerate() {
            let name = device.display_name().to_string();

            // On macOS, devices don't have device-path, just use the index
            devices.push(VideoDeviceInfo {
                index: index as u32,
                name,
                device_path: format!("{}", index),
            });
        }

        monitor.stop();

        // If no devices found via monitor, add default
        if devices.is_empty() {
            devices.push(VideoDeviceInfo {
                index: 0,
                name: "Default Camera".to_string(),
                device_path: String::new(),
            });
        }

        Ok(devices)
    }

    pub fn start(&mut self, device_index: Option<u32>) -> Result<()> {
        if self.is_running.load(Ordering::SeqCst) {
            self.stop();
        }

        let _device_idx = device_index.unwrap_or(0);

        // Build GStreamer pipeline for macOS (avfvideosrc) or Linux (v4l2src)
        // The caps filter must come AFTER videoconvert to force RGBA output
        #[cfg(target_os = "macos")]
        let pipeline_str = format!(
            "avfvideosrc device-index={} ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             videoscale ! \
             appsink name=sink",
            _device_idx
        );

        #[cfg(target_os = "linux")]
        let pipeline_str = format!(
            "v4l2src device=/dev/video{} ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             videoscale ! \
             appsink name=sink",
            _device_idx
        );

        #[cfg(target_os = "windows")]
        let pipeline_str = format!(
            "ksvideosrc device-index={} ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             videoscale ! \
             appsink name=sink",
            _device_idx
        );

        tracing::info!("Starting GStreamer pipeline: {}", pipeline_str);

        let pipeline = gst::parse::launch(&pipeline_str)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("Failed to downcast to Pipeline"))?;

        // Get appsink
        let appsink = pipeline
            .by_name("sink")
            .ok_or_else(|| anyhow!("Could not find appsink"))?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| anyhow!("Failed to downcast to AppSink"))?;

        // Configure appsink properties
        appsink.set_property("emit-signals", true);
        appsink.set_property("sync", false);
        appsink.set_property("max-buffers", 1u32);
        appsink.set_property("drop", true);

        // Start the pipeline
        self.is_running.store(true, Ordering::SeqCst);
        pipeline.set_state(gst::State::Playing)?;

        // Give it a moment to start
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Check for errors
        if let Some(bus) = pipeline.bus() {
            while let Some(msg) = bus.pop() {
                use gst::MessageView;
                if let MessageView::Error(err) = msg.view() {
                    let _ = pipeline.set_state(gst::State::Null);
                    return Err(anyhow!(
                        "GStreamer error: {} ({:?})",
                        err.error(),
                        err.debug()
                    ));
                }
            }
        }

        // Verify pipeline state
        let (state_result, _current, _pending) =
            pipeline.state(gst::ClockTime::from_seconds(2));
        if state_result == Err(gst::StateChangeError) {
            return Err(anyhow!("Pipeline failed to reach Playing state"));
        }

        tracing::info!("GStreamer video capture started successfully");

        self.pipeline = Some(pipeline);
        self.appsink = Some(appsink);
        Ok(())
    }

    pub fn get_frame(&self) -> Option<VideoFrame> {
        let appsink = self.appsink.as_ref()?;

        // Try to pull a sample with a small timeout
        let sample = match appsink.try_pull_sample(gst::ClockTime::from_mseconds(16)) {
            Some(s) => s,
            None => {
                // Check if EOS
                if appsink.is_eos() {
                    tracing::warn!("GStreamer appsink reached EOS");
                }
                return None;
            }
        };

        let buffer = sample.buffer()?;
        let caps = sample.caps()?;

        // Get video info from caps
        let video_info = gst_video::VideoInfo::from_caps(caps).ok()?;
        let width = video_info.width();
        let height = video_info.height();

        // Map buffer to read data
        let map = buffer.map_readable().ok()?;
        let data = map.as_slice().to_vec();

        tracing::info!("GStreamer: got frame {}x{}, {} bytes", width, height, data.len());

        Some(VideoFrame {
            width,
            height,
            data,
        })
    }

    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        self.appsink = None;

        if let Some(pipeline) = self.pipeline.take() {
            let _ = pipeline.set_state(gst::State::Null);
            tracing::info!("GStreamer video capture stopped");
        }
    }

    pub fn is_capturing(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }
}

impl Default for GstVideoCapture {
    fn default() -> Self {
        Self::new().expect("Failed to initialize GStreamer")
    }
}

impl Drop for GstVideoCapture {
    fn drop(&mut self) {
        self.stop();
    }
}
