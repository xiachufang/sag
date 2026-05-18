-- Provider credentials are now declared inline (or via env://) in the
-- YAML config; the encrypted-at-rest path has been removed.
DROP TABLE IF EXISTS provider_credentials;
