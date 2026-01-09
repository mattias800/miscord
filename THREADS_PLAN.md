# Miscord Threads Implementation Plan

## Overview

Add Slack-style message threads to Miscord, allowing users to have organized sub-conversations within channels. Thread replies are regular messages with a parent reference, reusing all existing message infrastructure.

## Core Design Principle

**A thread reply IS a message.** Instead of creating separate entities, we extend the existing `Message` entity with a thread parent reference. This means:
- All existing message features (edit, delete, reactions) work automatically for thread replies
- No code duplication
- Consistent behavior across channel messages and thread replies
- Simpler maintenance

---

## Database Schema Changes

### Current Schema

The `messages` table already has:
```sql
reply_to_id UUID REFERENCES messages(id) ON DELETE SET NULL
```

This field is used for **reply previews** ("replying to X" shown above a message). We keep this as-is.

### New Fields

Add three fields to the `messages` table:

```sql
ALTER TABLE messages ADD COLUMN thread_parent_id UUID REFERENCES messages(id) ON DELETE CASCADE;
ALTER TABLE messages ADD COLUMN reply_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN last_reply_at TIMESTAMPTZ;

CREATE INDEX idx_messages_thread_parent_id ON messages(thread_parent_id);
```

### Field Purposes

| Field | Purpose | When Set |
|-------|---------|----------|
| `reply_to_id` (existing) | Shows "replying to X" preview above message | When user explicitly replies to a specific message |
| `thread_parent_id` (new) | Identifies which thread this message belongs to | Always set for messages inside a thread |
| `reply_count` (new) | Cached count of thread replies | Only on thread starter messages |
| `last_reply_at` (new) | Timestamp of most recent reply | Only on thread starter messages |

### Examples

```
Channel message (not in thread):
  thread_parent_id = NULL
  reply_to_id = NULL

First reply in a thread (to message A):
  thread_parent_id = A
  reply_to_id = NULL (or A, if we want to show reply preview)

Reply to message B inside thread started by A:
  thread_parent_id = A    (still belongs to thread A)
  reply_to_id = B         (shows "replying to B" preview)

Channel message replying to another channel message C:
  thread_parent_id = NULL (not in a thread)
  reply_to_id = C         (shows "replying to C" preview)
```

**Logic:**
- `thread_parent_id == NULL` → Channel message (top-level)
- `thread_parent_id != NULL` → Thread message
- `reply_to_id != NULL` → Show reply preview regardless of thread context

---

## Backend Changes

### Model Changes (miscord-server)

**Update `crates/miscord-server/src/models/message.rs`:**

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Message {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub author_id: Uuid,
    pub content: String,
    pub edited_at: Option<DateTime<Utc>>,
    pub reply_to_id: Option<Uuid>,      // Existing - for reply previews
    pub thread_parent_id: Option<Uuid>, // New - thread membership
    pub reply_count: i32,               // New - cached thread reply count
    pub last_reply_at: Option<DateTime<Utc>>, // New - latest reply timestamp
    pub created_at: DateTime<Utc>,
}
```

### Protocol Changes (miscord-protocol)

**Update `crates/miscord-protocol/src/types.rs`:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageData {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub author_id: Uuid,
    pub author_name: String,
    pub content: String,
    pub edited_at: Option<DateTime<Utc>>,
    pub reply_to_id: Option<Uuid>,
    #[serde(default)]
    pub reactions: Vec<ReactionData>,
    pub created_at: DateTime<Utc>,
    // New thread fields
    pub thread_parent_id: Option<Uuid>,
    #[serde(default)]
    pub reply_count: i32,
    pub last_reply_at: Option<DateTime<Utc>>,
}

/// Lightweight message preview for reply context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePreview {
    pub id: Uuid,
    pub author_id: Uuid,
    pub author_name: String,
    pub content: String, // Truncated if long
}

/// Thread data with parent and replies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadData {
    pub parent_message: MessageData,
    pub replies: Vec<MessageData>,
    pub total_reply_count: i32,
}
```

### Service Changes

**Extend `crates/miscord-server/src/services/message.rs`:**

```rust
impl MessageService {
    // New method: Create a thread reply
    pub async fn create_thread_reply(
        &self,
        parent_message_id: Uuid,
        author_id: Uuid,
        content: String,
        reply_to_id: Option<Uuid>, // Optional reply-to within thread
    ) -> Result<Message> {
        // 1. Verify parent exists and get its channel_id
        let parent = self.get_by_id(parent_message_id).await?;

        // 2. Create the reply with thread_parent_id set
        let message = sqlx::query_as!(
            Message,
            r#"
            INSERT INTO messages (id, channel_id, author_id, content, reply_to_id, thread_parent_id, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, NOW())
            RETURNING *
            "#,
            Uuid::new_v4(),
            parent.channel_id,
            author_id,
            content,
            reply_to_id,
            parent_message_id,
        )
        .fetch_one(&self.db)
        .await?;

        // 3. Update parent's reply_count and last_reply_at
        sqlx::query!(
            r#"
            UPDATE messages
            SET reply_count = reply_count + 1, last_reply_at = NOW()
            WHERE id = $1
            "#,
            parent_message_id
        )
        .execute(&self.db)
        .await?;

        Ok(message)
    }

    // New method: Get thread with replies
    pub async fn get_thread(
        &self,
        parent_message_id: Uuid,
        limit: i64,
        before: Option<Uuid>,
    ) -> Result<Vec<Message>> {
        let messages = if let Some(before_id) = before {
            let before_msg = self.get_by_id(before_id).await?;
            sqlx::query_as!(
                Message,
                r#"
                SELECT * FROM messages
                WHERE thread_parent_id = $1 AND created_at < $2
                ORDER BY created_at ASC
                LIMIT $3
                "#,
                parent_message_id,
                before_msg.created_at,
                limit
            )
            .fetch_all(&self.db)
            .await?
        } else {
            sqlx::query_as!(
                Message,
                r#"
                SELECT * FROM messages
                WHERE thread_parent_id = $1
                ORDER BY created_at ASC
                LIMIT $2
                "#,
                parent_message_id,
                limit
            )
            .fetch_all(&self.db)
            .await?
        };

        Ok(messages)
    }

    // Update existing delete to handle thread reply count
    pub async fn delete(&self, id: Uuid, author_id: Uuid) -> Result<Option<Uuid>> {
        // Get the message first to check if it's a thread reply
        let message = self.get_by_id(id).await?;
        let thread_parent_id = message.thread_parent_id;

        let result = sqlx::query!(
            "DELETE FROM messages WHERE id = $1 AND author_id = $2",
            id,
            author_id
        )
        .execute(&self.db)
        .await?;

        if result.rows_affected() == 0 {
            return Err(AppError::NotFound("Message not found or not owned by user".to_string()));
        }

        // If this was a thread reply, decrement parent's reply_count
        if let Some(parent_id) = thread_parent_id {
            sqlx::query!(
                "UPDATE messages SET reply_count = reply_count - 1 WHERE id = $1",
                parent_id
            )
            .execute(&self.db)
            .await?;
        }

        Ok(thread_parent_id)
    }
}
```

### Modify Channel Messages Query

**Update `list_by_channel` to exclude thread messages:**

```rust
pub async fn list_by_channel(
    &self,
    channel_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Message>> {
    let messages = if let Some(before_id) = before {
        let before_msg = self.get_by_id(before_id).await?;
        sqlx::query_as!(
            Message,
            r#"
            SELECT * FROM messages
            WHERE channel_id = $1
              AND created_at < $2
              AND thread_parent_id IS NULL  -- Exclude thread messages
            ORDER BY created_at DESC
            LIMIT $3
            "#,
            channel_id,
            before_msg.created_at,
            limit
        )
        .fetch_all(&self.db)
        .await?
    } else {
        sqlx::query_as!(
            Message,
            r#"
            SELECT * FROM messages
            WHERE channel_id = $1
              AND thread_parent_id IS NULL  -- Exclude thread messages
            ORDER BY created_at DESC
            LIMIT $2
            "#,
            channel_id,
            limit
        )
        .fetch_all(&self.db)
        .await?
    };

    Ok(messages)
}
```

### New API Endpoints

**Add to `crates/miscord-server/src/api/messages.rs`:**

```rust
// GET /api/messages/{parent_id}/thread
pub async fn get_thread(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(parent_id): Path<Uuid>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<ThreadData>> {
    let limit = query.limit.unwrap_or(50).min(100);

    // Get parent message
    let parent = state.message_service.get_by_id(parent_id).await?;
    let parent_author = state.user_service.get_by_id(parent.author_id).await?;

    // Get thread replies
    let replies = state.message_service
        .get_thread(parent_id, limit, query.before)
        .await?;

    // Convert to MessageData with author names
    // ... (similar to list_messages)

    Ok(Json(ThreadData {
        parent_message: parent_data,
        replies: replies_data,
        total_reply_count: parent.reply_count,
    }))
}

// POST /api/messages/{parent_id}/replies
pub async fn create_thread_reply(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(parent_id): Path<Uuid>,
    Json(input): Json<CreateMessage>,
) -> Result<Json<MessageData>> {
    let message = state.message_service
        .create_thread_reply(parent_id, auth.user_id, input.content, input.reply_to_id)
        .await?;

    // Get author name, build MessageData...
    // Broadcast to thread subscribers and update parent in channel

    Ok(Json(message_data))
}
```

**Add routes in `crates/miscord-server/src/api/mod.rs`:**

```rust
.route("/api/messages/:parent_id/thread", get(messages::get_thread))
.route("/api/messages/:parent_id/replies", post(messages::create_thread_reply))
```

---

## WebSocket Changes

### New Message Types

**Update `crates/miscord-protocol/src/messages.rs`:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ClientMessage {
    // Existing...
    SubscribeChannel { channel_id: Uuid },

    // New thread messages
    SubscribeThread { parent_message_id: Uuid },
    UnsubscribeThread { parent_message_id: Uuid },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ServerMessage {
    // Existing...
    MessageCreated { message: MessageData },

    // New thread messages
    ThreadReplyCreated {
        parent_message_id: Uuid,
        message: MessageData,
    },
    ThreadMetadataUpdated {
        message_id: Uuid,
        reply_count: i32,
        last_reply_at: Option<DateTime<Utc>>,
    },
}
```

### WebSocket Handler Changes

**Update `crates/miscord-server/src/ws/handler.rs`:**

```rust
// Track thread subscriptions per connection
struct ConnectionState {
    user_id: Uuid,
    subscribed_channels: HashSet<Uuid>,
    subscribed_threads: HashSet<Uuid>, // New
}

// Handle thread subscription
ClientMessage::SubscribeThread { parent_message_id } => {
    connection_state.subscribed_threads.insert(parent_message_id);
}

ClientMessage::UnsubscribeThread { parent_message_id } => {
    connection_state.subscribed_threads.remove(&parent_message_id);
}
```

### Broadcast Logic

When a thread reply is created:
1. Broadcast `ThreadReplyCreated` to users subscribed to that thread
2. Broadcast `ThreadMetadataUpdated` to users subscribed to the channel (to update reply count badge)

---

## Client UI Changes

### State Changes

**Update `crates/miscord-client/src/state/app_state.rs`:**

```rust
pub struct AppStateInner {
    // Existing...

    // Thread state
    pub open_thread: Option<OpenThread>,
    pub thread_messages: HashMap<Uuid, Vec<MessageData>>, // parent_id -> replies
}

pub struct OpenThread {
    pub parent_message: MessageData,
    pub reply_input: String,
}
```

### Network Changes

**Add to `crates/miscord-client/src/network/mod.rs`:**

```rust
impl NetworkClient {
    pub async fn get_thread(&self, parent_message_id: Uuid) -> Result<ThreadData> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        api::get(
            &format!("{}/api/messages/{}/thread", server_url, parent_message_id),
            token.as_deref(),
        )
        .await
    }

    pub async fn create_thread_reply(
        &self,
        parent_message_id: Uuid,
        content: &str,
        reply_to_id: Option<Uuid>,
    ) -> Result<MessageData> {
        let server_url = self.get_server_url().await;
        let token = self.get_token().await;

        #[derive(serde::Serialize)]
        struct CreateReply {
            content: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            reply_to_id: Option<Uuid>,
        }

        api::post(
            &format!("{}/api/messages/{}/replies", server_url, parent_message_id),
            &CreateReply { content: content.to_string(), reply_to_id },
            token.as_deref(),
        )
        .await
    }

    pub async fn subscribe_thread(&self, parent_message_id: Uuid) {
        if let Some(client) = self.ws_client.read().await.as_ref() {
            client.subscribe_thread(parent_message_id).await;
        }
    }

    pub async fn unsubscribe_thread(&self, parent_message_id: Uuid) {
        if let Some(client) = self.ws_client.read().await.as_ref() {
            client.unsubscribe_thread(parent_message_id).await;
        }
    }
}
```

### UI Components

**New `crates/miscord-client/src/ui/thread_panel.rs`:**

```rust
pub struct ThreadPanel {
    reply_input: String,
}

impl ThreadPanel {
    pub fn new() -> Self {
        Self {
            reply_input: String::new(),
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        let open_thread = runtime.block_on(async {
            state.read().await.open_thread.clone()
        });

        let Some(thread) = open_thread else {
            return;
        };

        // Header with close button
        ui.horizontal(|ui| {
            ui.heading("Thread");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("X").clicked() {
                    let state = state.clone();
                    let network = network.clone();
                    let parent_id = thread.parent_message.id;
                    runtime.spawn(async move {
                        network.unsubscribe_thread(parent_id).await;
                        state.write().await.open_thread = None;
                    });
                }
            });
        });

        ui.separator();

        // Parent message preview
        ui.group(|ui| {
            ui.label(
                egui::RichText::new(&thread.parent_message.author_name)
                    .strong()
            );
            ui.label(&thread.parent_message.content);
            ui.label(
                egui::RichText::new(format!(
                    "{} replies",
                    thread.parent_message.reply_count
                ))
                .small()
                .color(egui::Color32::GRAY)
            );
        });

        ui.separator();

        // Thread replies (scrollable)
        let replies = runtime.block_on(async {
            state.read().await
                .thread_messages
                .get(&thread.parent_message.id)
                .cloned()
                .unwrap_or_default()
        });

        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for reply in &replies {
                    // Render each reply (similar to chat messages)
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&reply.author_name)
                                .strong()
                                .color(egui::Color32::from_rgb(88, 101, 242))
                        );
                        ui.label(&reply.content);
                    });
                    ui.add_space(4.0);
                }
            });

        ui.separator();

        // Reply input
        ui.horizontal(|ui| {
            let response = ui.text_edit_singleline(&mut self.reply_input);

            if ui.button("Reply").clicked() ||
               (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                if !self.reply_input.trim().is_empty() {
                    let content = self.reply_input.clone();
                    self.reply_input.clear();

                    let network = network.clone();
                    let parent_id = thread.parent_message.id;
                    runtime.spawn(async move {
                        let _ = network.create_thread_reply(parent_id, &content, None).await;
                    });
                }
            }
        });
    }
}
```

**Update message rendering in `chat.rs` to show thread indicator:**

```rust
// After message content, show thread indicator if has replies
if message.reply_count > 0 {
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let thread_btn = ui.button(
            egui::RichText::new(format!(
                "💬 {} replies · Last reply {}",
                message.reply_count,
                format_relative_time(message.last_reply_at.unwrap_or(message.created_at))
            ))
            .small()
            .color(egui::Color32::from_rgb(88, 101, 242))
        );

        if thread_btn.clicked() {
            // Open thread panel
            let state = state.clone();
            let network = network.clone();
            let msg = message.clone();
            runtime.spawn(async move {
                network.subscribe_thread(msg.id).await;
                if let Ok(thread_data) = network.get_thread(msg.id).await {
                    let mut s = state.write().await;
                    s.open_thread = Some(OpenThread {
                        parent_message: msg,
                        reply_input: String::new(),
                    });
                    s.thread_messages.insert(thread_data.parent_message.id, thread_data.replies);
                }
            });
        }
    });
}

// Add "Start thread" to action buttons
let thread_btn = ui.small_button("💬");
if thread_btn.clicked() {
    // Open thread panel for this message (even if no replies yet)
    // ...
}
thread_btn.on_hover_text("Reply in thread");
```

### Layout Integration

**Update `main_view.rs` to show thread panel:**

The thread panel replaces the member list when open (similar to Discord):

```rust
// Right panel - Thread panel takes priority over member list
if let Some(_) = runtime.block_on(async { state.read().await.open_thread.clone() }) {
    egui::SidePanel::right("thread_panel")
        .exact_width(350.0)
        .show(ctx, |ui| {
            self.thread_panel.show(ui, state, network, runtime);
        });
} else if has_community {
    egui::SidePanel::right("member_panel")
        .exact_width(240.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.member_list.show(ui, state, runtime);
            });
        });
}
```

**Note:** When in voice channel AND thread is open, thread panel takes priority. User can close thread to see voice controls. Consider adding a toggle or split view in future.

---

## Migration

**Create `crates/miscord-server/migrations/YYYYMMDDHHMMSS_add_threads.sql`:**

```sql
-- Add thread support to messages table
ALTER TABLE messages ADD COLUMN thread_parent_id UUID REFERENCES messages(id) ON DELETE CASCADE;
ALTER TABLE messages ADD COLUMN reply_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN last_reply_at TIMESTAMPTZ;

-- Index for efficient thread queries
CREATE INDEX idx_messages_thread_parent_id ON messages(thread_parent_id);

-- No data migration needed - existing messages have NULL thread_parent_id (top-level)
```

---

## Implementation Checklist

### Phase 1: Database & Models ✅

- [x] Create SQL migration file
- [x] Run migration
- [x] Update `Message` struct in `models/message.rs`
- [x] Update `CreateMessage` struct to include `thread_parent_id`
- [x] Update `MessageData` in `miscord-protocol/src/types.rs`
- [x] Add `MessagePreview` struct to protocol
- [x] Add `ThreadData` struct to protocol

### Phase 2: Backend Service ✅

- [x] Add `create_thread_reply` method to `MessageService`
- [x] Add `get_thread` method to `MessageService`
- [x] Update `list_by_channel` to exclude thread messages (`WHERE thread_parent_id IS NULL`)
- [x] Update `delete` to decrement parent's `reply_count`
- [x] Add validation: thread parent must exist and be in accessible channel

### Phase 3: API Endpoints ✅

- [x] Add `GET /api/messages/{parent_id}/thread` endpoint
- [x] Add `POST /api/messages/{parent_id}/replies` endpoint
- [x] Add routes to router
- [x] Test existing message endpoints work for thread replies

### Phase 4: WebSocket ✅

- [x] Add `SubscribeThread` / `UnsubscribeThread` client messages
- [x] Add `ThreadReplyCreated` server message
- [x] Add `ThreadMetadataUpdated` server message
- [x] Track thread subscriptions in connection state
- [x] Broadcast thread replies to subscribers
- [x] Broadcast metadata updates to channel subscribers

### Phase 5: Client Network ✅

- [x] Add `get_thread` method to `NetworkClient`
- [x] Add `create_thread_reply` method to `NetworkClient`
- [x] Add `subscribe_thread` / `unsubscribe_thread` methods
- [x] Add WebSocket methods to `WebSocketClient`
- [x] Handle `ThreadReplyCreated` message
- [x] Handle `ThreadMetadataUpdated` message

### Phase 6: Client State ✅

- [x] Add `open_thread: Option<OpenThread>` to `AppStateInner`
- [x] Add `thread_messages: HashMap<Uuid, Vec<MessageData>>`
- [x] Add `OpenThread` struct
- [x] Add methods to open/close thread

### Phase 7: Client UI ✅

- [x] Create `ThreadPanel` component
- [x] Add thread indicator to messages with `reply_count > 0`
- [x] Add "Reply in thread" button to message actions
- [x] Update `MainView` to show thread panel vs member list
- [x] Handle thread panel layout with voice channel view

### Phase 8: Polish ✅

- [x] Scroll to bottom when thread opens
- [x] Scroll to bottom on new reply
- [x] Handle parent message deleted while thread open
- [x] Empty state for threads with no replies
- [x] Loading state while fetching thread

### Phase 9: Testing ✅

- [x] Test creating thread reply
- [x] Test reply_count increment/decrement
- [x] Test channel messages exclude thread messages
- [x] Test thread fetch with pagination
- [x] Test WebSocket broadcasts
- [x] Test edit/delete thread replies (existing endpoints)
- [x] Test reactions on thread replies (existing endpoints)

---

## What We're NOT Building (Reusing Instead)

| Feature | Reused From |
|---------|-------------|
| Reply editing | Existing `PATCH /api/messages/{id}` |
| Reply deletion | Existing `DELETE /api/messages/{id}` |
| Reply reactions | Existing reaction endpoints |
| Reply entity | Existing `Message` entity |
| Reply DTO | Existing `MessageData` (extended) |

---

## Future Enhancements (Out of Scope)

- Unread thread tracking per user
- Thread notifications
- Thread search
- "Also send to channel" option when replying in thread
- Thread following/muting
- Thread previews on hover
- Jump to message when clicking reply preview
- Split view for thread + voice controls

---

**Last Updated:** 2026-01-09
**Status:** ✅ COMPLETED - Slack-style threads fully implemented with real-time updates
