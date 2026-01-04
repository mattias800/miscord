use anyhow::Result;
use xcap::{Monitor, Window};

pub struct ScreenFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA data
}

pub struct ScreenCapture {
    monitor: Option<Monitor>,
    window: Option<Window>,
    capture_type: CaptureType,
}

#[derive(Clone, Copy, PartialEq)]
pub enum CaptureType {
    None,
    Monitor,
    Window,
}

impl ScreenCapture {
    pub fn new() -> Self {
        Self {
            monitor: None,
            window: None,
            capture_type: CaptureType::None,
        }
    }

    pub fn list_monitors() -> Result<Vec<MonitorInfo>> {
        let monitors = Monitor::all()?;
        Ok(monitors
            .into_iter()
            .enumerate()
            .map(|(i, m)| MonitorInfo {
                id: i as u32,
                name: m.name().to_string(),
                width: m.width(),
                height: m.height(),
                is_primary: m.is_primary(),
            })
            .collect())
    }

    pub fn list_windows() -> Result<Vec<WindowInfo>> {
        let windows = Window::all()?;
        Ok(windows
            .into_iter()
            .filter(|w| !w.title().is_empty())
            .map(|w| WindowInfo {
                id: w.id(),
                title: w.title().to_string(),
                app_name: w.app_name().to_string(),
                width: w.width(),
                height: w.height(),
            })
            .collect())
    }

    pub fn start_monitor(&mut self, monitor_id: u32) -> Result<()> {
        let monitors = Monitor::all()?;
        let monitor = monitors
            .into_iter()
            .nth(monitor_id as usize)
            .ok_or_else(|| anyhow::anyhow!("Monitor not found"))?;

        tracing::info!(
            "Starting screen capture for monitor: {} ({}x{})",
            monitor.name(),
            monitor.width(),
            monitor.height()
        );

        self.monitor = Some(monitor);
        self.capture_type = CaptureType::Monitor;
        Ok(())
    }

    pub fn start_window(&mut self, window_id: u32) -> Result<()> {
        let windows = Window::all()?;
        let window = windows
            .into_iter()
            .find(|w| w.id() == window_id)
            .ok_or_else(|| anyhow::anyhow!("Window not found"))?;

        tracing::info!(
            "Starting screen capture for window: {} ({}x{})",
            window.title(),
            window.width(),
            window.height()
        );

        self.window = Some(window);
        self.capture_type = CaptureType::Window;
        Ok(())
    }

    pub fn capture_frame(&self) -> Result<Option<ScreenFrame>> {
        match self.capture_type {
            CaptureType::None => Ok(None),
            CaptureType::Monitor => {
                if let Some(monitor) = &self.monitor {
                    let image = monitor.capture_image()?;
                    Ok(Some(ScreenFrame {
                        width: image.width(),
                        height: image.height(),
                        data: image.into_raw(),
                    }))
                } else {
                    Ok(None)
                }
            }
            CaptureType::Window => {
                if let Some(window) = &self.window {
                    let image = window.capture_image()?;
                    Ok(Some(ScreenFrame {
                        width: image.width(),
                        height: image.height(),
                        data: image.into_raw(),
                    }))
                } else {
                    Ok(None)
                }
            }
        }
    }

    pub fn stop(&mut self) {
        self.monitor = None;
        self.window = None;
        self.capture_type = CaptureType::None;
    }

    pub fn is_capturing(&self) -> bool {
        self.capture_type != CaptureType::None
    }
}

impl Default for ScreenCapture {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub width: u32,
    pub height: u32,
}
