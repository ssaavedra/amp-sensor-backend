-- Remove index from created_at
DROP INDEX IF EXISTS energy_log_created_at;
DROP INDEX IF EXISTS energy_log_token_idx;

-- Undo the changes made in the up migration script
ALTER TABLE energy_log RENAME TO energy_log_old;
CREATE TABLE energy_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token TEXT NOT NULL,
    amps DECIMAL(5, 2) NOT NULL,
    volts DECIMAL(5, 2) NOT NULL,
    watts DECIMAL(5, 2) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    user_agent TEXT,
    client_ip TEXT
);

INSERT INTO energy_log (id, token, amps, volts, watts, created_at, user_agent, client_ip) SELECT id, token, amps, volts, watts, created_at, user_agent, client_ip FROM energy_log_old;
DROP TABLE energy_log_old;