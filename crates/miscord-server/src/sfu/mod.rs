//! SFU (Selective Forwarding Unit) for video streaming
//!
//! This module implements a zero-copy RTP packet forwarding system for video streams.
//! Each client sends their video to the SFU, which forwards it to all other participants
//! without any processing or transcoding.

mod session;
mod track_router;

pub use session::SfuSessionManager;
pub use track_router::TrackRouter;
