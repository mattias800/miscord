mod auth;
mod channels;
mod messages;
mod servers;
mod users;

use crate::state::AppState;
use crate::ws;
use axum::{
    routing::{get, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health check
        .route("/health", get(|| async { "OK" }))
        // Auth routes
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        // User routes
        .route("/api/users/me", get(users::get_me).patch(users::update_me))
        .route("/api/users/{id}", get(users::get_user))
        .route("/api/users/me/friends", get(users::get_friends))
        // Server routes
        .route("/api/servers", post(servers::create_server).get(servers::list_servers))
        .route(
            "/api/servers/{id}",
            get(servers::get_server)
                .patch(servers::update_server)
                .delete(servers::delete_server),
        )
        .route(
            "/api/servers/{id}/channels",
            get(servers::list_channels).post(servers::create_channel),
        )
        .route("/api/servers/{id}/invites", post(servers::create_invite))
        .route("/api/invites/{code}", post(servers::join_server))
        // Channel routes
        .route(
            "/api/channels/{id}",
            get(channels::get_channel)
                .patch(channels::update_channel)
                .delete(channels::delete_channel),
        )
        .route(
            "/api/channels/{id}/messages",
            get(messages::list_messages).post(messages::create_message),
        )
        // DM routes
        .route("/api/dms", get(channels::list_dms))
        .route("/api/dms/{user_id}", post(channels::create_dm))
        // Message routes
        .route(
            "/api/messages/{id}",
            axum::routing::patch(messages::update_message).delete(messages::delete_message),
        )
        .route(
            "/api/messages/{id}/reactions/{emoji}",
            post(messages::add_reaction).delete(messages::remove_reaction),
        )
        // Voice routes
        .route("/api/channels/{id}/voice/join", post(channels::join_voice))
        .route("/api/voice/leave", post(channels::leave_voice))
        .route("/api/voice/state", axum::routing::patch(channels::update_voice_state))
        // WebSocket endpoint
        .route("/ws", get(ws::handler::ws_handler))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
