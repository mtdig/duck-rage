use duckdb::{
    core::{DataChunkHandle, Inserter, LogicalTypeHandle, LogicalTypeId},
    duckdb_entrypoint_c_api,
    vtab::{BindInfo, InitInfo, TableFunctionInfo, VTab},
    Connection, Result,
};
use std::{
    error::Error,
    ffi::CString,
    io::Read,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
};

static SIBLING_CONN: OnceLock<Mutex<Connection>> = OnceLock::new();

// ---------------------------------------------------------------------------
// DbProvider trait — one impl per supported database type
// ---------------------------------------------------------------------------

/// Everything that differs between database backends.
trait DbProvider: Send + Sync + 'static {
    /// The DuckDB secret `TYPE` keyword (e.g. `postgres`, `mysql`).
    fn secret_type(&self) -> &'static str;

    /// Build the full `CREATE OR REPLACE SECRET` SQL.
    /// The secret is named `duck_rage_<database>`.
    fn create_secret_sql(&self, host: &str, port: i32, database: &str, user: &str, password: &str) -> String {
        format!(
            "CREATE OR REPLACE SECRET duck_rage_{database} ( \
                TYPE {typ}, \
                HOST '{host}', \
                PORT {port}, \
                DATABASE '{database}', \
                USER '{user}', \
                PASSWORD '{password}' \
            )",
            database = escape_sql_string(database),
            typ      = self.secret_type(),
            host     = escape_sql_string(host),
            port     = port,
            user     = escape_sql_string(user),
            password = escape_sql_string(password),
        )
    }
}

// ---------------------------------------------------------------------------
// Concrete backends
// ---------------------------------------------------------------------------

struct PostgresProvider;
struct MySqlProvider;

impl DbProvider for PostgresProvider {
    fn secret_type(&self) -> &'static str { "postgres" }
}

impl DbProvider for MySqlProvider {
    fn secret_type(&self) -> &'static str { "mysql" }
}

// ---------------------------------------------------------------------------
// DbType — parsed from the SQL parameter
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum DbType {
    Postgres,
    Mysql,
}

impl DbType {
    fn provider(self) -> Box<dyn DbProvider> {
        match self {
            DbType::Postgres => Box::new(PostgresProvider),
            DbType::Mysql    => Box::new(MySqlProvider),
        }
    }
}

impl FromStr for DbType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Ok(DbType::Postgres),
            "mysql"                   => Ok(DbType::Mysql),
            other => Err(format!(
                "Unknown db_type '{}'. Supported: postgres, mysql",
                other
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Bind / Init data
// ---------------------------------------------------------------------------

#[repr(C)]
struct RageBindData {
    host:             String,
    port:             i32,
    database:         String,
    user:             String,
    /// The CREATE SECRET SQL is built at bind time so the password never
    /// leaves `decrypt_age_file` unnecessarily.
    create_secret_sql: String,
}

#[repr(C)]
struct RageInitData {
    done: AtomicBool,
}

// ---------------------------------------------------------------------------
// Table function implementation
// ---------------------------------------------------------------------------

struct RageVTab;

impl VTab for RageVTab {
    type InitData = RageInitData;
    type BindData = RageBindData;

    fn bind(bind: &BindInfo) -> Result<Self::BindData, Box<dyn std::error::Error>> {
        bind.add_result_column("status", LogicalTypeHandle::from(LogicalTypeId::Varchar));

        // All parameters are positional and required:
        //   db_type, host, port, database, user, secrets_file, secret_key, identity_file
        const USAGE: &str = "Usage: duck_rage(\n  db_type      VARCHAR  -- 'postgres' or 'mysql'\n  host         VARCHAR  -- hostname or IP\n  port         INTEGER  -- e.g. 5432\n  database     VARCHAR  -- database name\n  user         VARCHAR  -- login user\n  secrets_file VARCHAR  -- path to age-encrypted JSON file\n  secret_key   VARCHAR  -- JSON key whose value is the password\n  identity_file VARCHAR -- path to age identity file (rage-keygen output)\n)";

        let db_type: DbType = bind.get_parameter(0).to_string().parse()
            .map_err(|e| format!("{e}\n\n{USAGE}"))?;
        let host          = bind.get_parameter(1).to_string();
        let port: i32     = bind.get_parameter(2).to_string().parse()
            .map_err(|_| format!("Invalid port '{}': must be an integer\n\n{USAGE}", bind.get_parameter(2)))?;
        let database      = bind.get_parameter(3).to_string();
        let user          = bind.get_parameter(4).to_string();
        let secrets_file  = bind.get_parameter(5).to_string();
        let secret_key    = bind.get_parameter(6).to_string();
        let identity_file = bind.get_parameter(7).to_string();

        let provider = db_type.provider();

        let password = decrypt_age_file(&secrets_file, &secret_key, &identity_file)
            .map_err(|e| format!("{e}\n\n{USAGE}"))?;
        let create_secret_sql =
            provider.create_secret_sql(&host, port, &database, &user, &password);

        Ok(RageBindData {
            host,
            port,
            database,
            user,
            create_secret_sql,
        })
    }

    fn init(_: &InitInfo) -> Result<Self::InitData, Box<dyn std::error::Error>> {
        Ok(RageInitData {
            done: AtomicBool::new(false),
        })
    }

    fn func(
        func: &TableFunctionInfo<Self>,
        output: &mut DataChunkHandle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let init_data = func.get_init_data();
        let bind_data = func.get_bind_data();

        if init_data.done.swap(true, Ordering::Relaxed) {
            output.set_len(0);
            return Ok(());
        }

        execute_sql_on_current_db(&bind_data.create_secret_sql)?;

        let msg = CString::new(format!(
            "Secret 'duck_rage_{}' created for {}@{}:{}/{}",
            bind_data.database,
            bind_data.user, bind_data.host, bind_data.port, bind_data.database,
        ))?;
        output.flat_vector(0).insert(0, msg);
        output.set_len(1);
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![
            LogicalTypeHandle::from(LogicalTypeId::Varchar), // db_type       (postgres|mysql)
            LogicalTypeHandle::from(LogicalTypeId::Varchar), // host
            LogicalTypeHandle::from(LogicalTypeId::Integer), // port
            LogicalTypeHandle::from(LogicalTypeId::Varchar), // database
            LogicalTypeHandle::from(LogicalTypeId::Varchar), // user
            LogicalTypeHandle::from(LogicalTypeId::Varchar), // secrets_file  (path to .age file)
            LogicalTypeHandle::from(LogicalTypeId::Varchar), // secret_key    (JSON key)
            LogicalTypeHandle::from(LogicalTypeId::Varchar), // identity_file (path to age key)
        ])
    }
}

// ---------------------------------------------------------------------------
// Execute SQL on the current in-process database
// ---------------------------------------------------------------------------

fn execute_sql_on_current_db(sql: &str) -> std::result::Result<(), Box<dyn Error>> {
    let conn = SIBLING_CONN
        .get()
        .ok_or("duck_rage: connection not initialised")?;
    conn.lock().unwrap().execute_batch(sql)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Age decryption helper
// ---------------------------------------------------------------------------

/// Decrypts an age file using an X25519 identity file, parses the contents
/// as JSON, and returns the string value for `key`.
///
/// Generate a key pair with:
///   `rage-keygen -o ~/.config/duck-rage/identity.txt`
///
/// Encrypt your secrets with the public key:
///   `echo '{"db_password": "hunter2"}' | rage -r age1... -o secrets.age`
fn decrypt_age_file(
    path: &str,
    key: &str,
    identity_file: &str,
) -> std::result::Result<String, Box<dyn Error>> {
    let ciphertext = std::fs::read(path)
        .map_err(|e| format!("Cannot read secrets file '{}': {}", path, e))?;

    let identity_contents = std::fs::read_to_string(identity_file)
        .map_err(|e| format!("Cannot read identity file '{}': {}", identity_file, e))?;

    let identities = age::IdentityFile::from_buffer(identity_contents.as_bytes())
        .map_err(|e| format!("Failed to parse identity file '{}': {}", identity_file, e))?
        .into_identities()
        .map_err(|e| format!("Failed to load identities from '{}': {}", identity_file, e))?;

    let decryptor = age::Decryptor::new_buffered(ciphertext.as_slice())
        .map_err(|e| format!("Failed to parse age file '{}': {}", path, e))?;

    let mut reader = decryptor
        .decrypt(identities.iter().map(|i| i.as_ref() as &dyn age::Identity))
        .map_err(|e| format!("Failed to decrypt '{}' with identity '{}': {}", path, identity_file, e))?;

    let mut contents = String::new();
    reader
        .read_to_string(&mut contents)
        .map_err(|e| format!("Failed to read decrypted content: {}", e))?;

    let map: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(contents.trim())
            .map_err(|e| format!("secrets file is not valid JSON: {}", e))?;

    match map.get(key) {
        Some(serde_json::Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(format!(
            "Key '{}' in secrets file is not a JSON string (got: {})",
            key, other
        )
        .into()),
        None => Err(format!("Key '{}' not found in secrets file", key).into()),
    }
}

// ---------------------------------------------------------------------------
// SQL string escaping
// ---------------------------------------------------------------------------

/// Escapes single-quotes for safe embedding in SQL string literals.
fn escape_sql_string(s: &str) -> String {
    s.replace('\'', "''")
}

// ---------------------------------------------------------------------------
// Extension entrypoint
// ---------------------------------------------------------------------------

const EXTENSION_NAME: &str = env!("CARGO_PKG_NAME");

#[duckdb_entrypoint_c_api()]
pub unsafe fn extension_entrypoint(con: Connection) -> Result<(), Box<dyn Error>> {
    let sibling = con.try_clone()?;
    let _ = SIBLING_CONN.set(Mutex::new(sibling));

    con.register_table_function::<RageVTab>(EXTENSION_NAME)
        .expect("Failed to register duck_rage table function");
    Ok(())
}
