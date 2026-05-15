-- Projects
CREATE TABLE projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

-- Gateway keys
CREATE TABLE gateway_keys (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    prefix        TEXT NOT NULL,
    hash          BLOB NOT NULL,
    last4         TEXT NOT NULL,
    scopes        TEXT NOT NULL,             -- JSON array
    status        TEXT NOT NULL,             -- active | revoked | expired
    expires_at    INTEGER,
    last_used_at  INTEGER,
    created_at    INTEGER NOT NULL,
    revoked_at    INTEGER
);
CREATE INDEX idx_keys_hash      ON gateway_keys(hash);
CREATE INDEX idx_keys_project   ON gateway_keys(project_id);

-- Provider credentials
CREATE TABLE provider_credentials (
    id             TEXT PRIMARY KEY,
    project_id     TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    provider       TEXT NOT NULL,
    name           TEXT NOT NULL,
    encrypted_key  BLOB NOT NULL,
    status         TEXT NOT NULL,
    created_at     INTEGER NOT NULL
);
CREATE INDEX idx_provider_credentials_project ON provider_credentials(project_id, provider);

-- Routes (one config blob per project)
CREATE TABLE routes (
    project_id  TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    config      TEXT NOT NULL,                -- JSON
    version     INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- Pricing catalog
CREATE TABLE pricing (
    provider             TEXT NOT NULL,
    model                TEXT NOT NULL,
    input_per_1k         REAL NOT NULL,
    output_per_1k        REAL NOT NULL,
    cached_input_per_1k  REAL,
    effective_from       INTEGER NOT NULL,
    effective_to         INTEGER,
    PRIMARY KEY (provider, model, effective_from)
);

-- Request logs
CREATE TABLE request_logs (
    id                 TEXT PRIMARY KEY,
    project_id         TEXT NOT NULL,
    gateway_key_id     TEXT,
    provider           TEXT,
    model              TEXT,
    endpoint           TEXT,
    request_ts         INTEGER NOT NULL,
    duration_ms        INTEGER,
    upstream_ms        INTEGER,
    ttfb_ms            INTEGER,
    status             TEXT NOT NULL,
    http_status        INTEGER,
    cached             INTEGER NOT NULL DEFAULT 0,
    retry_count        INTEGER NOT NULL DEFAULT 0,
    fallback_used      TEXT,
    prompt_tokens      INTEGER,
    completion_tokens  INTEGER,
    cached_tokens      INTEGER,
    total_tokens       INTEGER,
    cost_usd           REAL,
    would_have_cost_usd REAL,
    metadata           TEXT,
    client_ip          TEXT,
    user_agent         TEXT,
    error_message      TEXT,
    request_body       TEXT,
    response_body      TEXT
);
CREATE INDEX idx_logs_ts        ON request_logs(request_ts DESC);
CREATE INDEX idx_logs_key_ts    ON request_logs(gateway_key_id, request_ts DESC);
CREATE INDEX idx_logs_status_ts ON request_logs(status, request_ts DESC);
CREATE INDEX idx_logs_provider_model_ts ON request_logs(provider, model, request_ts DESC);

-- Pre-aggregated request logs (hourly)
CREATE TABLE request_logs_hourly (
    project_id           TEXT NOT NULL,
    gateway_key_id       TEXT NOT NULL DEFAULT '',
    provider             TEXT NOT NULL DEFAULT '',
    model                TEXT NOT NULL DEFAULT '',
    hour                 INTEGER NOT NULL,
    requests             INTEGER NOT NULL,
    prompt_tokens        INTEGER NOT NULL,
    completion_tokens    INTEGER NOT NULL,
    cost_usd             REAL NOT NULL,
    cached_savings_usd   REAL NOT NULL,
    PRIMARY KEY (project_id, gateway_key_id, provider, model, hour)
);

-- Budgets
CREATE TABLE budgets (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id   TEXT NOT NULL,
    period      TEXT NOT NULL,
    amount_usd  REAL NOT NULL,
    thresholds  TEXT NOT NULL,
    status      TEXT NOT NULL
);
CREATE TABLE budget_usage (
    budget_id     TEXT NOT NULL,
    period_start  INTEGER NOT NULL,
    used_usd      REAL NOT NULL,
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY (budget_id, period_start)
);

-- KV cache (used by Lite profile)
CREATE TABLE kv_cache (
    key                TEXT PRIMARY KEY,
    value              BLOB NOT NULL,
    expires_at         INTEGER NOT NULL,
    size_bytes         INTEGER NOT NULL,
    hit_count          INTEGER NOT NULL DEFAULT 0,
    last_accessed_at   INTEGER NOT NULL,
    created_at         INTEGER NOT NULL
);
CREATE INDEX idx_kv_expires ON kv_cache(expires_at);

-- Admin users
CREATE TABLE admin_users (
    id             TEXT PRIMARY KEY,
    username       TEXT UNIQUE NOT NULL,
    password_hash  TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    last_login_at  INTEGER
);
