pub mod audio;
pub mod capture;
pub mod gst_encoder;
pub mod gst_video;
// pub mod screen; // TODO: Fix xcap compilation issues
pub mod sfu_client;
pub mod vad;
pub mod video;

pub use audio::AudioCapture;
pub use gst_encoder::{GstVp8Decoder, GstVp8Encoder};
pub use capture::CaptureDevice;
pub use gst_video::{GstVideoCapture, VideoDeviceInfo as GstVideoDeviceInfo, VideoFrame as GstVideoFrame};
// pub use screen::ScreenCapture;
pub use sfu_client::{SfuClient, RemoteVideoFrame, IceCandidate, IceCandidateSender};
pub use vad::VoiceActivityDetector;
pub use video::{VideoCapture, VideoDeviceInfo, VideoFrame};
