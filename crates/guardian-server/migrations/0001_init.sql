-- Guardian 3.0 schema. All times are Unix seconds (INTEGER).

CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    login         TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('admin', 'teacher')),
    active        INTEGER NOT NULL DEFAULT 1,
    created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);
CREATE INDEX sessions_expires_at ON sessions(expires_at);

CREATE TABLE profiles (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    mode TEXT NOT NULL CHECK (mode IN ('free', 'blacklist', 'whitelist'))
);

CREATE TABLE profile_apps (
    profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    app        TEXT NOT NULL,
    PRIMARY KEY (profile_id, app)
);

CREATE TABLE rooms (
    id                INTEGER PRIMARY KEY,
    name              TEXT NOT NULL UNIQUE,
    active_profile_id INTEGER NOT NULL REFERENCES profiles(id)
);

CREATE TABLE user_rooms (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, room_id)
);

CREATE TABLE settings (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    default_profile_id INTEGER NOT NULL REFERENCES profiles(id),
    retention_days     INTEGER NOT NULL DEFAULT 90,
    state_version      INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE computers (
    id                   INTEGER PRIMARY KEY,
    public_key           BLOB NOT NULL UNIQUE,
    name                 TEXT NOT NULL DEFAULT '',
    hostname             TEXT NOT NULL DEFAULT '',
    os                   TEXT NOT NULL DEFAULT '',
    agent_version        TEXT NOT NULL DEFAULT '',
    room_id              INTEGER REFERENCES rooms(id) ON DELETE SET NULL,
    exception_profile_id INTEGER REFERENCES profiles(id),
    lan_addresses        TEXT NOT NULL DEFAULT '[]',
    enrolled_at          INTEGER NOT NULL,
    last_seen            INTEGER
);

CREATE TABLE consoles (
    id         INTEGER PRIMARY KEY,
    public_key BLOB NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);

CREATE TABLE enroll_codes (
    code       TEXT PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('agent', 'console')),
    room_id    INTEGER REFERENCES rooms(id) ON DELETE CASCADE,
    uses_left  INTEGER NOT NULL,
    max_uses   INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    created_by INTEGER REFERENCES users(id)
);

-- payload = protobuf-encoded CommandEnvelope.
CREATE TABLE commands (
    id         TEXT PRIMARY KEY,
    payload    BLOB NOT NULL,
    created_by INTEGER REFERENCES users(id),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX commands_expires_at ON commands(expires_at);

-- status: pending, delivered, ok, failed, no_user, unsupported, expired.
CREATE TABLE command_targets (
    command_id  TEXT NOT NULL REFERENCES commands(id) ON DELETE CASCADE,
    computer_id INTEGER NOT NULL,
    status      TEXT NOT NULL,
    detail      TEXT NOT NULL DEFAULT '',
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (command_id, computer_id)
);
CREATE INDEX command_targets_computer_status ON command_targets(computer_id, status);

CREATE TABLE audit_log (
    id            INTEGER PRIMARY KEY,
    at            INTEGER NOT NULL,
    actor_user_id INTEGER,
    action        TEXT NOT NULL,
    target        TEXT NOT NULL,
    detail        TEXT NOT NULL,
    source        TEXT NOT NULL CHECK (source IN ('server', 'local'))
);
CREATE INDEX audit_log_at ON audit_log(at);
