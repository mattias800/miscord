-- Add thread support to messages table

-- thread_parent_id: which thread this message belongs to (NULL = top-level channel message)
ALTER TABLE messages ADD COLUMN IF NOT EXISTS thread_parent_id UUID REFERENCES messages(id) ON DELETE CASCADE;

-- reply_count: cached count of thread replies (only meaningful on thread starter messages)
ALTER TABLE messages ADD COLUMN IF NOT EXISTS reply_count INTEGER NOT NULL DEFAULT 0;

-- last_reply_at: timestamp of most recent reply (only meaningful on thread starter messages)
ALTER TABLE messages ADD COLUMN IF NOT EXISTS last_reply_at TIMESTAMPTZ;

-- Index for efficient thread queries
CREATE INDEX IF NOT EXISTS idx_messages_thread_parent_id ON messages(thread_parent_id);
