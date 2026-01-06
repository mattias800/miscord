use anyhow::Result;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use nokhwa::pixel_format::RgbFormat;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use turbojpeg::PixelFormat;

#[derive(Debug, Clone)]
pub struct VideoDeviceInfo {
    pub index: u32,
    pub name: String,
}

pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGB data
}

pub struct VideoCapture {
    stop_flag: Arc<AtomicBool>,
    capture_thread: Option<thread::JoinHandle<()>>,
    frame_rx: Option<std::sync::mpsc::Receiver<VideoFrame>>,
}

impl VideoCapture {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            capture_thread: None,
            frame_rx: None,
        }
    }

    pub fn list_devices() -> Result<Vec<VideoDeviceInfo>> {
        let devices = nokhwa::query(nokhwa::utils::ApiBackend::Auto)?;
        Ok(devices
            .iter()
            .map(|d| VideoDeviceInfo {
                index: d.index().as_index().unwrap_or(0),
                name: d.human_name().to_string(),
            })
            .collect())
    }

    pub fn start(&mut self, device_index: Option<u32>) -> Result<()> {
        // Reset stop flag
        self.stop_flag.store(false, Ordering::SeqCst);

        // Create channel for frames (use std::sync::mpsc for thread compatibility)
        let (tx, rx) = std::sync::mpsc::sync_channel(2); // Small buffer to avoid lag
        self.frame_rx = Some(rx);

        let stop_flag = self.stop_flag.clone();
        let frame_duration = Duration::from_millis(33); // ~30 FPS
        let device_idx = device_index.unwrap_or(0);

        // Spawn capture thread - camera must be created in the thread
        let handle = thread::spawn(move || {
            let index = CameraIndex::Index(device_idx);
            // Use highest frame rate - camera will pick best supported resolution
            let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);

            let mut camera = match Camera::new(index, requested) {
                Ok(cam) => cam,
                Err(e) => {
                    tracing::error!("Failed to create camera: {}", e);
                    return;
                }
            };

            // Log supported formats for debugging
            if let Ok(formats) = camera.compatible_list_by_resolution(nokhwa::utils::FrameFormat::MJPEG) {
                tracing::info!("Camera supported MJPEG resolutions:");
                for (res, _fps_list) in formats.iter().take(5) {
                    tracing::info!("  {}x{}", res.width(), res.height());
                }
            }
            if let Ok(formats) = camera.compatible_list_by_resolution(nokhwa::utils::FrameFormat::YUYV) {
                tracing::info!("Camera supported YUYV resolutions:");
                for (res, _fps_list) in formats.iter().take(5) {
                    tracing::info!("  {}x{}", res.width(), res.height());
                }
            }
            if let Ok(formats) = camera.compatible_list_by_resolution(nokhwa::utils::FrameFormat::NV12) {
                tracing::info!("Camera supported NV12 resolutions:");
                for (res, _fps_list) in formats.iter().take(5) {
                    tracing::info!("  {}x{}", res.width(), res.height());
                }
            }

            if let Err(e) = camera.open_stream() {
                tracing::error!("Failed to open camera stream: {}", e);
                return;
            }

            let resolution = camera.resolution();
            tracing::info!(
                "Starting video capture: {}x{} at 30 FPS",
                resolution.width(),
                resolution.height()
            );

            let mut frame_count = 0u32;
            let fps_start = Instant::now();

            while !stop_flag.load(Ordering::SeqCst) {
                let frame_start = Instant::now();

                // Capture frame
                match camera.frame() {
                    Ok(frame) => {
                        let capture_time = frame_start.elapsed();

                        // Get raw JPEG data and decode with turbojpeg (much faster than nokhwa's decoder)
                        let decode_start = Instant::now();
                        let jpeg_data = frame.buffer();

                        // Try turbojpeg decoding
                        match turbojpeg::decompress(jpeg_data, PixelFormat::RGB) {
                            Ok(image) => {
                                let decode_time = decode_start.elapsed();
                                let (orig_width, orig_height) = (image.width, image.height);
                                let data = &image.pixels;

                                // Log timing every 30 frames
                                frame_count += 1;
                                if frame_count % 30 == 0 {
                                    let elapsed = fps_start.elapsed().as_secs_f32();
                                    let fps = frame_count as f32 / elapsed;
                                    tracing::info!(
                                        "Video stats: {:.1} FPS, capture={:?}, decode={:?}",
                                        fps, capture_time, decode_time
                                    );
                                }

                                // Downscale by 2x for better performance (sample every 2nd pixel)
                                let new_width = orig_width / 2;
                                let new_height = orig_height / 2;
                                let mut downscaled = Vec::with_capacity(new_width * new_height * 3);

                                for y in 0..new_height {
                                    for x in 0..new_width {
                                        let src_x = x * 2;
                                        let src_y = y * 2;
                                        let src_idx = (src_y * orig_width + src_x) * 3;
                                        if src_idx + 2 < data.len() {
                                            downscaled.push(data[src_idx]);
                                            downscaled.push(data[src_idx + 1]);
                                            downscaled.push(data[src_idx + 2]);
                                        }
                                    }
                                }

                                let video_frame = VideoFrame {
                                    width: new_width as u32,
                                    height: new_height as u32,
                                    data: downscaled
                                };

                                // Try to send, but don't block if receiver is slow
                                let _ = tx.try_send(video_frame);
                            }
                            Err(e) => {
                                tracing::warn!("JPEG decode error: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Frame capture error: {}", e);
                    }
                }

                // Sleep to maintain frame rate
                let elapsed = frame_start.elapsed();
                if elapsed < frame_duration {
                    thread::sleep(frame_duration - elapsed);
                }
            }

            // Clean up
            let _ = camera.stop_stream();
            tracing::info!("Video capture stopped");
        });

        self.capture_thread = Some(handle);
        Ok(())
    }

    /// Get the latest frame (non-blocking)
    pub fn get_frame(&self) -> Option<VideoFrame> {
        if let Some(rx) = &self.frame_rx {
            // Drain to get the latest frame
            let mut latest = None;
            while let Ok(frame) = rx.try_recv() {
                latest = Some(frame);
            }
            latest
        } else {
            None
        }
    }

    pub fn stop(&mut self) {
        // Signal thread to stop
        self.stop_flag.store(true, Ordering::SeqCst);

        // Wait for thread to finish
        if let Some(handle) = self.capture_thread.take() {
            let _ = handle.join();
        }

        self.frame_rx = None;
    }

    pub fn is_capturing(&self) -> bool {
        self.capture_thread.is_some() && !self.stop_flag.load(Ordering::SeqCst)
    }
}

impl Default for VideoCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for VideoCapture {
    fn drop(&mut self) {
        self.stop();
    }
}
