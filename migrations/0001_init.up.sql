-- Add up migration script here
CREATE TABLE energy_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token TEXT NOT NULL,
    amps DECIMAL(5, 2) NOT NULL,
    volts DECIMAL(5, 2) NOT NULL,
    watts DECIMAL(5, 2) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
