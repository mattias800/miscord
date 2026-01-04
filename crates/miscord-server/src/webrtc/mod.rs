use crate::auth::AuthUser;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IceServersResponse {
    pub ice_servers: Vec<IceServer>,
}

pub async fn get_ice_servers(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Json<IceServersResponse> {
    let mut ice_servers = vec![];

    // Add STUN servers
    for stun_url in &state.config.stun_servers {
        ice_servers.push(IceServer {
            urls: vec![stun_url.clone()],
            username: None,
            credential: None,
        });
    }

    // Add TURN servers
    for turn in &state.config.turn_servers {
        ice_servers.push(IceServer {
            urls: vec![turn.url.clone()],
            username: Some(turn.username.clone()),
            credential: Some(turn.credential.clone()),
        });
    }

    Json(IceServersResponse { ice_servers })
}
