//! Media processing utilities for Miscord
//!
//! This crate provides audio and video processing capabilities including:
//! - Audio encoding/decoding (Opus)
//! - Video encoding/decoding
//! - WebRTC media handling

pub mod audio;
pub mod codec;

pub use audio::*;
