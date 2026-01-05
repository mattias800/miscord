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
    /// Atomic storage for input level (RMS as f32 bits, after gain)
    level: Arc<AtomicU32>,
    /// Atomic storage for gain (f32 bits, 0.0-3.0)
    gain: Arc<AtomicU32>,
    /// Atomic storage for gate threshold (f32 bits, 0.0-1.0)
    gate_threshold: Arc<AtomicU32>,
    /// Whether gate is enabled
    gate_enabled: Arc<AtomicBool>,
}

impl AudioCapture {
    pub fn new() -> Self {
        Self {
            stream: None,
            is_capturing: false,
            level: Arc::new(AtomicU32::new(0)),
            gain: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            gate_threshold: Arc::new(AtomicU32::new(0.01f32.to_bits())),
            gate_enabled: Arc::new(AtomicBool::new(true)),
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

    /// Set the input gain (0.0 to 3.0, where 1.0 is unity)
    pub fn set_gain(&self, gain: f32) {
        self.gain.store(gain.clamp(0.0, 3.0).to_bits(), Ordering::Relaxed);
    }

    /// Set the gate threshold (0.0 to 1.0)
    pub fn set_gate_threshold(&self, threshold: f32) {
        self.gate_threshold.store(threshold.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Enable or disable the noise gate
    pub fn set_gate_enabled(&self, enabled: bool) {
        self.gate_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Get the current gate threshold
    pub fn get_gate_threshold(&self) -> f32 {
        f32::from_bits(self.gate_threshold.load(Ordering::Relaxed))
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
        let gain = self.gain.clone();
        let gate_threshold = self.gate_threshold.clone();
        let gate_enabled = self.gate_enabled.clone();

        let stream = device.build_input_stream(
            &config.into(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Get current gain and gate settings
                let current_gain = f32::from_bits(gain.load(Ordering::Relaxed));
                let current_threshold = f32::from_bits(gate_threshold.load(Ordering::Relaxed));
                let gate_on = gate_enabled.load(Ordering::Relaxed);

                // Convert to mono by averaging all channels, apply gain
                let mono_samples: Vec<f32> = if channels > 1 {
                    data.chunks(channels)
                        .map(|frame| (frame.iter().sum::<f32>() / channels as f32) * current_gain)
                        .collect()
                } else {
                    data.iter().map(|&s| s * current_gain).collect()
                };

                // Calculate RMS level (after gain, before gate)
                if !mono_samples.is_empty() {
                    let sum: f32 = mono_samples.iter().map(|s| s * s).sum();
                    let rms = (sum / mono_samples.len() as f32).sqrt();
                    // Clamp to 0.0-1.0 range
                    let clamped = rms.min(1.0);
                    level.store(clamped.to_bits(), Ordering::Relaxed);
                }

                // Apply noise gate
                let output_samples: Vec<f32> = if gate_on {
                    let rms = f32::from_bits(level.load(Ordering::Relaxed));
                    if rms < current_threshold {
                        // Below threshold - mute
                        vec![0.0; mono_samples.len()]
                    } else {
                        mono_samples
                    }
                } else {
                    mono_samples
                };

                let _ = tx.try_send(output_samples);
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
