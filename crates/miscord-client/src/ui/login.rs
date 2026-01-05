use eframe::egui;

use crate::network::NetworkClient;
use crate::state::{LoginRequest, RegisterRequest};
use miscord_protocol::UserData;

pub struct LoginView {
    mode: LoginMode,
    username: String,
    password: String,
    email: String,
    display_name: String,
    server_url: String,
    error: Option<String>,
    is_loading: bool,
    auto_login_attempted: bool,
}

enum LoginMode {
    Login,
    Register,
}

/// Configuration loaded from environment variables
pub struct AutoLoginConfig {
    pub server_url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl AutoLoginConfig {
    pub fn from_env() -> Self {
        Self {
            server_url: std::env::var("MISCORD_SERVER_URL").ok(),
            username: std::env::var("MISCORD_AUTO_LOGIN_USER").ok(),
            password: std::env::var("MISCORD_AUTO_LOGIN_PASS").ok(),
        }
    }

    pub fn has_credentials(&self) -> bool {
        self.username.is_some() && self.password.is_some()
    }
}

impl LoginView {
    pub fn new() -> Self {
        let config = AutoLoginConfig::from_env();

        Self {
            mode: LoginMode::Login,
            username: config.username.clone().unwrap_or_default(),
            password: config.password.clone().unwrap_or_default(),
            email: String::new(),
            display_name: String::new(),
            server_url: config
                .server_url
                .unwrap_or_else(|| "http://localhost:8080".to_string()),
            error: None,
            is_loading: false,
            auto_login_attempted: false,
        }
    }

    /// Check if auto-login should be attempted
    pub fn should_auto_login(&self) -> bool {
        !self.auto_login_attempted && AutoLoginConfig::from_env().has_credentials()
    }

    /// Attempt automatic login using environment credentials
    pub fn try_auto_login(
        &mut self,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) -> Option<(String, UserData)> {
        self.auto_login_attempted = true;

        let config = AutoLoginConfig::from_env();

        if let (Some(username), Some(password)) = (config.username, config.password) {
            tracing::info!("Attempting auto-login as {}", username);

            let request = LoginRequest { username, password };
            let server_url = self.server_url.clone();

            match runtime.block_on(network.login(&server_url, request)) {
                Ok((token, user)) => {
                    tracing::info!("Auto-login successful");
                    return Some((token, user));
                }
                Err(e) => {
                    tracing::warn!("Auto-login failed: {}", e);
                    self.error = Some(format!("Auto-login failed: {}", e));
                }
            }
        }

        None
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) -> Option<(String, UserData)> {
        let mut result = None;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);

                ui.heading("Miscord");
                ui.add_space(20.0);

                ui.group(|ui| {
                    ui.set_width(300.0);

                    // Server URL
                    ui.horizontal(|ui| {
                        ui.label("Server:");
                        ui.text_edit_singleline(&mut self.server_url);
                    });

                    ui.add_space(10.0);

                    match self.mode {
                        LoginMode::Login => {
                            ui.horizontal(|ui| {
                                ui.label("Username:");
                                ui.text_edit_singleline(&mut self.username);
                            });

                            ui.horizontal(|ui| {
                                ui.label("Password:");
                                ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
                            });

                            ui.add_space(10.0);

                            if ui.button("Login").clicked() && !self.is_loading {
                                self.error = None;
                                self.is_loading = true;

                                let request = LoginRequest {
                                    username: self.username.clone(),
                                    password: self.password.clone(),
                                };

                                let network = network.clone();
                                let server_url = self.server_url.clone();

                                match runtime.block_on(network.login(&server_url, request)) {
                                    Ok((token, user)) => {
                                        result = Some((token, user));
                                    }
                                    Err(e) => {
                                        self.error = Some(e.to_string());
                                    }
                                }

                                self.is_loading = false;
                            }

                            ui.add_space(5.0);

                            if ui.link("Don't have an account? Register").clicked() {
                                self.mode = LoginMode::Register;
                                self.error = None;
                            }
                        }
                        LoginMode::Register => {
                            ui.horizontal(|ui| {
                                ui.label("Username:");
                                ui.text_edit_singleline(&mut self.username);
                            });

                            ui.horizontal(|ui| {
                                ui.label("Display Name:");
                                ui.text_edit_singleline(&mut self.display_name);
                            });

                            ui.horizontal(|ui| {
                                ui.label("Email:");
                                ui.text_edit_singleline(&mut self.email);
                            });

                            ui.horizontal(|ui| {
                                ui.label("Password:");
                                ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
                            });

                            ui.add_space(10.0);

                            if ui.button("Register").clicked() && !self.is_loading {
                                self.error = None;
                                self.is_loading = true;

                                let request = RegisterRequest {
                                    username: self.username.clone(),
                                    display_name: if self.display_name.is_empty() {
                                        self.username.clone()
                                    } else {
                                        self.display_name.clone()
                                    },
                                    email: self.email.clone(),
                                    password: self.password.clone(),
                                };

                                let network = network.clone();
                                let server_url = self.server_url.clone();

                                match runtime.block_on(network.register(&server_url, request)) {
                                    Ok(_) => {
                                        // After registration, switch to login
                                        self.mode = LoginMode::Login;
                                        self.error = None;
                                    }
                                    Err(e) => {
                                        self.error = Some(e.to_string());
                                    }
                                }

                                self.is_loading = false;
                            }

                            ui.add_space(5.0);

                            if ui.link("Already have an account? Login").clicked() {
                                self.mode = LoginMode::Login;
                                self.error = None;
                            }
                        }
                    }

                    if let Some(error) = &self.error {
                        ui.add_space(10.0);
                        ui.colored_label(egui::Color32::RED, error);
                    }

                    if self.is_loading {
                        ui.add_space(10.0);
                        ui.spinner();
                    }
                });
            });
        });

        result
    }
}

impl Default for LoginView {
    fn default() -> Self {
        Self::new()
    }
}
