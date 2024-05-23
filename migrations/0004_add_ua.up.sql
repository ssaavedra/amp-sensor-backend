-- Add up migration script here
-- Add User-Agent and IP columns to energy_log table
ALTER TABLE energy_log ADD COLUMN user_agent TEXT;
ALTER TABLE energy_log ADD COLUMN client_ip TEXT;

