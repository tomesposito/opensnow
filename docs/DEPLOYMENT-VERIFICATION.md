# Issue #19 local verification

Run date: 2026-11-16. Branch: work/issue-19-deploy-packaging.
No AWS account access, cloud resource, purchase, OAuth change or Registry
submission. No Rust code, dependency, lockfile or existing test changed.

## Source check

GitHub main resolved to 42efa2771c029a7d1a4f6a0210faf7b849429c86.
Its Git commit API returned tree
25e759c43a8916c7e6a101c07a32a2c523446b84, equal to local
`git rev-parse 'main^{tree}'`. The existing requested branch was clean
at eba45a5, the same tree. No work was built from a stale main tree.

## Workspace suite

From the repository root, with the existing spike toolchain:

```sh
export CARGO_HOME="$PWD/spikes/B2-iceberg/cargo"
export RUSTUP_HOME="$PWD/spikes/B2-iceberg/rustup"
export CARGO_TARGET_DIR="$PWD/spikes/B2-iceberg/rs-spike/target"
export CARGO_BUILD_JOBS=2
export PATH="$CARGO_HOME/bin:$PATH"
cargo test --workspace --offline
```

Exit 0. Totals: **75 passed, 0 failed, 0 ignored, 0 skipped, 0 filtered**.
There are no doctests. The actual localhost:5433 Postgres 16.4 fixture
was reachable during this run. Hosted and catalog tests ran against it.

| Group | Passed |
| --- | ---: |
| Control credentials | 3 |
| Control integration | 13 |
| SQL guard | 5 |
| Engine | 15 |
| Gate 1 small harness | 1 |
| MCP | 9 |
| UI | 2 |
| Catalog | 2 |
| Maintenance | 5 |
| Writer | 19 |
| Combined stack | 1 |
| Total | 75 |

An earlier run inside the restricted socket sandbox failed the 13
control integration tests with PermissionDenied. The complete suite
above was rerun with local-network permission and passed. No test was
changed or weakened to obtain that result.

## Release build

The release was built from a separate source-only archive, with the
existing local release dependency cache. The archive retains tracked
spike files, but contains no local toolchain, Cargo registry or build
output. Docker excludes spikes entirely through .dockerignore. Rust
and Cargo both report 1.99.1.
The staged tree below contains the packaging source and updated embedded
MCP.md; later edits affect only verification cleanup and reporting, not
Rust sources or embedded docs. The final Dockerfile also pins the same
base digest pulled by the container attempt below. To repeat on a fresh
checkout, replace this historical staged-tree ID with HEAD.

```sh
mkdir -p /tmp/opensnow-issue19-clean
git archive 7dbd5bc48695315ab085747a7566421870dd479a | tar -x -C /tmp/opensnow-issue19-clean
export CARGO_HOME="$PWD/spikes/B2-iceberg/cargo"
export RUSTUP_HOME="$PWD/spikes/B2-iceberg/rustup"
export CARGO_TARGET_DIR="$PWD/target"
export CARGO_BUILD_JOBS=2
export PATH="$CARGO_HOME/bin:$PATH"
cd /tmp/opensnow-issue19-clean
cargo build --release --locked -p opensnow-mcp
```

This is the Dockerfile's exact Cargo build command. It runs natively in
the brief's runtime-gap fallback, not inside Docker. Exit 0:

```text
Finished `release` profile [optimized] target(s) in 59m 27s
```

Back at the repository root,
`cmp docs/MCP.md /tmp/opensnow-issue19-clean/docs/MCP.md` returned 0.
The embedded MCP source is the updated version. Binary identity:

```text
6a9f6ee46b50d032f1a7ea64ff2926fc6151bd4ffb3977b450d537b9bf7a5654  target/release/opensnow-mcp
```

Actual `ldd target/release/opensnow-mcp` output:

```text
linux-vdso.so.1 (0x00007c7fbb7e6000)
libgcc_s.so.1 => /lib/x86_64-linux-gnu/libgcc_s.so.1 (0x00007c7fbb7a5000)
libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6 (0x00007c7fb2d17000)
libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007c7fb2b04000)
/lib64/ld-linux-x86-64.so.2 (0x00007c7fbb7e8000)
```

These are the shared-library names allowed by the Dockerfile's runtime
copy step. No source tree or compiler is needed at runtime.

## Container runtime gap

Docker and Podman were absent at the start. Docker 29.1.3 was installed
from Ubuntu's OS packages, as allowed by the brief. A disposable daemon
started with vfs storage, no bridge, and no iptables changes. It could
pull hello-world, but could not run it:

```text
docker run --rm --network host hello-world
failed to create default sandbox: failed to create a netlink handle:
failed to set into network namespace 27 while creating netlink socket:
operation not permitted

docker run --rm --network none hello-world
OCI runtime create failed: runc create failed: unable to start container
process: error spawning mount remapping thread: mount source thread:
join container mntns: setns: operation not permitted
```

The shipped script was also run against a staged clean tree:

```sh
DOCKER_HOST=unix:///tmp/opensnow-docker.sock ./deploy/verify-container.sh 7dbd5bc48695315ab085747a7566421870dd479a
```

Exit 1. Actual output, with pull progress omitted:

```text
Sending build context to Docker daemon  6.401MB
Step 1/14 : FROM rust:1.99.1-bookworm AS build
Digest: sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c
Status: Downloaded newer image for rust:1.99.1-bookworm
Step 4/14 : RUN cargo build --release --locked -p opensnow-mcp
OCI runtime create failed: runc create failed: unable to start container process:
error spawning mount remapping thread: mount source thread:
join container mntns: setns: operation not permitted
```

User namespace creation also failed: `unshare -Ur true` reported
`cannot open /proc/self/uid_map: Read-only file system`. Installing a
runtime does not remove these VM restrictions. A container start and
round-trip are **unverified**, not passed. The sweep must run
`./deploy/verify-container.sh` on a capable Docker host before closing
that gap. The script builds the committed clean tree, starts an isolated
throwaway Postgres database and fresh warehouse volume, checks health,
provisions a token, and runs CREATE, INSERT and SELECT. Test containers, databases, warehouse volume and network
are removed on exit; Docker base images and build cache may remain.
Failures return a nonzero status.

## HTTPS

The official Caddy 2.10.2 Linux amd64 archive was checked against the
release's SHA512 checksums. Result:

```text
PASS Caddy 2.10.2 official SHA512:
747df7ee74de188485157a383633a1a963fd9233b71fbb4a69ddcbcc589ce4e2cc82dacf5dbbe136cb51d17e14c59daeb5d9bc92487610b0f3b93680b2646546
```

Caddy 2.6.2 from Ubuntu rejected request_buffers, so the tests used the
verified official 2.10.2 binary, also pinned in the runbook. The real
shipped production config parsed successfully with 2.10.2. For the local
TLS run, verify-tls.sh preserves the proxy block, uses localhost:8443,
and replaces ACME issuance with a one-day openssl self-signed certificate.
The Python client trusts that certificate explicitly and checks its
hostname. Real Caddy terminates TLS; no substitute terminator was used.
Production ACME issuance was not exercised by this local-only test.


## Release HTTP and HTTPS round-trips

From the repository root, with the verified Caddy binary in /tmp:

```sh
PATH="/tmp:$PATH" ./deploy/verify-native.sh target/release/opensnow-mcp
```

Exit 0. Exact stdout below; Caddy's routine JSON stderr logs are omitted.
The script created a UUID-named throwaway database on localhost:5433,
started the real release server on 127.0.0.1:8080 with a fresh warehouse,
and ran the HTTP and HTTPS checks. The SELECT response was asserted to
be `[["19"]]` before printing PASS. Cleanup stopped server and proxy,
dropped the database and removed the warehouse and test certificate.

```text
PASS /health: {"status":"ok"}
PASS /api/provision: token returned (redacted)
PASS /api/execute: CREATE TABLE deploy_check (id BIGINT NOT NULL)
PASS /api/execute: INSERT INTO deploy_check VALUES (19)
PASS /api/execute: SELECT id FROM deploy_check
PASS round-trip
Valid configuration
Valid configuration
PASS /health: {"status":"ok"}
PASS /api/provision: token returned (redacted)
PASS /api/execute: CREATE TABLE deploy_check (id BIGINT NOT NULL)
PASS /api/execute: INSERT INTO deploy_check VALUES (19)
PASS /api/execute: SELECT id FROM deploy_check
PASS round-trip
PASS HTTPS via real Caddy; shipped config validated; self-signed test certificate
PASS native binary HTTP and HTTPS; throwaway database and warehouse removed on exit
```

The temporary Docker daemon was stopped and its failed build container
removed. The pre-existing Postgres fixture and its authentication
configuration were left intact. Docker base images and build cache may
remain locally. The installed OS tools create no cloud resources.
