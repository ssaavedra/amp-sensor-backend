-- Add up migration script here
CREATE INDEX energy_log_token_idx ON energy_log (token);
