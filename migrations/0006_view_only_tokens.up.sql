-- Add up migration script here

CREATE TABLE view_tokens (
    id SERIAL PRIMARY KEY,
    token TEXT NOT NULL,
    user_id INT NOT NULL,
    view_token_valid_until TIMESTAMP NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_accessed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    FOREIGN KEY (user_id) REFERENCES users (id)
);

CREATE INDEX IF NOT EXISTS idx_view_tokens_token ON view_tokens (token);

-- Insert all existing tokens also as view tokens
INSERT INTO view_tokens (token, user_id, view_token_valid_until)
SELECT token, user_id, datetime('now', '+60 years') as view_token_valid_until
FROM tokens;
