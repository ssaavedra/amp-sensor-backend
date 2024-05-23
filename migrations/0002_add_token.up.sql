-- Add up migration script here
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    location VARCHAR(255) NOT NULL
);

CREATE TABLE tokens (
    token VARCHAR(255) PRIMARY KEY NOT NULL,
    user_id INT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id)
);

INSERT INTO users (location) VALUES ('default');

-- Add all tokens that already exist as tokens for the default location
INSERT INTO tokens (token, user_id) SELECT token, 1 FROM energy_log;

-- Make the token column in energy_log a foreign key to the tokens table
ALTER TABLE energy_log RENAME TO energy_log_old;
CREATE TABLE energy_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token VARCHAR(255) NOT NULL,
    amps TEXT NOT NULL,
    volts TEXT NOT NULL,
    watts TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (token) REFERENCES tokens(token)
);
INSERT INTO energy_log (id, token, amps, volts, watts, created_at) SELECT id, token, amps, volts, watts, created_at FROM energy_log_old;
DROP TABLE energy_log_old;
