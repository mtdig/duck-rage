# <img src="img/friendly_raging_duck.png" width="60" height="60" style="float: none;" /> duck-rage  

A DuckDB extension that reads passwords from an [age](https://age-encryption.org/)-encrypted JSON file and creates
DuckDB secrets for PostgreSQL or MySQL connections — so credentials never live in plaintext on disk or in SQL history.

This is based on the experimental Rust duckdb-template and is .. experimental...
There may be better solutions out there, but I was curious on how this would work with Rust.
Additionally, I also wanted to use something easy to work with from the command line, and the [rage](https://github.com/str4d/rage) tool seemed like a good idea at the time.


## How it works

1. Store your database passwords in a JSON file and encrypt it with a  [rage](https://github.com/str4d/rage) key pair.
2. Call `duck_rage(...)` from DuckDB — it decrypts the file, looks up the requested key, and runs
   `CREATE OR REPLACE SECRET duck_rage_<database>` in the current session.
3. DuckDB's `postgres_scanner` or `mysql_scanner` will automatically use that secret for subsequent connections.

## Setup

### 1. Generate an age key pair

```bash
rage-keygen -o ~/.config/duck-rage/identity.txt
# Public key: age1...
```

Keep `identity.txt` private. Use the printed public key to encrypt your secrets.

### 2. Create and encrypt a secrets file

```bash
echo '{"prod_password": "s3cr3t", "analytics_password": "hunter2"}' \
  | rage -r age1... -o secrets.age
```

## Building

### With Nix (recommended)

A `flake.nix` is provided that pins all dependencies, including the exact DuckDB
version (1.4.4), Rust stable toolchain, and `libstdc++.so.6` needed by the Python
test runner wheel. This is the easiest path on NixOS or any system with Nix installed.

```bash
nix develop          # enter the dev shell (first run downloads deps)
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

SELECT * FROM duck_rage(
    'postgres',                                    -- db_type: 'postgres' or 'mysql'
    'localhost',                                   -- host
    5432,                                          -- port
    'mydb',                                        -- database
    'myuser',                                      -- user
    '/path/to/secrets.age',                        -- age-encrypted JSON secrets file
    'prod_password',                               -- JSON key whose value is the password
    '/home/you/.config/duck-rage/identity.txt'     -- age identity file (private key)
);
```

```
┌────────────────────────────────────────────────────────────────┐
│                       status                                   │
│                       varchar                                  │
├────────────────────────────────────────────────────────────────┤
│ Secret 'duck_rage_mydb' created for myuser@localhost:5432/mydb │
└────────────────────────────────────────────────────────────────┘
```

Then use the secret transparently:

```sql
INSTALL postgres_scanner;
LOAD postgres_scanner;
SELECT * FROM postgres_scan('', 'public', 'my_table');
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
