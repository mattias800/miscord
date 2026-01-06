pub mod audio;
pub mod capture;
pub mod gst_video;
// pub mod screen; // TODO: Fix xcap compilation issues
pub mod video;

pub use audio::AudioCapture;
pub use capture::CaptureDevice;
pub use gst_video::{GstVideoCapture, VideoDeviceInfo as GstVideoDeviceInfo, VideoFrame as GstVideoFrame};
// pub use screen::ScreenCapture;
pub use video::{VideoCapture, VideoDeviceInfo, VideoFrame};
