-- The addresses an account sends as, from its provider: `[{name, email, signature}]`.
ALTER TABLE accounts ADD COLUMN identities JSONB NOT NULL DEFAULT '[]';

-- Sent to a list or by a machine, and how to leave the list: `{url, mailto, one_click}`.
ALTER TABLE messages ADD COLUMN bulk BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE messages ADD COLUMN unsubscribe JSONB;

-- A label made in the apps has no provider id until the account's worker has made it there.
ALTER TABLE labels ALTER COLUMN provider_id DROP NOT NULL;

-- The user's synced settings. The core decides the keys; the server reads `muted:` and `blocked:`.
CREATE TABLE preferences (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value JSONB NOT NULL DEFAULT 'null',
    rev BIGINT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (user_id, key)
);

CREATE INDEX preferences_by_user ON preferences(user_id, rev);

-- Drafts kept for every device of the user. The device that started one chose its id.
CREATE TABLE drafts (
    id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    draft JSONB NOT NULL,
    updated TIMESTAMPTZ NOT NULL DEFAULT now(),
    rev BIGINT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (user_id, id)
);

CREATE INDEX drafts_by_user ON drafts(user_id, rev);

-- Files uploaded to go out with a draft; gone after the send that used them, or a week.
CREATE TABLE uploads (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    mime TEXT NOT NULL,
    bytes BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE outgoing ADD COLUMN remind_at TIMESTAMPTZ;

-- Sent mail to snooze until `remind_at` once it has been synced.
CREATE TABLE reminders (
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    remind_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (account_id, provider_id)
);

-- What search matches, weighted so a query can ask for one part: the sender is A, the recipients
-- B, the subject C, the words of the message D. Addresses come with their domain on its own.
CREATE FUNCTION message_search(from_name TEXT, from_email TEXT, recipients JSONB, subject TEXT, words TEXT)
RETURNS tsvector LANGUAGE sql IMMUTABLE AS $$
    SELECT setweight(to_tsvector('simple',
               coalesce(from_name, '') || ' ' || from_email || ' ' || split_part(from_email, '@', 2)), 'A')
        || setweight(to_tsvector('simple', coalesce((
               SELECT string_agg(coalesce(address->>'name', '') || ' ' || (address->>'email') || ' '
                   || split_part(address->>'email', '@', 2), ' ')
               FROM jsonb_array_elements(coalesce(recipients->'to', '[]') || coalesce(recipients->'cc', '[]')
                   || coalesce(recipients->'bcc', '[]')) AS address), '')), 'B')
        || setweight(to_tsvector('simple', subject), 'C')
        || to_tsvector('simple', words)
$$;

-- The server rebuilds the search of mail synced before this, a batch at a time in id order, after
-- the id kept here; the row goes once it is done.
CREATE TABLE search_backfill (after UUID NOT NULL);
INSERT INTO search_backfill VALUES ('00000000-0000-0000-0000-000000000000');

-- Search and the newest inbox mail walk these newest first and stop early.
DROP INDEX messages_by_date;
CREATE INDEX messages_by_user_date ON messages(user_id, date DESC) WHERE NOT deleted;
CREATE INDEX messages_inbox ON messages(account_id, date DESC) WHERE NOT deleted AND 'inbox' = ANY(labels);
-- A reply ends the snooze of its thread's messages.
CREATE INDEX messages_snoozed_threads ON messages(account_id, thread_id) WHERE snoozed_until IS NOT NULL;
