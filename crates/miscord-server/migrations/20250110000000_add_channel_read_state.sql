-- Track when users last read each channel (for unread indicators)

CREATE TABLE IF NOT EXISTS channel_read_states (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    last_read_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, channel_id)
);

-- Index for efficient lookups by user
CREATE INDEX IF NOT EXISTS idx_channel_read_states_user_id ON channel_read_states(user_id);

-- Index for efficient lookups by channel
CREATE INDEX IF NOT EXISTS idx_channel_read_states_channel_id ON channel_read_states(channel_id);
