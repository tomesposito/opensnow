# Pinned engine stack

Slice 1.1, issue #4. Checked locally on Nov 6, 2026.

Use Candidate B: DataFusion 54.1.0 with Iceberg 0.11.0. The shipped
provider and both harness paths share one dependency tree. All external
sources in Cargo.lock are crates.io registry sources. No DuckDB dependency
or fork is needed. This uses the public extension points required by D-01.

## Pins and direct dependency licenses

All versions below are exact workspace pins. Cargo.lock pins the rest.

| Crate | Version | Source | License |
| --- | --- | --- | --- |
| datafusion | 54.1.0 | crates.io | Apache-2.0 |
| arrow | 58.4.1 | crates.io | Apache-2.0 |
| iceberg | 0.11.0 | crates.io | Apache-2.0 |
| iceberg-datafusion | 0.11.0 | crates.io | Apache-2.0 |
| iceberg-catalog-sql | 0.11.0 | crates.io | Apache-2.0 |
| tokio | 1.54.1 | crates.io | MIT |
| futures | 0.3.34 | crates.io | MIT OR Apache-2.0 |
| tempfile | 3.27.0 | crates.io | MIT OR Apache-2.0 |
| sqlx | 0.8.6 | crates.io | MIT OR Apache-2.0 |

The three opensnow crates use Apache-2.0. SQLx explicitly enables
runtime-tokio, sqlite, and any, with default features off. Without the
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

Upgrade when datafusion-iceberg has a crates.io release for DataFusion 55
or the then-current DataFusion. Require a registry-only lock with one
DataFusion and Arrow version, the same combined test passing, and a fresh
license check. A git dependency needs a separate written D-01 justification.

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
