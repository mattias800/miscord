use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Information about an audio device
#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub name: String,
    pub is_default: bool,
}

/// List all available input (microphone) devices
pub fn list_input_devices() -> Result<Vec<AudioDevice>> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok());

    let devices: Vec<AudioDevice> = host
        .input_devices()?
        .filter_map(|d| {
            let name = d.name().ok()?;
            Some(AudioDevice {
                is_default: default_name.as_ref() == Some(&name),
                name,
            })
        })
        .collect();

    Ok(devices)
}

/// List all available output (speaker) devices
pub fn list_output_devices() -> Result<Vec<AudioDevice>> {
    let host = cpal::default_host();
    let default_name = host
        .default_output_device()
        .and_then(|d| d.name().ok());

    let devices: Vec<AudioDevice> = host
        .output_devices()?
        .filter_map(|d| {
            let name = d.name().ok()?;
            Some(AudioDevice {
                is_default: default_name.as_ref() == Some(&name),
                name,
            })
        })
        .collect();

    Ok(devices)
}

pub struct AudioCapture {
    stream: Option<cpal::Stream>,
    is_capturing: bool,
    /// Atomic storage for input level (RMS as f32 bits)
    level: Arc<AtomicU32>,
}

impl AudioCapture {
    pub fn new() -> Self {
        Self {
            stream: None,
            is_capturing: false,
            level: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn list_devices() -> Result<Vec<String>> {
        let host = cpal::default_host();
        let devices: Vec<String> = host
            .input_devices()?
            .filter_map(|d| d.name().ok())
            .collect();
        Ok(devices)
    }

    /// Get the current input level (0.0 to 1.0)
    pub fn get_level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Get a clone of the level Arc for external monitoring
    pub fn level_monitor(&self) -> Arc<AtomicU32> {
        self.level.clone()
    }

    pub fn start(&mut self, device_name: Option<&str>) -> Result<mpsc::Receiver<Vec<f32>>> {
        let host = cpal::default_host();

        let device = if let Some(name) = device_name {
            host.input_devices()?
                .find(|d| d.name().map(|n| n == name).unwrap_or(false))
                .ok_or_else(|| anyhow::anyhow!("Device not found: {}", name))?
        } else {
            host.default_input_device()
                .ok_or_else(|| anyhow::anyhow!("No default input device"))?
        };

        let config = device.default_input_config()?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();

        tracing::info!(
            "Starting audio capture: {} Hz, {} channels",
            sample_rate,
            channels
        );

        let (tx, rx) = mpsc::channel(100);
        let level = self.level.clone();

        let stream = device.build_input_stream(
            &config.into(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Calculate RMS level
                if !data.is_empty() {
                    let sum: f32 = data.iter().map(|s| s * s).sum();
                    let rms = (sum / data.len() as f32).sqrt();
                    // Clamp to 0.0-1.0 range
                    let clamped = rms.min(1.0);
                    level.store(clamped.to_bits(), Ordering::Relaxed);
                }

                let samples = data.to_vec();
                let _ = tx.try_send(samples);
            },
            |err| {
                tracing::error!("Audio capture error: {}", err);
            },
            None,
        )?;

        stream.play()?;
        self.stream = Some(stream);
        self.is_capturing = true;

        Ok(rx)
    }

    pub fn stop(&mut self) {
        self.stream = None;
        self.is_capturing = false;
        self.level.store(0.0f32.to_bits(), Ordering::Relaxed);
    }

    pub fn is_capturing(&self) -> bool {
        self.is_capturing
    }
}

impl Default for AudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AudioPlayback {
    stream: Option<cpal::Stream>,
}

impl AudioPlayback {
    pub fn new() -> Self {
        Self { stream: None }
    }

    /// Start playback on a specific device (or default if None)
    pub fn start_with_device(
        &mut self,
        device_name: Option<&str>,
        mut rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<()> {
        let host = cpal::default_host();

        let device = if let Some(name) = device_name {
            host.output_devices()?
                .find(|d| d.name().map(|n| n == name).unwrap_or(false))
                .ok_or_else(|| anyhow::anyhow!("Output device not found: {}", name))?
        } else {
            host.default_output_device()
                .ok_or_else(|| anyhow::anyhow!("No default output device"))?
        };

        let config = device.default_output_config()?;

        let sample_buffer = Arc::new(std::sync::Mutex::new(Vec::new()));
        let buffer_clone = sample_buffer.clone();

        // Spawn thread to receive samples (using std::thread to avoid Tokio runtime requirement)
        std::thread::spawn(move || {
            while let Some(samples) = rx.blocking_recv() {
                let mut buffer = buffer_clone.lock().unwrap();
                buffer.extend(samples);
            }
        });

        let stream = device.build_output_stream(
            &config.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut buffer = sample_buffer.lock().unwrap();
                let len = data.len().min(buffer.len());

                for (i, sample) in data.iter_mut().enumerate() {
                    if i < len {
                        *sample = buffer[i];
                    } else {
                        *sample = 0.0;
                    }
                }

                if len > 0 {
                    buffer.drain(0..len);
                }
            },
            |err| {
                tracing::error!("Audio playback error: {}", err);
            },
            None,
        )?;

        stream.play()?;
        self.stream = Some(stream);

        Ok(())
    }

    /// Start playback on default device (backwards compatibility)
    pub fn start(&mut self, rx: mpsc::Receiver<Vec<f32>>) -> Result<()> {
        self.start_with_device(None, rx)
    }

    pub fn stop(&mut self) {
        self.stream = None;
    }
}

impl Default for AudioPlayback {
    fn default() -> Self {
        Self::new()
    }
}
