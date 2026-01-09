# Text Channel & Direct Message Improvement Plan

This document outlines planned improvements for the text channel and direct message experience in Miscord.

## Current State

### Text Channels
- Basic send/receive with real-time SignalR updates
- Message editing with "(edited)" indicator
- Message deletion (own messages + admin permissions)
- Markdown support with syntax-highlighted code blocks
- User avatars, timestamps, and presence indicators
- Pagination support exists but UI doesn't utilize it (always loads 50)

### Direct Messages
- Same core messaging features as text channels
- Conversation list with last message preview
- Unread count badges and "NEW MESSAGES" separator
- Read receipts (bulk - marks all as read when conversation opened)
- Online/offline status indicator on avatars

---

## Improvement Roadmap

### Phase 1: Quick Wins (Client-side only) ✅ COMPLETED

These improvements require minimal changes and can be implemented quickly:

#### 1.1 Date Separators ✅
- [ ] Group messages by day in the chat view
- [ ] Show dividers with "Today", "Yesterday", or full date (e.g., "January 5, 2026")
- [ ] Apply to both text channels and DMs

#### 1.2 Selected Conversation Highlight ✅
- [ ] Add visual distinction for the currently selected DM conversation
- [ ] Background color change on selected state

#### 1.3 Relative Timestamps ✅
- [ ] Show "Just now", "2 min ago", "1 hour ago" for recent messages
- [ ] Show full timestamp on hover via tooltip
- [ ] Fall back to full date for messages older than 24 hours

#### 1.4 Message Formatting Toolbar ✅
- [ ] Add toolbar above message input with Bold, Italic, Code buttons
- [ ] Insert markdown syntax when clicked
- [ ] Help users who don't know markdown shortcuts

---

### Phase 2: High Impact Features ✅ COMPLETED

These features significantly improve the core messaging experience:

#### 2.1 Typing Indicators ✅
- [ ] Show "User is typing..." when someone is composing a message
- [ ] Throttled SignalR events (send every ~3 seconds while typing)
- [ ] Auto-dismiss after ~5 seconds of inactivity
- [ ] Support multiple users typing simultaneously in channels

#### 2.2 Unread Indicators for Text Channels ✅
- [ ] Track last read message per user per channel (server-side)
- [ ] Show unread badge on channel list in sidebar
- [ ] Display "NEW MESSAGES" separator when returning to a channel
- [ ] API endpoint to mark channel as read

#### 2.3 Message Replies/Threading ✅
- [ ] Add "Reply" button on message hover
- [ ] Show quoted preview above the reply message
- [ ] Click preview to jump to original message
- [ ] Visual connection line between reply and original

#### 2.4 Link Previews ✅
- [ ] Parse URLs in message content
- [ ] Make URLs clickable (opens in browser)
- [ ] Style URLs with blue color and underline
- [ ] Fetch OpenGraph metadata (title, description, image)
- [ ] Display preview card below message
- [ ] Server-side proxy to avoid CORS issues
- [ ] Support YouTube embeds (via oEmbed API)

#### 2.5 User Mentions (@username) ✅
- [ ] Autocomplete dropdown when typing `@`
- [ ] Filter by partial username match
- [ ] Highlight mentions in rendered message
- [ ] Notification/ping when user is mentioned
- [ ] `@everyone` and `@here` for channels (admin only)

---

### Phase 3: Medium Impact Features

Polish and enhanced functionality:

#### 3.1 Message Reactions ✅
- [ ] Add emoji reactions to any message
- [ ] Show reaction counts below message
- [ ] Tooltip showing who reacted
- [ ] Emoji picker UI (common emojis)
- [ ] Database model for reactions
- [ ] Real-time sync via SignalR
- [ ] Emoji search in picker
- [ ] Custom emoji reactions (requires 3.2 File Attachments)

#### 3.2 File Attachments ✅
- [ ] Upload images and files with messages
- [ ] Drag-and-drop support
- [ ] Image previews inline (lightbox on click)
- [ ] Download links for non-image files
- [ ] File size limits and validation (25MB max, configurable extensions)
- [ ] Server-side storage (local directory with GUID-based filenames)
- [ ] Audio file playback with inline player
  - [ ] Play/pause, progress bar, seek functionality
  - [ ] Current time and total duration display
  - [ ] Volume control slider
  - [ ] Uses LibVLCSharp (requires VLC on macOS arm64)

#### 3.3 Infinite Scroll / Message History
- [ ] Implement scroll-to-load-more for older messages
- [ ] Use existing pagination API (skip/take)
- [ ] "Jump to present" button when scrolled up
- [ ] Loading indicator while fetching
- [ ] Smart auto-scroll (scroll to new messages only when at bottom, preserve position when reading older messages)

#### 3.4 Message Search
- [ ] Search within current channel or DM
- [ ] Full-text search across all messages
- [ ] Filter by user, date range
- [ ] Jump to message in context
- [ ] Search results highlighting

#### 3.5 Pinned Messages ✅
- [ ] Pin important messages to a channel
- [ ] "Pinned" button in channel header
- [ ] Pinned messages panel/modal
- [ ] Unpin option for admins/message author

---

### Phase 4: Nice to Have

Lower priority enhancements:

#### 4.1 Delivery Status Indicators
- [ ] Checkmarks showing message status: sent → delivered → read
- [ ] Particularly useful for DMs
- [ ] Server acknowledgment on receive

#### 4.2 GIF Support ✅
- [ ] Integration with Tenor API (server-side proxy to hide API key)
- [ ] Picker UI in message composer (popup with search and trending GIFs)
- [ ] GIF search and trending endpoints
- [ ] Inline GIF display (detects Tenor URLs, renders inline)
- [ ] Image caching for performance
- [ ] Sticker packs (future enhancement)

#### 4.3 Voice/Video Calls in DMs
- [ ] Add call button in DM conversation header
- [ ] 1-on-1 voice call using existing WebRTC infrastructure
- [ ] Video call support
- [ ] Call history

#### 4.4 Draft Messages
- [ ] Auto-save unsent message when switching channels
- [ ] Restore draft when returning to channel
- [ ] Visual indicator that draft exists
- [ ] Local storage (not server-side)

#### 4.5 Compact Mode
- [ ] Toggle for denser message display
- [ ] Hide avatars, reduce vertical spacing
- [ ] User preference saved in settings

#### 4.6 Keyboard Navigation
- [ ] Arrow keys to navigate messages
- [ ] `E` to edit selected message
- [ ] `R` to reply to selected message
- [ ] `Delete` to delete selected message
- [ ] `Ctrl+K` / `Cmd+K` for quick channel/DM switcher
- [ ] `Escape` to deselect/cancel

---

## Implementation Notes

### Files to Modify (Text/DM Focus)

**Client-side (Rust/egui):**
- `crates/miscord-client/src/ui/chat.rs` - Channel chat UI (date separators, timestamps, replies, reactions, typing)
- `crates/miscord-client/src/ui/markdown.rs` - Markdown rendering for messages
- `crates/miscord-client/src/state/app_state.rs` - State management (reactions, typing users)
- `crates/miscord-client/src/network/websocket.rs` - WebSocket message handling

**Server-side (Rust/Axum):**
- `crates/miscord-server/src/services/message.rs` - Message service (reactions)
- `crates/miscord-server/src/api/messages.rs` - Message API endpoints
- `crates/miscord-server/src/ws/handler.rs` - WebSocket handler (typing, reactions broadcasts)

**Shared protocol:**
- `crates/miscord-protocol/src/types.rs` - Data types (MessageData, ReactionData)
- `crates/miscord-protocol/src/messages.rs` - WebSocket message types

### Coordination with Other Agents

Another agent is currently working on server, community, and user account management. To avoid conflicts:

1. **File ownership**: This plan focuses on messaging UI (Views, ViewModels, Converters)
2. **Shared files**: Avoid `Program.cs`, `MiscordDbContext.cs`, `User.cs` unless necessary
3. **Communication**: Check `git status` before editing shared files
4. **Small commits**: Make focused, atomic commits for easier merging

---

## Progress Tracking

### Completed (Rust/egui implementation)
- [x] Initial plan created
- [x] Phase 1: Quick Wins
  - [x] Date separators (shows "Today", "Yesterday", or full date between messages from different days)
  - [ ] Selected conversation highlight (DMs show selected state) - DM feature not fully implemented
  - [x] Relative timestamps with full timestamp tooltip ("Just now", "2m ago", etc.)
  - [x] Message formatting toolbar (Bold, Italic, Strikethrough, Code, Code block, Link buttons)
- [x] Phase 2: High Impact Features
  - [x] Typing indicators (shows "User is typing..." via WebSocket events)
  - [x] Unread indicators for text channels (badge with count, bright text, mark-as-read on select)
  - [x] Message replies/threading - Slack-style threads with side panel, real-time updates
  - [x] Link previews - OpenGraph endpoint, clickable URLs in messages
  - [x] User mentions (@username) - autocomplete dropdown, keyboard navigation, highlight in messages
  - [x] Smart auto-scroll - Uses egui's stick_to_bottom

### In Progress
- [ ] Phase 3: Medium Impact Features
  - [x] Message reactions (emoji picker, counts, real-time sync via WebSocket, user ID tracking)
  - [ ] File attachments - Not yet implemented
  - [ ] Pinned messages - Not yet implemented
  - [ ] Infinite scroll / Message history - Not yet implemented
  - [ ] Message search - Not yet implemented

### Not Started
- [ ] Phase 4: Nice to Have
  - [ ] GIF support (Tenor API integration, picker UI, inline display)
  - [ ] Delivery status indicators
  - [ ] Voice/Video calls in DMs
  - [ ] Draft messages
  - [ ] Compact mode
  - [ ] Keyboard navigation

### Other Improvements
- [x] UI state persistence (community/channel selection, collapsed sections)
