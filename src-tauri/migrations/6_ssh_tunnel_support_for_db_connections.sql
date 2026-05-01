-- NULL = no tunnel = legacy behaviour. We don't add a FK constraint because
-- SQLite ALTER TABLE can't add one cleanly; referential integrity is enforced
-- at the application layer (the runtime treats a missing profile as a
-- connect-time error).
ALTER TABLE sql_connections   ADD COLUMN ssh_profile_id TEXT;
ALTER TABLE nosql_connections ADD COLUMN ssh_profile_id TEXT;
