-- Distinguish admin-created keys from keys seeded via the YAML config.
-- Config-origin keys are managed declaratively (added/removed by reloading
-- the config file) and cannot be revoked through the Admin API.
ALTER TABLE gateway_keys ADD COLUMN origin TEXT NOT NULL DEFAULT 'admin';
