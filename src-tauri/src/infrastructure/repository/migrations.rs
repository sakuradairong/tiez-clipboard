use rusqlite::{Connection, Result};

pub fn run_migrations(connection: &Connection) -> Result<()> {
    tiez_core::database_migrations::run_migrations_with_decrypt(
        connection,
        crate::infrastructure::encryption::decrypt_value,
    )
}
