//! Persistent settings storage
//!
//! Saves user preferences to a local JSON file.
//! Also handles session persistence for automatic login.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Get the current profile name from environment or use "default"
fn get_profile_name() -> String {
    std::env::var("MISCORD_PROFILE").unwrap_or_else(|_| "default".to_string())
}

/// Get the base config directory for miscord
fn get_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("miscord"))
}

/// Persistent user session for automatic login
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Session {
    /// The auth token from login
    pub auth_token: String,
    /// The server URL used for login
    pub server_url: String,
    /// User ID
    pub user_id: String,
    /// Username for display
    pub username: String,
}

impl Session {
    /// Get the session file path for the current profile
    fn session_path() -> Option<PathBuf> {
        let profile = get_profile_name();
        get_config_dir().map(|p| p.join("sessions").join(format!("{}.json", profile)))
    }

    /// Load session from disk
    pub fn load() -> Option<Self> {
        let path = Self::session_path()?;

        if !path.exists() {
            tracing::debug!("No saved session for profile '{}'", get_profile_name());
            return None;
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(session) => {
                    tracing::info!(
                        "Loaded session for profile '{}' from {:?}",
                        get_profile_name(),
                        path
                    );
                    Some(session)
                }
                Err(e) => {
                    tracing::error!("Failed to parse session file: {}", e);
                    None
                }
            },
            Err(e) => {
                tracing::debug!("Failed to read session file: {}", e);
                None
            }
        }
    }

    /// Save session to disk
    pub fn save(&self) {
        let Some(path) = Self::session_path() else {
            tracing::warn!("Could not determine config directory");
            return;
        };

        // Create directory if it doesn't exist
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!("Failed to create sessions directory: {}", e);
                return;
            }
        }

        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::error!("Failed to write session file: {}", e);
                } else {
                    tracing::info!(
                        "Saved session for profile '{}' to {:?}",
                        get_profile_name(),
                        path
                    );
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize session: {}", e);
            }
        }
    }

    /// Delete the saved session (for logout)
    pub fn delete() {
        if let Some(path) = Self::session_path() {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    tracing::error!("Failed to delete session file: {}", e);
                } else {
                    tracing::info!("Deleted session for profile '{}'", get_profile_name());
                }
            }
        }
    }
}

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

/// Persistent UI state (community/channel selection, collapsed sections)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiState {
    /// Currently selected community ID
    pub current_community_id: Option<Uuid>,
    /// Currently selected text channel ID
    pub current_channel_id: Option<Uuid>,
    /// Whether the text channels section is expanded
    #[serde(default = "default_true")]
    pub text_channels_expanded: bool,
    /// Whether the voice channels section is expanded
    #[serde(default = "default_true")]
    pub voice_channels_expanded: bool,
}

fn default_true() -> bool {
    true
}

impl UiState {
    /// Get the UI state file path for the current profile
    fn ui_state_path() -> Option<PathBuf> {
        let profile = get_profile_name();
        get_config_dir().map(|p| p.join("ui_state").join(format!("{}.json", profile)))
    }

    /// Load UI state from disk
    pub fn load() -> Self {
        let Some(path) = Self::ui_state_path() else {
            tracing::warn!("Could not determine config directory for UI state");
            return Self::default();
        };

        if !path.exists() {
            tracing::debug!("UI state file does not exist, using defaults");
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(state) => {
                    tracing::info!("Loaded UI state from {:?}", path);
                    state
                }
                Err(e) => {
                    tracing::error!("Failed to parse UI state file: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                tracing::error!("Failed to read UI state file: {}", e);
                Self::default()
            }
        }
    }

    /// Save UI state to disk
    pub fn save(&self) {
        let Some(path) = Self::ui_state_path() else {
            tracing::warn!("Could not determine config directory for UI state");
            return;
        };

        // Create directory if it doesn't exist
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!("Failed to create UI state directory: {}", e);
                return;
            }
        }

        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::error!("Failed to write UI state file: {}", e);
                } else {
                    tracing::debug!("Saved UI state to {:?}", path);
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize UI state: {}", e);
            }
        }
    }
}
