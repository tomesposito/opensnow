# Pinned engine stack

Slice 1.1, issue #4. Checked locally on Nov 6, 2026.

Use Candidate B: DataFusion 54.1.0 with Iceberg 0.11.0. The shipped
provider and both harness paths share one dependency tree. Iceberg 0.11.0
is now a vendored path dependency under Tom's Nov 7 ruling for issue #7.
Every other external source in Cargo.lock stays on crates.io. There are no
git sources, version bumps, or DuckDB dependencies. This applies D-01's
pinned fork and small patch set rule.

## Pins and direct dependency licenses

All versions below are exact workspace pins. Cargo.lock pins the rest.

| Crate | Version | Source | License |
| --- | --- | --- | --- |
| datafusion | 54.1.0 | crates.io | Apache-2.0 |
| arrow | 58.4.1 | crates.io | Apache-2.0 |
| iceberg | 0.11.0 | vendor/iceberg-0.11.0 | Apache-2.0 |
| iceberg-datafusion | 0.11.0 | crates.io | Apache-2.0 |
| iceberg-catalog-sql | 0.11.0 | crates.io | Apache-2.0 |
| tokio | 1.54.1 | crates.io | MIT |
| futures | 0.3.34 | crates.io | MIT OR Apache-2.0 |
| tempfile | 3.27.0 | crates.io | MIT OR Apache-2.0 |
| sqlx | 0.8.6 | crates.io | MIT OR Apache-2.0 |
| uuid | 1.29.0 | crates.io | MIT OR Apache-2.0 |
| sha2 | 0.10.9 | crates.io | MIT OR Apache-2.0 |
| hmac | 0.12.1 | crates.io | MIT OR Apache-2.0 |
| rand | 0.8.8 | crates.io | MIT OR Apache-2.0 |
| hex | 0.4.3 | crates.io | MIT OR Apache-2.0 |
| subtle | 2.6.1 | crates.io | BSD-3-Clause |
| sqlparser | 0.62.0 | crates.io | Apache-2.0 |
| serde | 1.0.229 | crates.io | MIT OR Apache-2.0 |
| serde_json | 1.0.154 | crates.io | MIT OR Apache-2.0 |

The opensnow crates use Apache-2.0. Serde and serde_json were promoted
from transitive to direct dependencies in slice 1.9, issue #12, at the
versions the lock already held. No external package was added; the
lock count rose by one only because opensnow-mcp is a new workspace
crate (479 entries, of which 5 are workspace crates). SQLx explicitly enables
runtime-tokio, sqlite, postgres, and any, with default features off. Without the
runtime feature, the SQL catalog panics at connection time.

## Candidate A

Tried first:

```sh
timeout 90 git ls-remote https://github.com/apache/datafusion-iceberg.git HEAD
```

It failed in under one second, exit 128:

```text
fatal: unable to access 'https://github.com/apache/datafusion-iceberg.git/': Failed to connect to hatch-egress-proxy port 3128 after 1 ms: Couldn't connect to server
```

No revision could be obtained, so no exact git pin, candidate lockfile,
fetch, build, or API comparison could be tested. This is an access failure,
not evidence of an API defect. The attempt stayed within the 25-minute
setup budget. Candidate A does not meet the reproducibility requirement
on this VM. We did not substitute an unverified revision.

The upgrade signal is an Iceberg release whose transaction module exposes
a public overwrite or replace-files action. Return to registry Iceberg and
drop the patch when the combined harness and writer tests pass on that
release. Keep one DataFusion and Arrow version, restore registry sources
for all external packages, and repeat the license check. A version number
alone is not the trigger. A git dependency needs a separate written D-01
justification.

## Reproduce

Run from the repository root using the existing spike toolchain:

```sh
export CARGO_HOME="$PWD/spikes/B2-iceberg/cargo"
export RUSTUP_HOME="$PWD/spikes/B2-iceberg/rustup"
export CARGO_TARGET_DIR="$PWD/spikes/B2-iceberg/rs-spike/target"
export PATH="$CARGO_HOME/bin:$PATH"
cargo test --workspace --offline
cargo run -p stack-check --offline
cargo tree --offline -i datafusion
cargo tree --offline -i arrow
cargo metadata --offline --filter-platform x86_64-unknown-linux-gnu --format-version 1
```

Rust and Cargo are 1.99.1. No new toolchain was installed. The installed
spike toolchain is named stable, not 1.99.1, so there is no root
rust-toolchain.toml: a numbered override would ask rustup to download it.
CI selects 1.99.1 explicitly. The target directory reuses B2 artifacts;
test data never reads from spikes/. A clean checkout only needs Rust and
registry dependencies, then `cargo test --workspace` and
`cargo run -p stack-check`.

The B1 library ports the six SQL shapes without API changes from 55.2 to
54.1. It creates 24 sales and two products in memory. It checks result row
counts, then compares the net_price UDF aggregate to a plain Rust sum over
the generated rows. Sessions have their own 64 MiB GreedyMemoryPool and
two target partitions. This small fixture does not prove spill behavior.

The B2 library creates a fresh temporary SQLite database and local
warehouse. It registers IcebergCatalogProvider, creates the table through
SQL, appends 100 then 50 rows, and reads snapshot one through the Iceberg
scan API. Errors fail the binary or test. Temporary files are removed on
return. The combined_stack test runs both libraries sequentially in one
process, as does the binary.

Tree checks show exactly datafusion 54.1.0 and arrow 58.4.1. The provider,
engine, and storage all use those versions. Cargo.lock contains 477
packages including the three local crates, no git sources, and no DuckDB.

## Harness results

`cargo test --workspace --offline` passed: 1 combined test, 0 failures.
Both library test targets and both doc-test targets also completed with
0 failures. The warm dependency build took 5m 38s; the combined test took
0.68s. A final `cargo test --workspace --locked` also passed (1 test,
0 failures), followed by a green `cargo run -p stack-check`.
`cargo run -p stack-check --offline` exited 0 and printed:

```text
PASS B1 Q1: 1 rows
PASS B1 Q2: 6 rows
PASS B1 Q3: 2 rows
PASS B1 Q4: 10 rows
PASS B1 Q5: 12 rows
PASS B1 Q6: 2 rows
PASS B1 net_price: 1191 equals plain Rust 1191
PASS B1 path: six analytical shapes and independent revenue check
PASS B2 SQL catalog and CREATE TABLE
PASS B2 append: live count = 100
PASS B2 append: live count = 150
PASS B2 path: snapshot count = 100, live count = 150
```

## License scope

Direct dependency licenses above were checked from Cargo metadata and
cached source manifests for D-04. The Linux metadata contains 420 packages,
including local crates. No AGPL, Elastic License, Business Source License,
or other source-available license appeared in that scan.

The permissive licenses outside the Apache/MIT/BSD spellings are ISC
(untrusted), CC0-1.0 (tiny-keccak), Zlib (foldhash and zlib-rs), bzip2-1.0.6
(libbz2-rs-sys), and Unicode-3.0 (the ICU4X, yoke, zerovec, and related
Unicode support crates). ar_archive_writer uses Apache-2.0 with the LLVM
exception. ring also carries ISC terms and some Unicode
support crates combine Unicode-3.0 with MIT/Apache. These are permissive
terms; they need their notices preserved when distributing. BSL-1.0 in
an OR expression means Boost Software License, not Business Source
License; the Apache-2.0 option is available there.

The full lock also contains platform-specific or optional crates absent
from the local source cache. Their licenses are not verified here.
Unfiltered offline cargo metadata stops at android_system_properties
because it is not cached. These are the unresolved manifest checks:

android_system_properties 0.1.6, bumpalo 3.20.3, core-foundation-sys 0.8.7, etcetera 0.8.0, gloo-timers 0.3.0, iana-time-zone-haiku 0.1.2, jiff-static 0.2.39, jiff-tzdb 0.1.9, jiff-tzdb-platform 0.1.3, js-sys 0.3.107, libredox 0.1.25, plain 0.2.3, portable-atomic-util 0.2.8, quad-rand 0.2.3, r-efi 5.3.0, r-efi 6.0.0, redox_syscall 0.5.18, redox_syscall 0.9.5, rustversion 1.0.23, wasi 0.11.1+wasi-snapshot-preview1, wasip2 1.0.4+wasi-0.2.12, wasite 0.1.0, wasm-bindgen 0.2.130, wasm-bindgen-futures 0.4.80, wasm-bindgen-macro 0.2.130, wasm-bindgen-macro-support 0.2.130, wasm-bindgen-shared 0.2.130, web-sys 0.3.107, web-time 1.1.0, winapi-util 0.1.11, windows-core 0.62.2, windows-implement 0.60.2, windows-interface 0.59.3, windows-link 0.2.1, windows-result 0.4.1, windows-strings 0.5.1, windows-sys 0.48.0, windows-sys 0.52.0, windows-sys 0.61.2, windows-targets 0.48.5, windows-targets 0.52.6, windows_aarch64_gnullvm 0.48.5, windows_aarch64_gnullvm 0.52.6, windows_aarch64_msvc 0.48.5, windows_aarch64_msvc 0.52.6, windows_i686_gnu 0.48.5, windows_i686_gnu 0.52.6, windows_i686_gnullvm 0.52.6, windows_i686_msvc 0.48.5, windows_i686_msvc 0.52.6, windows_x86_64_gnu 0.48.5, windows_x86_64_gnu 0.52.6, windows_x86_64_gnullvm 0.48.5, windows_x86_64_gnullvm 0.52.6, windows_x86_64_msvc 0.48.5, windows_x86_64_msvc 0.52.6, wit-bindgen 0.57.1.

This license result covers the Linux build, not a verified all-platform
release audit. Recheck those manifests before enabling other platforms or
shipping a distribution.

## CI

The separate CI commit runs cargo test --workspace --locked on pull
requests and pushes to main with Rust 1.99.1. Swatinem/rust-cache@v2 caches
the registry and build dependencies, keyed on the toolchain and Cargo.lock.
This avoids paying the spikes' roughly two-hour cold build on every run.
The local harness was built with debug info off, as in B2. No fmt or clippy
gate was added: those components are absent from the installed toolchain.
CI itself has not been run locally.

## Slice 1.3 dependency update

SQLx keeps its exact 0.8.6 pin and adds the postgres driver feature.
UUID is now a direct exact 1.29.0 workspace pin with v7 enabled for fresh
clone identities. Its cached manifest declares Apache-2.0 OR MIT. SQLx
remains MIT OR Apache-2.0. No other direct dependency was added.

Slice 1.3 verification (Nov 7, 2026): `cargo test --workspace --offline`
passed 11 tests: 8 engine tests, 2 storage integration tests, and
`combined_stack`, with 0 failures and 0 ignored tests. The new tests are
`sqlite_round_trip_time_travel_and_clone` and
`postgres_round_trip_time_travel_and_clone`. The latter executed against
localhost:5433 and printed `PASS Postgres catalog round trip`. Doc-tests
also passed (0 tests). Localhost access requires running outside the network
sandbox. Integration tests use two-worker Tokio runtimes for the provider's
synchronous-to-async table-registration bridge.

`cargo run -p stack-check --offline` exited 0 with the existing B1 and B2
PASS lines. Both inverse tree checks show one version, datafusion 54.1.0
and arrow 58.4.1. The full tree contains no DuckDB. Cargo.lock still has
477 packages and no git sources.

## Slice 1.4 vendored overwrite patch

Root Cargo.toml patches crates.io iceberg to `vendor/iceberg-0.11.0` and
excludes that directory from workspace membership. Its shipped Cargo.toml
and version are unchanged. LICENSE, NOTICE, README, source, and testdata are
preserved. The registry checksum file is omitted. The root lockfile records
Iceberg as a path source. All package versions remain identical.

The maintenance rule is a byte comparison against the cached pristine
0.11.0 package. Only these source files may differ:

- `src/transaction/overwrite.rs`: new public OverwriteAction with file adds,
  removals, commit UUID, and snapshot properties.
- `src/transaction/mod.rs`: module registration, export, and Transaction::overwrite.
- `src/transaction/snapshot.rs`: explicit removals, manifest rewrites, pure
  deletes, empty overwrites, and accurate summary deltas.

Do not edit any other vendored file. Keep this patch suitable for upstream
review. Added files use the existing append validation and duplicate check.
A removal must still be a live file with identical metadata at commit time.
Manifests without removals pass through unchanged. Rewritten manifests mark
removed live entries deleted and preserve kept entries' snapshot and sequence
numbers. Older deleted entries in a rewritten manifest are omitted; older
snapshots still reference their original immutable manifests.

The cached summary helper's third argument is `truncate_full_table`, not an
`is_overwrite` flag. Passing true would count every prior row as removed,
including untouched files. The patch uses explicit remove_file accounting
and passes false for delta totals. The snapshot operation is Overwrite.
Writer tests verify total records, files, bytes, and zero delete files.

The writer uses Iceberg's per-file Arrow reader and its Parquet writer stack.
No direct Parquet dependency or other new external dependency is needed.
The obsolete blocker test and compile-fail doctests are removed.

Slice 1.4 verification (Nov 7, 2026):
`cargo test --workspace --offline --locked -- --nocapture` passed 22 tests:
8 engine, 2 catalog, 11 writer, and 1 combined stack. There were 0 failures,
0 ignored tests, and 0 doctests. The Postgres test connected to localhost:5433
and printed `PASS Postgres catalog round trip`. The new writer suite ran
against fresh temporary SQLite catalogs and filesystem warehouses.
All 477 lockfile package names and versions match the previous lockfile.
Only Iceberg's source and registry checksum fields changed. Byte comparison
against the cached source confirmed exactly the three patch files above;
all other vendored files, including Cargo.toml, match.

## Slice 1.5 verification

`cargo test --workspace --offline -- --nocapture` passed 27 tests:
8 engine, 2 catalog, 5 maintenance, 11 writer, and 1 combined stack.
There were 0 failures, 0 ignored tests, and 0 doctests. The Postgres test
connected to localhost:5433 and printed `PASS Postgres catalog round trip`.
The run needed local socket access outside the network sandbox.
The maintenance fixture compacted 50 files and 500 rows to one file under
its 64 KiB target. Its five test names are recorded in
`crates/opensnow-storage/tests/maintenance.rs`. No dependency, lockfile,
version, or vendored source changed.

## Slice 1.6 dependency update

Six already locked crates became direct workspace pins at their locked
versions, for the opensnow-control crate: sha2 0.10.9, hmac 0.12.1, and
rand 0.8.8 (credential hashing and token generation), hex 0.4.3 and
subtle 2.6.1 (encoding and constant time compare), and sqlparser 0.62.0
(the execute path guard; datafusion 54.1.0 already depended on the same
version). Licenses are in the pins table above and confirmed in cached
manifests. No version changed and no new external package was added.
Cargo.lock goes from 477 to 478 packages: the one addition is the
opensnow-control crate itself.

The dev profile also gains opt-level 2 overrides for opensnow-control,
sha2, and hmac. Password hashing is CPU-bound by design, and unoptimized
debug code made PBKDF2 slow enough to break the provisioning time bar
under test load (6.09 seconds measured). With the overrides, the same
test measures 0.20 seconds. No algorithm, iteration count, or version
changed.

Slice 1.6 verification (Nov 9, 2026): `cargo test --workspace --offline`
passed 38 tests: 9 engine, 3 control unit, 7 control integration, 2
catalog, 5 maintenance, 11 writer, and 1 combined stack. There were 0
failures and 0 ignored tests. The control integration tests ran against
throwaway Postgres databases on localhost:5433, and the provisioning
test measured 0.20 seconds end to end.
