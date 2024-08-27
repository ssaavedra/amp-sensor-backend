-- Add down migration script here

DROP INDEX IF EXISTS idx_view_tokens_token;
DROP TABLE view_tokens;
