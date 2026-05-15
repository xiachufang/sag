-- Projects
CREATE TABLE projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  BIGINT NOT NULL
);

CREATE TABLE gateway_keys (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    prefix        TEXT NOT NULL,
    hash          BYTEA NOT NULL,
    last4         TEXT NOT NULL,
    scopes        TEXT NOT NULL,
    status        TEXT NOT NULL,
    expires_at    BIGINT,
    last_used_at  BIGINT,
    created_at    BIGINT NOT NULL,
    revoked_at    BIGINT
);
CREATE INDEX idx_keys_hash    ON gateway_keys(hash);
CREATE INDEX idx_keys_project ON gateway_keys(project_id);

CREATE TABLE provider_credentials (
    id             TEXT PRIMARY KEY,
    project_id     TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    provider       TEXT NOT NULL,
    name           TEXT NOT NULL,
    encrypted_key  BYTEA NOT NULL,
    status         TEXT NOT NULL,
    created_at     BIGINT NOT NULL
);
CREATE INDEX idx_provider_credentials_project ON provider_credentials(project_id, provider);

CREATE TABLE routes (
    project_id  TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    config      TEXT NOT NULL,
    version     BIGINT NOT NULL,
    updated_at  BIGINT NOT NULL
);

CREATE TABLE pricing (
    provider             TEXT NOT NULL,
    model                TEXT NOT NULL,
    input_per_1k         DOUBLE PRECISION NOT NULL,
    output_per_1k        DOUBLE PRECISION NOT NULL,
    cached_input_per_1k  DOUBLE PRECISION,
    effective_from       BIGINT NOT NULL,
    effective_to         BIGINT,
    PRIMARY KEY (provider, model, effective_from)
);

CREATE TABLE request_logs (
    id                 TEXT PRIMARY KEY,
    project_id         TEXT NOT NULL,
    gateway_key_id     TEXT,
    provider           TEXT,
    model              TEXT,
    endpoint           TEXT,
    request_ts         BIGINT NOT NULL,
    duration_ms        BIGINT,
    upstream_ms        BIGINT,
    ttfb_ms            BIGINT,
    status             TEXT NOT NULL,
    http_status        INTEGER,
    cached             BOOLEAN NOT NULL DEFAULT FALSE,
    retry_count        INTEGER NOT NULL DEFAULT 0,
    fallback_used      TEXT,
    prompt_tokens      BIGINT,
    completion_tokens  BIGINT,
    cached_tokens      BIGINT,
    total_tokens       BIGINT,
    cost_usd           DOUBLE PRECISION,
    would_have_cost_usd DOUBLE PRECISION,
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

CREATE TABLE request_logs_hourly (
    project_id           TEXT NOT NULL,
    gateway_key_id       TEXT NOT NULL DEFAULT '',
    provider             TEXT NOT NULL DEFAULT '',
    model                TEXT NOT NULL DEFAULT '',
    hour                 BIGINT NOT NULL,
    requests             BIGINT NOT NULL,
    prompt_tokens        BIGINT NOT NULL,
    completion_tokens    BIGINT NOT NULL,
    cost_usd             DOUBLE PRECISION NOT NULL,
    cached_savings_usd   DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (project_id, gateway_key_id, provider, model, hour)
);

CREATE TABLE budgets (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id   TEXT NOT NULL,
    period      TEXT NOT NULL,
    amount_usd  DOUBLE PRECISION NOT NULL,
    thresholds  TEXT NOT NULL,
    status      TEXT NOT NULL
);
CREATE TABLE budget_usage (
    budget_id     TEXT NOT NULL,
    period_start  BIGINT NOT NULL,
    used_usd      DOUBLE PRECISION NOT NULL,
    updated_at    BIGINT NOT NULL,
    PRIMARY KEY (budget_id, period_start)
);

CREATE TABLE admin_users (
    id             TEXT PRIMARY KEY,
    username       TEXT UNIQUE NOT NULL,
    password_hash  TEXT NOT NULL,
    created_at     BIGINT NOT NULL,
    last_login_at  BIGINT
);
