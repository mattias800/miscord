use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
        let channels = config.channels() as usize;

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

                // Convert to mono by averaging all channels
                let mono_samples: Vec<f32> = if channels > 1 {
                    data.chunks(channels)
                        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                        .collect()
                } else {
                    data.to_vec()
                };

                let _ = tx.try_send(mono_samples);
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
    stop_flag: Arc<AtomicBool>,
}

impl AudioPlayback {
    pub fn new() -> Self {
        Self {
            stream: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
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
        let output_channels = config.channels() as usize;

        tracing::info!(
            "Starting audio playback: {} Hz, {} channels",
            config.sample_rate().0,
            output_channels
        );

        let sample_buffer = Arc::new(std::sync::Mutex::new(Vec::<f32>::new()));
        let buffer_clone = sample_buffer.clone();

        // Reset stop flag
        self.stop_flag.store(false, Ordering::SeqCst);
        let stop_flag = self.stop_flag.clone();

        // Spawn thread to receive mono samples and expand to output channels
        std::thread::spawn(move || {
            while !stop_flag.load(Ordering::SeqCst) {
                // Use a timeout to periodically check stop flag
                match rx.blocking_recv() {
                    Some(mono_samples) => {
                        // Expand mono to output channels (duplicate sample to all channels)
                        let expanded: Vec<f32> = mono_samples
                            .iter()
                            .flat_map(|&sample| std::iter::repeat(sample).take(output_channels))
                            .collect();

                        let mut buffer = buffer_clone.lock().unwrap();
                        // Limit buffer size to prevent memory growth
                        const MAX_BUFFER_SIZE: usize = 48000 * 2; // ~1 second at 48kHz stereo
                        if buffer.len() < MAX_BUFFER_SIZE {
                            buffer.extend(expanded);
                        }
                    }
                    None => break, // Channel closed
                }
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
        // Signal the receiver thread to stop
        self.stop_flag.store(true, Ordering::SeqCst);
        // Drop the stream to stop playback
        self.stream = None;
    }
}

impl Default for AudioPlayback {
    fn default() -> Self {
        Self::new()
    }
}
