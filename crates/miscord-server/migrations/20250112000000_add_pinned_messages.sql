-- Add pinned message support
ALTER TABLE messages ADD COLUMN pinned_at TIMESTAMPTZ;
ALTER TABLE messages ADD COLUMN pinned_by_id UUID REFERENCES users(id) ON DELETE SET NULL;

-- Index for efficient querying of pinned messages per channel
CREATE INDEX idx_messages_channel_pinned ON messages(channel_id, pinned_at DESC) WHERE pinned_at IS NOT NULL;
