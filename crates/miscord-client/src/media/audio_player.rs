//! Audio file player for playing audio attachments inline
//!
//! Uses rodio for audio decoding and playback.

use anyhow::Result;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// State shared between the player and UI
pub struct AudioPlayerState {
    /// ID of the attachment being played
    pub attachment_id: Uuid,
    /// Whether playback is active
    pub is_playing: Arc<AtomicBool>,
    /// Current position in milliseconds
    pub position_ms: Arc<AtomicU64>,
    /// Total duration in milliseconds
    pub duration_ms: u64,
    /// Volume (0.0 to 1.0)
    pub volume: Arc<std::sync::atomic::AtomicU32>,
}

impl AudioPlayerState {
    pub fn get_position_ms(&self) -> u64 {
        self.position_ms.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    pub fn get_volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, vol: f32) {
        self.volume.store(vol.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

/// Audio file player using rodio
pub struct AudioPlayer {
    /// The output stream (must be kept alive)
    _stream: OutputStream,
    /// Stream handle for creating sinks
    stream_handle: OutputStreamHandle,
    /// Current playback sink
    sink: Option<Sink>,
    /// Current playback state
    state: Option<Arc<AudioPlayerState>>,
    /// Cached audio data for seeking
    audio_data: Option<Vec<u8>>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) = OutputStream::try_default()?;
        Ok(Self {
            _stream: stream,
            stream_handle,
            sink: None,
            state: None,
            audio_data: None,
        })
    }

    /// Get the current playback state
    pub fn state(&self) -> Option<Arc<AudioPlayerState>> {
        self.state.clone()
    }

    /// Check if we're currently playing a specific attachment
    pub fn is_playing(&self, attachment_id: Uuid) -> bool {
        self.state
            .as_ref()
            .map(|s| s.attachment_id == attachment_id && s.is_playing())
            .unwrap_or(false)
    }

    /// Get the state for a specific attachment (if it's the current one)
    pub fn get_state_for(&self, attachment_id: Uuid) -> Option<Arc<AudioPlayerState>> {
        self.state.as_ref().and_then(|s| {
            if s.attachment_id == attachment_id {
                Some(s.clone())
            } else {
                None
            }
        })
    }

    /// Play audio data from bytes
    pub fn play(&mut self, attachment_id: Uuid, data: Vec<u8>) -> Result<Arc<AudioPlayerState>> {
        // Stop any existing playback
        self.stop();

        // Get duration first
        let duration = Self::get_duration(&data)?;
        let duration_ms = duration.as_millis() as u64;

        // Store data for potential seeking
        self.audio_data = Some(data.clone());

        // Create decoder and sink
        let cursor = Cursor::new(data);
        let source = Decoder::new(cursor)?;

        let sink = Sink::try_new(&self.stream_handle)?;
        sink.append(source);

        // Create shared state
        let state = Arc::new(AudioPlayerState {
            attachment_id,
            is_playing: Arc::new(AtomicBool::new(true)),
            position_ms: Arc::new(AtomicU64::new(0)),
            duration_ms,
            volume: Arc::new(std::sync::atomic::AtomicU32::new(1.0f32.to_bits())),
        });

        // Start position tracking thread
        let state_clone = state.clone();
        let start_time = std::time::Instant::now();
        std::thread::spawn(move || {
            while state_clone.is_playing.load(Ordering::Relaxed) {
                let elapsed = start_time.elapsed().as_millis() as u64;
                let pos = elapsed.min(state_clone.duration_ms);
                state_clone.position_ms.store(pos, Ordering::Relaxed);

                if pos >= state_clone.duration_ms {
                    state_clone.is_playing.store(false, Ordering::Relaxed);
                    break;
                }

                std::thread::sleep(Duration::from_millis(50));
            }
        });

        self.sink = Some(sink);
        self.state = Some(state.clone());

        Ok(state)
    }

    /// Pause playback
    pub fn pause(&mut self) {
        if let Some(sink) = &self.sink {
            sink.pause();
        }
        if let Some(state) = &self.state {
            state.is_playing.store(false, Ordering::Relaxed);
        }
    }

    /// Resume playback
    pub fn resume(&mut self) {
        if let Some(sink) = &self.sink {
            sink.play();
        }
        if let Some(state) = &self.state {
            state.is_playing.store(true, Ordering::Relaxed);
        }
    }

    /// Toggle play/pause
    pub fn toggle(&mut self) {
        if let Some(state) = &self.state {
            if state.is_playing() {
                self.pause();
            } else {
                self.resume();
            }
        }
    }

    /// Stop playback completely
    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        if let Some(state) = &self.state {
            state.is_playing.store(false, Ordering::Relaxed);
        }
        self.state = None;
        self.audio_data = None;
    }

    /// Set volume (0.0 to 1.0)
    pub fn set_volume(&mut self, volume: f32) {
        let vol = volume.clamp(0.0, 1.0);
        if let Some(sink) = &self.sink {
            sink.set_volume(vol);
        }
        if let Some(state) = &self.state {
            state.set_volume(vol);
        }
    }

    /// Seek to a position (0.0 to 1.0)
    pub fn seek(&mut self, position: f32) -> Result<()> {
        let position = position.clamp(0.0, 1.0);

        if let (Some(data), Some(state)) = (&self.audio_data, &self.state) {
            let target_ms = (position * state.duration_ms as f32) as u64;

            // Stop current sink
            if let Some(sink) = self.sink.take() {
                sink.stop();
            }

            // Create new decoder and sink
            let cursor = Cursor::new(data.clone());
            let source = Decoder::new(cursor)?;

            let sink = Sink::try_new(&self.stream_handle)?;

            // Try to skip to position (rodio's seek is limited)
            // For now, we'll just update the position display
            sink.append(source);

            // Apply current volume
            sink.set_volume(state.get_volume());

            // Update position
            state.position_ms.store(target_ms, Ordering::Relaxed);

            if !state.is_playing() {
                sink.pause();
            }

            self.sink = Some(sink);
        }

        Ok(())
    }

    /// Get duration of audio data
    fn get_duration(data: &[u8]) -> Result<Duration> {
        let cursor = Cursor::new(data.to_vec());
        let source = Decoder::new(cursor)?;

        // Estimate duration from sample rate and length
        // This is approximate but works for most formats
        if let Some(duration) = source.total_duration() {
            Ok(duration)
        } else {
            // Fallback: estimate from file size (rough estimate for MP3: ~128kbps)
            let estimated_seconds = data.len() as f64 / 16000.0;
            Ok(Duration::from_secs_f64(estimated_seconds))
        }
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new().expect("Failed to create audio player")
    }
}

/// Format duration as MM:SS
pub fn format_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    let minutes = seconds / 60;
    let secs = seconds % 60;
    format!("{:02}:{:02}", minutes, secs)
}
