# <img src="img/friendly_raging_duck.png" width="150" height="150" style="float: none;" /> duck-rage  

A DuckDB extension that reads passwords from an [age](https://age-encryption.org/)-encrypted JSON file and creates
DuckDB secrets (connection string with user, host, port, password, db) for PostgreSQL or MySQL connections — so credentials never live in plaintext on disk or in SQL history.

This is based on the experimental [Rust duckdb-template](https://github.com/duckdb/extension-template-rs/) and is .. experimental...

There may be better solutions out there, but I was curious on how this would work with Rust.
Honestly, I was expecting a lot more friction between C++ and Rust, but with the great example and simplified template, it was definitely a much easier experience than my first encounters with writing duckdb extensions.

Additionally, I also wanted to use something easy to work with from the command line, and the [rage](https://github.com/str4d/rage) tool seemed like a good idea at the time.

Refer to the excellent DuckDB documentation for the [postgres](https://duckdb.org/docs/stable/core_extensions/postgres) and [mysql](https://duckdb.org/docs/stable/core_extensions/mysql) extensions.


## How it works

1. Store your database passwords in a JSON file and encrypt it with a  [rage](https://github.com/str4d/rage) key pair.
2. Call `duck_rage(...)` from DuckDB — it decrypts the file, looks up the requested key, and runs
   `CREATE OR REPLACE SECRET duck_rage_<database>` in the current session.  The secret will be named 'duck_rage_<DBNAME>'.
3. DuckDB's `postgres_scanner` or `mysql_scanner` can use secrets automatically, but in this case, it's a named secret, so just pass the secret name into the ATTACH statement.


## Setup

### 1. Generate an age key pair

```bash
rage-keygen -o ~/.config/duck-rage/identity.txt
# Public key: age1...
```

Keep `identity.txt` private. Use the printed public key to encrypt your secrets.

### 2. Create and encrypt a secrets file

```bash
echo '{"appuser": "s3cr3t", "admin": "Xq7#mK2$vL9@nR4!"}' \
  | rage -r age1... -o secrets.age
```

## Building

### With Nix (recommended)

A `flake.nix` is provided that pins all dependencies, including the exact DuckDB
version (1.4.4), Rust stable toolchain, and `libstdc++.so.6` needed by the Python
test runner wheel. This is the easiest path on NixOS or any system with Nix installed.

```bash
nix develop          # enter the dev shell (first run will download deps)
make configure       # sets up Python venv with DuckDB test runner
make debug           # builds the extension
```

The dev shell provides:
- Rust stable toolchain
- Python 3 with pip (the Makefile pip-installs `duckdb==1.4.4` into a venv)
- DuckDB 1.4.4 CLI
- gcc / `libstdc++.so.6` (required by the pip DuckDB wheel)
- make, pkg-config, openssl, git, rage

Every subsequent session just needs `nix develop` before running make targets.

### Without Nix

Install the following manually:

| Dependency | Notes |
|---|---|
| [Rust](https://rustup.rs) stable | `rustup install stable` |
| Python 3 + venv | distro package or [python.org](https://python.org) |
| Make | pre-installed on most Linux/macOS |
| Git | pre-installed on most systems |
| `libstdc++` | usually part of `gcc` / `g++` / `libstdc++-dev` |
| [rage](https://github.com/str4d/rage) | `cargo install rage` or distro package |

On Ubuntu/Debian:
```bash
sudo apt install build-essential python3 python3-venv git pkg-config libssl-dev
cargo install rage
```

On macOS with Homebrew:
```bash
brew install rust python make openssl rage
```

Then build:
```bash
make configure
make debug
```

> **Note**: The `make configure` step pip-installs `duckdb==1.4.4` into a local
> venv. If the pip wheel fails to load with `ImportError: libstdc++.so.6`, make
> sure `libstdc++` is on your `LD_LIBRARY_PATH` — or use the Nix path above.

### Output

The extension is written to:
```
build/debug/extension/duck_rage/duck_rage.duckdb_extension
```

For an optimized release build, run `make release` instead of `make debug`.

## Running the extension

Start DuckDB with `-unsigned` to allow loading local extensions:

```bash
duckdb -unsigned
```

Load the extension and call `duck_rage`:

```sql
LOAD './build/debug/extension/duck_rage/duck_rage.duckdb_extension';

-- Minimal usage: relies on default paths or environment variables
SELECT * FROM duck_rage(
    'postgres',                                    -- db_type: 'postgres' or 'mysql'
    'localhost',                                   -- host
    5432,                                          -- port
    'mydb',                                        -- database
    'myuser',                                      -- user
    'appuser'                                -- JSON key whose value is the password we're looking for
);

-- With explicit secrets_file and identity_file (named parameters)
SELECT * FROM duck_rage(
    'postgres',
    'localhost',
    5432,
    'mydb',
    'myuser',
    'appuser',
    secrets_file => '/path/to/secrets.age',
    identity_file => '/home/you/.config/duck-rage/identity.txt'
);
```

### File Resolution

Both `secrets_file` and `identity_file` are optional named parameters with automatic fallbacks:

#### Secrets File Resolution
1. **Named parameter** `secrets_file` (if provided)
2. **Environment variable** `RAGE_SECRETS_FILE`
3. **Default path** `~/.config/duck-rage/secrets.age`

#### Identity File Resolution
1. **Named parameter** `identity_file` (if provided)
2. **Environment variable** `RAGE_IDENTITY_FILE`
3. **Default path** `~/.config/duck-rage/identity.txt`

Example using environment variables:

```bash
export RAGE_SECRETS_FILE=~/.config/duck-rage/secrets.age
export RAGE_IDENTITY_FILE=~/.config/duck-rage/identity.txt
duckdb -unsigned
```

Then in DuckDB, you only need to specify the connection details and secret key:

```sql
SELECT * FROM duck_rage('mysql', '192.168.56.20', 3306, 'appdb', 'appuser', 'appuser');
┌────────────────────────────────────────────────────────────────┐
│                       status                                   │
│                       varchar                                  │
├────────────────────────────────────────────────────────────────┤
│ Secret 'duck_rage_mydb' created for myuser@localhost:5432/mydb │
└────────────────────────────────────────────────────────────────┘
```

Then use the secret to attach the db:


```sql
INSTALL mysql;
LOAD mysql;
ATTACH '' AS appdb (TYPE mysql, SECRET duck_rage_appdb);
```


## Testing

Tests are written in SQLLogicTest format in `test/sql/duck_rage.test`.
A self-contained test key pair and secrets file are committed under `test/data/`
so no external setup is needed to run the tests.

```bashell
make test_debug
```

On NixOS / with Nix:

```bashell
nix develop --command make test_debug
```

For the release build:

```bashell
make test_release
```

## Dependencies

- Python3 + venv
- [Make](https://www.gnu.org/software/make)
- Git
- [rage](https://github.com/str4d/rage) (`rage-keygen`, `rage`) for key generation and encryption
