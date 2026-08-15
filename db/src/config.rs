use std::sync::OnceLock;

fn db_env_enable(name: &str) -> bool {
    std::env::var(name).map_or(false, |v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub(crate) fn db_sync_enabled() -> bool {
    static DB_SYNC: OnceLock<bool> = OnceLock::new();
    *DB_SYNC.get_or_init(|| db_env_enable("HACASH_DB_SYNC"))
}

#[allow(dead_code)]
pub(crate) fn db_sled_small_machine_enabled() -> bool {
    static DB_SLED_SMALL_MACHINE: OnceLock<bool> = OnceLock::new();
    *DB_SLED_SMALL_MACHINE.get_or_init(|| db_env_enable("HACASH_DB_SMALL_MACHINE"))
}

/*
HACASH_DB_SMALL_MACHINE=1 HACASH_DB_SYNC=1
*/
