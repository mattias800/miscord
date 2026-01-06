//! Persistent settings storage
//!
//! Saves user preferences to a local JSON file.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Persistent user settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistentSettings {
    /// Selected audio input device name
    pub input_device: Option<String>,
    /// Selected audio output device name
    pub output_device: Option<String>,
    /// Selected video device index
    pub video_device: Option<u32>,
    /// Input gain in dB
    pub input_gain_db: Option<f32>,
    /// Gate threshold in dB
    pub gate_threshold_db: Option<f32>,
    /// Gate enabled
    pub gate_enabled: Option<bool>,
    /// Loopback enabled
    pub loopback_enabled: Option<bool>,
}

impl PersistentSettings {
    /// Get the settings file path
    fn settings_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("miscord").join("settings.json"))
    }

    /// Load settings from disk
    pub fn load() -> Self {
        let Some(path) = Self::settings_path() else {
            tracing::warn!("Could not determine config directory");
            return Self::default();
        };

        if !path.exists() {
            tracing::debug!("Settings file does not exist, using defaults");
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(settings) => {
                    tracing::info!("Loaded settings from {:?}", path);
                    settings
                }
                Err(e) => {
                    tracing::error!("Failed to parse settings file: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                tracing::error!("Failed to read settings file: {}", e);
                Self::default()
            }
        }
    }

    /// Save settings to disk
    pub fn save(&self) {
        let Some(path) = Self::settings_path() else {
            tracing::warn!("Could not determine config directory");
            return;
        };

        // Create directory if it doesn't exist
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!("Failed to create config directory: {}", e);
                return;
            }
        }

        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::error!("Failed to write settings file: {}", e);
                } else {
                    tracing::debug!("Saved settings to {:?}", path);
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize settings: {}", e);
            }
        }
    }
}
