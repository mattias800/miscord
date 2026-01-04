use anyhow::Result;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGB data
}

pub struct VideoCapture {
    camera: Option<Camera>,
    is_capturing: Arc<RwLock<bool>>,
}

impl VideoCapture {
    pub fn new() -> Self {
        Self {
            camera: None,
            is_capturing: Arc::new(RwLock::new(false)),
        }
    }

    pub fn list_devices() -> Result<Vec<String>> {
        let devices = nokhwa::query(nokhwa::utils::ApiBackend::Auto)?;
        Ok(devices.iter().map(|d| d.human_name().to_string()).collect())
    }

    pub fn start(&mut self, device_index: Option<u32>) -> Result<mpsc::Receiver<VideoFrame>> {
        let index = CameraIndex::Index(device_index.unwrap_or(0));

        let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);

        let mut camera = Camera::new(index, requested)?;
        camera.open_stream()?;

        let resolution = camera.resolution();
        tracing::info!(
            "Starting video capture: {}x{}",
            resolution.width(),
            resolution.height()
        );

        let (tx, rx) = mpsc::channel(10);
        let is_capturing = self.is_capturing.clone();

        // Store camera
        self.camera = Some(camera);

        // Mark as capturing
        {
            let mut capturing = futures::executor::block_on(is_capturing.write());
            *capturing = true;
        }

        // Note: In a real implementation, you'd spawn a thread to continuously
        // capture frames. For now, this is a simplified version.

        Ok(rx)
    }

    pub async fn capture_frame(&mut self) -> Result<Option<VideoFrame>> {
        if let Some(camera) = &mut self.camera {
            let frame = camera.frame()?;
            let resolution = camera.resolution();

            Ok(Some(VideoFrame {
                width: resolution.width(),
                height: resolution.height(),
                data: frame.buffer().to_vec(),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut camera) = self.camera.take() {
            let _ = camera.stop_stream();
        }

        let is_capturing = self.is_capturing.clone();
        futures::executor::block_on(async {
            let mut capturing = is_capturing.write().await;
            *capturing = false;
        });
    }

    pub async fn is_capturing(&self) -> bool {
        *self.is_capturing.read().await
    }
}

impl Default for VideoCapture {
    fn default() -> Self {
        Self::new()
    }
}
