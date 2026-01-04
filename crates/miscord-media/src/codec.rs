//! Video codec utilities
//!
//! This module will contain video encoding/decoding using VP8/VP9 or H.264
//! through the webrtc-rs stack or FFmpeg bindings.

use anyhow::Result;

/// Video codec type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    VP8,
    VP9,
    H264,
}

/// Video frame for encoding/decoding
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub format: PixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    I420,
    NV12,
    RGB24,
    RGBA,
}

/// Video encoder trait
pub trait VideoEncoder: Send {
    fn encode(&mut self, frame: &VideoFrame) -> Result<Vec<u8>>;
    fn codec(&self) -> VideoCodec;
}

/// Video decoder trait
pub trait VideoDecoder: Send {
    fn decode(&mut self, data: &[u8]) -> Result<VideoFrame>;
    fn codec(&self) -> VideoCodec;
}

// TODO: Implement VP8/VP9 encoder/decoder using vpx-rs or webrtc-rs
// TODO: Implement H.264 encoder/decoder using openh264 or x264
