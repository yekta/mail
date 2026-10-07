-- Every synced row takes its rev from this one sequence. A client asks for what changed after
-- the highest rev it has.
CREATE SEQUENCE revs;

CREATE TABLE users (
    id UUID PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A Google sign-in an app started. `id` is the state sent to Google; `code_hash` is set once
-- Google has answered and is what the app exchanges, with the secret behind `challenge`.
-- `link_user_id` is the user whose link ticket started it, for a second account.
CREATE TABLE sign_ins (
    id TEXT PRIMARY KEY,
    challenge TEXT NOT NULL,
    app_state TEXT NOT NULL,
    google_verifier TEXT NOT NULL,
    nonce TEXT NOT NULL,
    link_user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE link_tickets (
    ticket_hash TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE accounts (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    address TEXT NOT NULL,
    -- For JMAP the session URL and username; Gmail needs none.
    login TEXT NOT NULL DEFAULT '',
    -- AES-GCM sealed with SECRET_KEY: the refresh token or the password.
    credentials BYTEA NOT NULL,
    sync_state JSONB NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'syncing',
    color TEXT NOT NULL,
    rev BIGINT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX accounts_by_login ON accounts(provider, address, login) WHERE NOT deleted;
CREATE INDEX accounts_by_user ON accounts(user_id, rev);

CREATE TABLE labels (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    name TEXT NOT NULL,
    rev BIGINT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT false,
    UNIQUE (account_id, provider_id)
);

CREATE INDEX labels_by_user ON labels(user_id, rev);

CREATE TABLE messages (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    from_name TEXT,
    from_email TEXT NOT NULL DEFAULT '',
    recipients JSONB NOT NULL DEFAULT '{}',
    subject TEXT NOT NULL DEFAULT '',
    snippet TEXT NOT NULL DEFAULT '',
    date TIMESTAMPTZ NOT NULL,
    unread BOOLEAN NOT NULL DEFAULT false,
    starred BOOLEAN NOT NULL DEFAULT false,
    -- Roles (inbox, sent, drafts, trash, spam) and the ids of custom labels.
    labels TEXT[] NOT NULL DEFAULT '{}',
    attachments JSONB NOT NULL DEFAULT '[]',
    message_id TEXT,
    in_reply_to TEXT,
    "references" TEXT[] NOT NULL DEFAULT '{}',
    snoozed_until TIMESTAMPTZ,
    search tsvector,
    rev BIGINT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT false,
    UNIQUE (account_id, provider_id)
);

CREATE INDEX messages_by_user ON messages(user_id, rev);
CREATE INDEX messages_by_date ON messages(account_id, date DESC) WHERE NOT deleted;
CREATE INDEX messages_snoozed ON messages(snoozed_until) WHERE snoozed_until IS NOT NULL;
CREATE INDEX messages_search ON messages USING gin(search);

CREATE TABLE bodies (
    message_id UUID PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    html TEXT,
    text TEXT,
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Changes made here that the provider doesn't have yet. The account's worker sends them.
CREATE TABLE provider_ops (
    id BIGSERIAL PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id UUID NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    op JSONB NOT NULL,
    attempts INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX provider_ops_by_account ON provider_ops(account_id, id);
CREATE INDEX provider_ops_by_message ON provider_ops(message_id);

-- Ops a client sent, so that one sent twice is applied once.
CREATE TABLE client_ops (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    op_id TEXT NOT NULL,
    ok BOOLEAN NOT NULL,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, op_id)
);

-- Mail waiting to be sent: undo-send and send-later both wait for `send_at`.
CREATE TABLE outgoing (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    draft JSONB NOT NULL,
    send_at TIMESTAMPTZ NOT NULL,
    -- pending, sending, sent, failed or canceled.
    status TEXT NOT NULL DEFAULT 'pending',
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX outgoing_due ON outgoing(send_at) WHERE status = 'pending';
