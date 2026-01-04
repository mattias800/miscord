-- User status enum
CREATE TYPE user_status AS ENUM ('offline', 'online', 'idle', 'donotdisturb', 'invisible');

-- Channel type enum
CREATE TYPE channel_type AS ENUM ('text', 'voice', 'direct_message', 'group_dm');

-- Users table
CREATE TABLE users (
    id UUID PRIMARY KEY,
    username VARCHAR(32) NOT NULL UNIQUE,
    display_name VARCHAR(64) NOT NULL,
    email VARCHAR(255) NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    avatar_url TEXT,
    status user_status NOT NULL DEFAULT 'offline',
    custom_status VARCHAR(128),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_users_username ON users(username);
CREATE INDEX idx_users_email ON users(email);

-- Friendships table
CREATE TABLE friendships (
    id UUID PRIMARY KEY,
    user1_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user2_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status VARCHAR(20) NOT NULL DEFAULT 'pending', -- pending, accepted, blocked
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user1_id, user2_id)
);

CREATE INDEX idx_friendships_user1 ON friendships(user1_id);
CREATE INDEX idx_friendships_user2 ON friendships(user2_id);

-- Servers table
CREATE TABLE servers (
    id UUID PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    description TEXT,
    icon_url TEXT,
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_servers_owner ON servers(owner_id);

-- Server members table
CREATE TABLE server_members (
    id UUID PRIMARY KEY,
    server_id UUID NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    nickname VARCHAR(64),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(server_id, user_id)
);

CREATE INDEX idx_server_members_server ON server_members(server_id);
CREATE INDEX idx_server_members_user ON server_members(user_id);

-- Server roles table
CREATE TABLE server_roles (
    id UUID PRIMARY KEY,
    server_id UUID NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    name VARCHAR(64) NOT NULL,
    color VARCHAR(7), -- Hex color
    permissions BIGINT NOT NULL DEFAULT 0,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_server_roles_server ON server_roles(server_id);

-- Member roles junction table
CREATE TABLE member_roles (
    member_id UUID NOT NULL REFERENCES server_members(id) ON DELETE CASCADE,
    role_id UUID NOT NULL REFERENCES server_roles(id) ON DELETE CASCADE,
    PRIMARY KEY (member_id, role_id)
);

-- Server invites table
CREATE TABLE server_invites (
    id UUID PRIMARY KEY,
    server_id UUID NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    code VARCHAR(16) NOT NULL UNIQUE,
    created_by UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    uses INTEGER NOT NULL DEFAULT 0,
    max_uses INTEGER,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_server_invites_code ON server_invites(code);

-- Channels table
CREATE TABLE channels (
    id UUID PRIMARY KEY,
    server_id UUID REFERENCES servers(id) ON DELETE CASCADE, -- NULL for DMs
    name VARCHAR(100) NOT NULL,
    topic TEXT,
    channel_type channel_type NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_channels_server ON channels(server_id);

-- Direct message channels
CREATE TABLE direct_message_channels (
    id UUID PRIMARY KEY,
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    user1_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user2_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user1_id, user2_id)
);

CREATE INDEX idx_dm_channels_user1 ON direct_message_channels(user1_id);
CREATE INDEX idx_dm_channels_user2 ON direct_message_channels(user2_id);

-- Group DM channels
CREATE TABLE group_dm_channels (
    id UUID PRIMARY KEY,
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    name VARCHAR(100),
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Group DM members
CREATE TABLE group_dm_members (
    id UUID PRIMARY KEY,
    group_dm_id UUID NOT NULL REFERENCES group_dm_channels(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(group_dm_id, user_id)
);

-- Voice states (who is in which voice channel)
CREATE TABLE voice_states (
    id UUID PRIMARY KEY,
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    muted BOOLEAN NOT NULL DEFAULT FALSE,
    deafened BOOLEAN NOT NULL DEFAULT FALSE,
    self_muted BOOLEAN NOT NULL DEFAULT FALSE,
    self_deafened BOOLEAN NOT NULL DEFAULT FALSE,
    video_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    screen_sharing BOOLEAN NOT NULL DEFAULT FALSE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id) -- User can only be in one voice channel at a time
);

CREATE INDEX idx_voice_states_channel ON voice_states(channel_id);

-- Messages table
CREATE TABLE messages (
    id UUID PRIMARY KEY,
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    author_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    edited_at TIMESTAMPTZ,
    reply_to_id UUID REFERENCES messages(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_messages_channel ON messages(channel_id);
CREATE INDEX idx_messages_channel_time ON messages(channel_id, created_at DESC);
CREATE INDEX idx_messages_author ON messages(author_id);

-- Message attachments
CREATE TABLE message_attachments (
    id UUID PRIMARY KEY,
    message_id UUID NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    filename VARCHAR(255) NOT NULL,
    content_type VARCHAR(100) NOT NULL,
    size_bytes BIGINT NOT NULL,
    url TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_attachments_message ON message_attachments(message_id);

-- Message reactions
CREATE TABLE message_reactions (
    id UUID PRIMARY KEY,
    message_id UUID NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    emoji VARCHAR(64) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(message_id, user_id, emoji)
);

CREATE INDEX idx_reactions_message ON message_reactions(message_id);
