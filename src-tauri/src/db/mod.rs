use postgresql_embedded::{PostgreSQL, Settings};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::path::Path;

const DB_NAME: &str = "deadlock_mm";

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("embedded postgres error: {0}")]
    Embedded(String),
    #[error("database error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Local PostgreSQL started by the app itself (no external server, no admin rights).
/// Binaries are bundled at build time, so first start needs no internet.
pub struct Db {
    pub pool: PgPool,
    pg: PostgreSQL,
}

impl Db {
    pub async fn start(app_dir: &Path) -> Result<Self, DbError> {
        let pg_dir = app_dir.join("postgres");
        std::fs::create_dir_all(&pg_dir)?;

        // Stable password across runs, stored next to the data (local-only listener).
        let pw_file = pg_dir.join("password");
        let password = match std::fs::read_to_string(&pw_file) {
            Ok(p) if !p.trim().is_empty() => p.trim().to_string(),
            _ => {
                let p = hex::encode(rand_bytes());
                std::fs::write(&pw_file, &p)?;
                p
            }
        };

        let settings = Settings {
            installation_dir: pg_dir.join("bin"),
            data_dir: pg_dir.join("data"),
            password_file: pg_dir.join("pgpass"),
            host: "127.0.0.1".into(),
            port: 0, // pick a free port on every start
            password,
            temporary: false,
            timeout: Some(std::time::Duration::from_secs(120)), // initdb is slow on Windows with antivirus
            ..Settings::default()
        };

        let mut pg = PostgreSQL::new(settings);
        pg.setup().await.map_err(|e| DbError::Embedded(e.to_string()))?;
        pg.start().await.map_err(|e| DbError::Embedded(e.to_string()))?;
        if !pg.database_exists(DB_NAME).await.map_err(|e| DbError::Embedded(e.to_string()))? {
            pg.create_database(DB_NAME).await.map_err(|e| DbError::Embedded(e.to_string()))?;
        }

        let pool = PgPoolOptions::new().max_connections(5).connect(&pg.settings().url(DB_NAME)).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool, pg })
    }

    /// Stop the server without consuming the handle (used on app exit).
    pub async fn shutdown(&self) -> Result<(), DbError> {
        self.pool.close().await;
        self.pg.stop().await.map_err(|e| DbError::Embedded(e.to_string()))
    }

    pub async fn stop(self) -> Result<(), DbError> {
        self.pool.close().await;
        self.pg.stop().await.map_err(|e| DbError::Embedded(e.to_string()))
    }
}

fn rand_bytes() -> [u8; 16] {
    // Not cryptographic secrecy against local users; the server only listens on 127.0.0.1.
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let mut out = [0u8; 16];
    out.copy_from_slice(&(n ^ (std::process::id() as u128) << 64).to_le_bytes());
    out
}
