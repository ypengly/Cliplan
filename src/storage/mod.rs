pub mod database;
pub mod models;

use rusqlite::Connection;
use std::sync::{Arc, Mutex};

/// Shared handle to the sqlite connection. sqlite access here is
/// synchronous and fast (small metadata rows, WAL mode); the mutex is only
/// ever held for the duration of a single statement, so it does not become
/// a bottleneck for a LAN app serving a handful of devices.
pub type Db = Arc<Mutex<Connection>>;
