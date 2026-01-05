pub mod audio;
pub mod capture;
// pub mod screen; // TODO: Fix xcap compilation issues
pub mod video;

pub use audio::AudioCapture;
pub use capture::CaptureDevice;
// pub use screen::ScreenCapture;
pub use video::VideoCapture;
