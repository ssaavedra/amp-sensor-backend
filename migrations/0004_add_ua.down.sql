-- Add down migration script here

-- Remove User-Agent and IP columns from energy_log table
ALTER TABLE energy_log DROP COLUMN user_agent;
ALTER TABLE energy_log DROP COLUMN client_ip;
