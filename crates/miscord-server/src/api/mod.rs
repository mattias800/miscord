mod attachments;
mod auth;
mod channels;
mod communities;
mod messages;
mod opengraph;
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
        // Community routes
        .route("/api/communities", post(communities::create_community).get(communities::list_communities))
        .route(
            "/api/communities/{id}",
            get(communities::get_community)
                .patch(communities::update_community)
                .delete(communities::delete_community),
        )
        .route(
            "/api/communities/{id}/channels",
            get(communities::list_channels).post(communities::create_channel),
        )
        .route("/api/communities/{id}/members", get(communities::list_members))
        .route("/api/communities/{id}/invites", post(communities::create_invite))
        .route("/api/invites/{code}", post(communities::join_community))
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
        // Thread routes
        .route(
            "/api/messages/{id}/thread",
            get(messages::get_thread),
        )
        .route(
            "/api/messages/{id}/replies",
            post(messages::create_thread_reply),
        )
        // Voice routes
        .route("/api/channels/{id}/voice/join", post(channels::join_voice))
        .route("/api/channels/{id}/voice/participants", get(channels::get_voice_participants))
        .route("/api/voice/leave", post(channels::leave_voice))
        .route("/api/voice/state", axum::routing::patch(channels::update_voice_state))
        // Channel read state routes
        .route("/api/channels/{id}/read", post(channels::mark_channel_read))
        .route("/api/channels/{id}/unread", get(channels::get_unread_count))
        // OpenGraph metadata endpoint
        .route("/api/opengraph", get(opengraph::fetch_opengraph))
        // File attachment routes
        .route("/api/channels/{id}/upload", post(attachments::upload_files))
        .route("/api/files/{id}", get(attachments::download_file))
        .route(
            "/api/attachments/{id}",
            get(attachments::get_attachment).delete(attachments::delete_attachment),
        )
        // WebSocket endpoint
        .route("/ws", get(ws::handler::ws_handler))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
