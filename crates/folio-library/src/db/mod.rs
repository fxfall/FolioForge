mod migration;

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::error::LibraryError;

pub use migration::CURRENT_SCHEMA_VERSION;

pub struct LibraryDb {
    connection: Connection,
}

impl LibraryDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LibraryError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI,
        )?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self, LibraryError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(mut connection: Connection) -> Result<Self, LibraryError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        migration::apply(&mut connection)?;
        Ok(Self { connection })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }

    pub fn schema_version(&self) -> Result<i64, LibraryError> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(LibraryError::from)
    }
}
