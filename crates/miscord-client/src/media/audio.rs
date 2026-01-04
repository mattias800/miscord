use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct AudioCapture {
    stream: Option<cpal::Stream>,
    is_capturing: bool,
}

impl AudioCapture {
    pub fn new() -> Self {
        Self {
            stream: None,
            is_capturing: false,
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

        let stream = device.build_input_stream(
            &config.into(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
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

    pub fn start(&mut self, mut rx: mpsc::Receiver<Vec<f32>>) -> Result<()> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("No default output device"))?;

        let config = device.default_output_config()?;

        let sample_buffer = Arc::new(std::sync::Mutex::new(Vec::new()));
        let buffer_clone = sample_buffer.clone();

        // Spawn task to receive samples
        tokio::spawn(async move {
            while let Some(samples) = rx.recv().await {
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

    pub fn stop(&mut self) {
        self.stream = None;
    }
}

impl Default for AudioPlayback {
    fn default() -> Self {
        Self::new()
    }
}
