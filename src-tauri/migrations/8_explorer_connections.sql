-- Explorer mode — remote file system connections (SFTP / FTP / S3 / Azure Blob).
-- One profile per remote endpoint. Kind-discriminated columns; secrets live in
-- the per-OS keychain (CredentialStore service "Clauge Explorer"), never here.
CREATE TABLE explorer_connections (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL CHECK(kind IN ('sftp','ftp','s3','azure_blob')),
    accent_color    TEXT,
    last_used_at    TEXT,
    created_at      TEXT NOT NULL,

    -- SFTP (kind='sftp') — preferred path: reuse an existing ssh_profiles row.
    ssh_profile_id  TEXT REFERENCES ssh_profiles(id) ON DELETE SET NULL,
    sftp_working_dir TEXT,

    -- Direct host fields. Used for FTP, and for SFTP when ssh_profile_id is null.
    host            TEXT,
    port            INTEGER,
    username        TEXT,
    auth_type       TEXT,
    key_path        TEXT,

    -- FTP-specific.
    ftp_passive     INTEGER NOT NULL DEFAULT 1,
    ftp_tls         TEXT CHECK(ftp_tls IN ('none','explicit','implicit')) DEFAULT 'none',

    -- S3-specific.
    s3_preset       TEXT,
    s3_endpoint     TEXT,
    s3_region       TEXT,
    s3_bucket       TEXT,
    s3_path_style   INTEGER NOT NULL DEFAULT 0,

    -- Azure Blob-specific.
    azure_account   TEXT,
    azure_container TEXT,
    azure_auth_kind TEXT CHECK(azure_auth_kind IN ('shared_key','sas','connection_string'))
);

CREATE INDEX idx_explorer_connections_kind ON explorer_connections(kind);
CREATE INDEX idx_explorer_connections_last_used ON explorer_connections(last_used_at DESC);
