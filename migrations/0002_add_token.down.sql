-- Add down migration script here

--- Drop foreign table constraint
ALTER TABLE energy_log RENAME TO energy_log_old;
CREATE TABLE energy_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token TEXT NOT NULL,
    amps TEXT NOT NULL,
    volts TEXT NOT NULL,
    watts TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
INSERT INTO energy_log (id, token, amps, volts, watts, created_at) SELECT id, token, amps, volts, watts, created_at FROM energy_log_old;
DROP TABLE energy_log_old;

DROP TABLE tokens;
DROP TABLE users;
