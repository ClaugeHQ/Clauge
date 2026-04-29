// Cross-platform infrastructure shared across modes.
// `credential_store` is the OS-keyring abstraction (macOS Keychain today;
// Windows / Linux backends plug in here in later phases).

pub mod credential_store;
