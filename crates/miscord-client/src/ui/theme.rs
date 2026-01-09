use eframe::egui::Color32;

// Modern dark theme colors - inspired by GitHub Dark, Linear, and Discord
// Darker, richer palette with better contrast

// Backgrounds - deeper darks for a more modern feel
pub const BG_PRIMARY: Color32 = Color32::from_rgb(30, 32, 36);      // Main content background
pub const BG_SECONDARY: Color32 = Color32::from_rgb(24, 26, 30);    // Sidebars, panels
pub const BG_TERTIARY: Color32 = Color32::from_rgb(18, 19, 22);     // Darkest areas, inputs
pub const BG_ACCENT: Color32 = Color32::from_rgb(45, 48, 54);       // Hover states, highlights
pub const BG_ELEVATED: Color32 = Color32::from_rgb(38, 40, 46);     // Cards, popups

// Text colors - high contrast for readability
pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(255, 255, 255);  // Bright white for emphasis
pub const TEXT_NORMAL: Color32 = Color32::from_rgb(230, 232, 236);  // Primary text
pub const TEXT_MUTED: Color32 = Color32::from_rgb(148, 155, 164);   // Secondary text
pub const TEXT_LINK: Color32 = Color32::from_rgb(96, 165, 250);     // Links - softer blue

// Brand/accent colors - deeper, richer blues
pub const BLURPLE: Color32 = Color32::from_rgb(79, 91, 213);        // Primary brand - deeper blue
pub const BLURPLE_LIGHT: Color32 = Color32::from_rgb(96, 108, 230); // Hover state
pub const BLURPLE_DARK: Color32 = Color32::from_rgb(62, 72, 186);   // Active/pressed state

// Status colors
pub const GREEN: Color32 = Color32::from_rgb(72, 187, 120);         // Online/success - vibrant green
pub const YELLOW: Color32 = Color32::from_rgb(251, 191, 36);        // Idle/warning
pub const RED: Color32 = Color32::from_rgb(239, 68, 68);            // DND/Error

pub const CHANNEL_ICON: Color32 = Color32::from_rgb(148, 155, 164); // # icon color
