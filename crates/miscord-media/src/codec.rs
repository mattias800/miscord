//! Video codec utilities
//!
//! This module contains video encoding/decoding using VP8/VP9 or H.264.
//! The implementations use software codecs for cross-platform compatibility.

use anyhow::{anyhow, Result};

/// Video codec type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    VP8,
    VP9,
    H264,
}

impl VideoCodec {
    /// Returns the MIME type for this codec
    pub fn mime_type(&self) -> &'static str {
        match self {
            VideoCodec::VP8 => "video/VP8",
            VideoCodec::VP9 => "video/VP9",
            VideoCodec::H264 => "video/H264",
        }
    }

    /// Returns the RTP payload type commonly used for this codec
    pub fn payload_type(&self) -> u8 {
        match self {
            VideoCodec::VP8 => 96,
            VideoCodec::VP9 => 98,
            VideoCodec::H264 => 102,
        }
    }
}

/// Video frame for encoding/decoding
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub format: PixelFormat,
    pub timestamp: u64,
    pub is_keyframe: bool,
}

impl VideoFrame {
    /// Create a new video frame
    pub fn new(width: u32, height: u32, format: PixelFormat) -> Self {
        let size = Self::calculate_size(width, height, format);
        Self {
            width,
            height,
            data: vec![0u8; size],
            format,
            timestamp: 0,
            is_keyframe: false,
        }
    }

    /// Calculate buffer size for given dimensions and format
    pub fn calculate_size(width: u32, height: u32, format: PixelFormat) -> usize {
        let pixels = (width * height) as usize;
        match format {
            PixelFormat::I420 => pixels + pixels / 2, // Y + U/4 + V/4
            PixelFormat::NV12 => pixels + pixels / 2, // Y + interleaved UV
            PixelFormat::RGB24 => pixels * 3,
            PixelFormat::RGBA => pixels * 4,
        }
    }

    /// Convert from RGBA to I420 format
    pub fn rgba_to_i420(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        let w = width as usize;
        let h = height as usize;
        let y_size = w * h;
        let uv_size = y_size / 4;
        let mut yuv = vec![0u8; y_size + uv_size * 2];

        // Y plane
        for y in 0..h {
            for x in 0..w {
                let idx = (y * w + x) * 4;
                let r = rgba[idx] as f32;
                let g = rgba[idx + 1] as f32;
                let b = rgba[idx + 2] as f32;
                yuv[y * w + x] = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
            }
        }

        // U and V planes (subsampled 2x2)
        let u_offset = y_size;
        let v_offset = y_size + uv_size;
        for y in (0..h).step_by(2) {
            for x in (0..w).step_by(2) {
                let idx = (y * w + x) * 4;
                let r = rgba[idx] as f32;
                let g = rgba[idx + 1] as f32;
                let b = rgba[idx + 2] as f32;

                let uv_idx = (y / 2) * (w / 2) + (x / 2);
                yuv[u_offset + uv_idx] = (-0.169 * r - 0.331 * g + 0.5 * b + 128.0) as u8;
                yuv[v_offset + uv_idx] = (0.5 * r - 0.419 * g - 0.081 * b + 128.0) as u8;
            }
        }

        yuv
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    I420,
    NV12,
    RGB24,
    RGBA,
}

/// Video encoder configuration
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub bitrate: u32,
    pub framerate: u32,
    pub keyframe_interval: u32,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            bitrate: 2_000_000, // 2 Mbps
            framerate: 30,
            keyframe_interval: 60, // Keyframe every 2 seconds at 30fps
        }
    }
}

/// Video encoder trait
pub trait VideoEncoder: Send {
    fn encode(&mut self, frame: &VideoFrame) -> Result<Vec<u8>>;
    fn codec(&self) -> VideoCodec;
    fn force_keyframe(&mut self);
}

/// Video decoder trait
pub trait VideoDecoder: Send {
    fn decode(&mut self, data: &[u8]) -> Result<VideoFrame>;
    fn codec(&self) -> VideoCodec;
}

/// Software VP8 encoder
///
/// Uses libvpx for encoding. Falls back to passthrough if codec unavailable.
pub struct Vp8Encoder {
    config: EncoderConfig,
    frame_count: u64,
    force_keyframe: bool,
}

impl Vp8Encoder {
    pub fn new(config: EncoderConfig) -> Result<Self> {
        Ok(Self {
            config,
            frame_count: 0,
            force_keyframe: true, // First frame is always keyframe
        })
    }
}

impl VideoEncoder for Vp8Encoder {
    fn encode(&mut self, frame: &VideoFrame) -> Result<Vec<u8>> {
        if frame.width != self.config.width || frame.height != self.config.height {
            return Err(anyhow!(
                "Frame dimensions {}x{} don't match encoder config {}x{}",
                frame.width,
                frame.height,
                self.config.width,
                self.config.height
            ));
        }

        // Placeholder: In a real implementation, this would use libvpx
        // For now, return raw frame data with a simple header
        let is_keyframe =
            self.force_keyframe || (self.frame_count % self.config.keyframe_interval as u64 == 0);

        let mut encoded = Vec::with_capacity(frame.data.len() + 16);

        // Simple header: [magic:4][flags:1][width:2][height:2][timestamp:8]
        encoded.extend_from_slice(b"VP8\0");
        encoded.push(if is_keyframe { 0x01 } else { 0x00 });
        encoded.extend_from_slice(&(frame.width as u16).to_le_bytes());
        encoded.extend_from_slice(&(frame.height as u16).to_le_bytes());
        encoded.extend_from_slice(&frame.timestamp.to_le_bytes());

        // For now, just pass through the raw data
        // Real implementation would encode using vpx_codec_encode
        encoded.extend_from_slice(&frame.data);

        self.frame_count += 1;
        self.force_keyframe = false;

        Ok(encoded)
    }

    fn codec(&self) -> VideoCodec {
        VideoCodec::VP8
    }

    fn force_keyframe(&mut self) {
        self.force_keyframe = true;
    }
}

/// Software VP8 decoder
pub struct Vp8Decoder {
    width: u32,
    height: u32,
}

impl Vp8Decoder {
    pub fn new() -> Self {
        Self {
            width: 0,
            height: 0,
        }
    }
}

impl Default for Vp8Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoDecoder for Vp8Decoder {
    fn decode(&mut self, data: &[u8]) -> Result<VideoFrame> {
        if data.len() < 17 {
            return Err(anyhow!("Data too short for VP8 frame"));
        }

        if &data[0..4] != b"VP8\0" {
            return Err(anyhow!("Invalid VP8 magic"));
        }

        let is_keyframe = data[4] & 0x01 != 0;
        let width = u16::from_le_bytes([data[5], data[6]]) as u32;
        let height = u16::from_le_bytes([data[7], data[8]]) as u32;
        let timestamp = u64::from_le_bytes([
            data[9], data[10], data[11], data[12], data[13], data[14], data[15], data[16],
        ]);

        self.width = width;
        self.height = height;

        Ok(VideoFrame {
            width,
            height,
            data: data[17..].to_vec(),
            format: PixelFormat::I420,
            timestamp,
            is_keyframe,
        })
    }

    fn codec(&self) -> VideoCodec {
        VideoCodec::VP8
    }
}

/// Create a video encoder for the specified codec
pub fn create_encoder(codec: VideoCodec, config: EncoderConfig) -> Result<Box<dyn VideoEncoder>> {
    match codec {
        VideoCodec::VP8 => Ok(Box::new(Vp8Encoder::new(config)?)),
        VideoCodec::VP9 => {
            // VP9 uses similar API to VP8, could use same structure
            // For now, fallback to VP8
            tracing::warn!("VP9 not implemented, falling back to VP8");
            Ok(Box::new(Vp8Encoder::new(config)?))
        }
        VideoCodec::H264 => {
            // H.264 would require openh264 or x264 bindings
            Err(anyhow!("H.264 encoder not yet implemented"))
        }
    }
}

/// Create a video decoder for the specified codec
pub fn create_decoder(codec: VideoCodec) -> Result<Box<dyn VideoDecoder>> {
    match codec {
        VideoCodec::VP8 => Ok(Box::new(Vp8Decoder::new())),
        VideoCodec::VP9 => {
            tracing::warn!("VP9 not implemented, falling back to VP8");
            Ok(Box::new(Vp8Decoder::new()))
        }
        VideoCodec::H264 => Err(anyhow!("H.264 decoder not yet implemented")),
    }
}
