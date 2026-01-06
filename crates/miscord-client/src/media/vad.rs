//! Voice Activity Detection module
//!
//! Uses audio level monitoring to detect when the user is speaking.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Voice Activity Detector
///
/// Monitors audio level and determines if the user is speaking.
/// Uses hysteresis (holdover) to prevent rapid on/off flickering.
pub struct VoiceActivityDetector {
    /// Audio level in dB (stored as f32 bits in AtomicU32)
    level_monitor: Arc<AtomicU32>,
    /// Threshold in dB - audio above this is considered speech
    threshold_db: f32,
    /// Current speaking state
    is_speaking: Arc<AtomicBool>,
    /// Time of last speech detection (for holdover)
    last_speech_time: Option<Instant>,
    /// How long to keep speaking state after audio drops below threshold
    holdover_duration: Duration,
}

impl VoiceActivityDetector {
    /// Create a new VAD with the given level monitor and threshold
    ///
    /// # Arguments
    /// * `level_monitor` - Arc<AtomicU32> from AudioCapture::level_monitor()
    /// * `threshold_db` - Level in dB above which audio is considered speech (e.g., -40.0)
    pub fn new(level_monitor: Arc<AtomicU32>, threshold_db: f32) -> Self {
        Self {
            level_monitor,
            threshold_db,
            is_speaking: Arc::new(AtomicBool::new(false)),
            last_speech_time: None,
            holdover_duration: Duration::from_millis(200), // 200ms holdover
        }
    }

    /// Update VAD state - call this every frame
    ///
    /// Returns true if currently speaking, false otherwise
    pub fn update(&mut self) -> bool {
        let level_db = f32::from_bits(self.level_monitor.load(Ordering::Relaxed));
        let now = Instant::now();

        if level_db > self.threshold_db {
            // Audio above threshold - speaking
            self.last_speech_time = Some(now);
            self.is_speaking.store(true, Ordering::SeqCst);
            true
        } else if let Some(last) = self.last_speech_time {
            // Below threshold - check holdover period
            if now.duration_since(last) < self.holdover_duration {
                // Still in holdover period - keep speaking state
                true
            } else {
                // Holdover expired - stop speaking
                self.is_speaking.store(false, Ordering::SeqCst);
                false
            }
        } else {
            // Never spoke
            self.is_speaking.store(false, Ordering::SeqCst);
            false
        }
    }

    /// Check if currently speaking without updating state
    pub fn is_speaking(&self) -> bool {
        self.is_speaking.load(Ordering::SeqCst)
    }

    /// Get the current audio level in dB
    pub fn get_level_db(&self) -> f32 {
        f32::from_bits(self.level_monitor.load(Ordering::Relaxed))
    }

    /// Set the threshold for speech detection
    pub fn set_threshold(&mut self, threshold_db: f32) {
        self.threshold_db = threshold_db;
    }

    /// Get the current threshold
    pub fn threshold(&self) -> f32 {
        self.threshold_db
    }

    /// Set the holdover duration
    pub fn set_holdover(&mut self, duration: Duration) {
        self.holdover_duration = duration;
    }

    /// Get a clone of the speaking state atomic for sharing
    pub fn speaking_state(&self) -> Arc<AtomicBool> {
        self.is_speaking.clone()
    }
}

impl Default for VoiceActivityDetector {
    fn default() -> Self {
        // Default with a dummy level monitor at -60 dB
        Self {
            level_monitor: Arc::new(AtomicU32::new((-60.0f32).to_bits())),
            threshold_db: -40.0,
            is_speaking: Arc::new(AtomicBool::new(false)),
            last_speech_time: None,
            holdover_duration: Duration::from_millis(200),
        }
    }
}
