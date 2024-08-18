-- Modify the energy_log table to use REAL fields for amps, volts, and watts as required by sqlite3
ALTER TABLE energy_log RENAME TO energy_log_old;
CREATE TABLE energy_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token TEXT NOT NULL,
    amps REAL NOT NULL,
    volts REAL NOT NULL,
    watts REAL NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    user_agent TEXT,
    client_ip TEXT,
    FOREIGN KEY (token) REFERENCES tokens(token)
);

INSERT INTO energy_log (id, token, amps, volts, watts, created_at, user_agent, client_ip) SELECT id, token, amps, volts, watts, created_at, user_agent, client_ip FROM energy_log_old;
DROP TABLE energy_log_old;

-- Add index to created_at
CREATE INDEX energy_log_created_at ON energy_log (created_at);
CREATE INDEX energy_log_token_idx ON energy_log (token);

