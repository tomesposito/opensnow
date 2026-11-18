# Deployment D-04 license check

Deployment checked 2026-11-16. The locked Rust audit also covers
the PostgreSQL wire TLS dependencies added for issue #39. The core
remains Apache-2.0. Existing pins and the vendored Iceberg Apache-2.0
patch are described in [PINNED-STACK.md](PINNED-STACK.md).

## Images

The build stage is `rust:1.99.1-bookworm`, the official Rust image.
The [image source](https://github.com/rust-lang/docker-rust) and
[Rust copyright](https://github.com/rust-lang/rust/blob/main/COPYRIGHT)
identify Rust's MIT OR Apache-2.0 terms. The Debian Bookworm build OS
is a collection of packages, not a single licensed program. Its
[license policy](https://www.debian.org/legal/licenses/) identifies
main as free software. The compiler, Cargo, build tools and OS programs
do not enter the final image. This VM could not execute a container;
the application image build remains unverified here. The base image
was pulled at digest
`sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c`.
Without starting it, docker cp retrieved its dpkg status and libc6,
libgcc-s1 and ca-certificates copyright files. They confirm libc6 2.36-9+deb12u14,
libgcc-s1 12.2.0-14+deb12u1 and ca-certificates 20250419~deb12u2.
The copied library notices confirm the LGPL and GCC exception below.

The runtime base is `scratch`, an empty filesystem, not a distribution.
It adds no program or license. Dockerfile copies only the following:

| Shipped content | License and source |
| --- | --- |
| opensnow-mcp, including embedded UI and docs | Apache-2.0, workspace manifests |
| Vendored worksheet chart renderer | BSD-3-Clause, ISC, MIT, and Unlicense. See the [renderer inventory](../vendor/vega/dependency-licenses.tsv). |
| Statically linked Rust dependencies | MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, CC0, Unlicense, bzip2 and BSL alternatives listed in the inventory below |
| glibc loader, libc and libm | LGPL-2.1-or-later with upstream exceptions; [glibc COPYING.LIB](https://sourceware.org/git/?p=glibc.git;a=blob;f=COPYING.LIB;hb=HEAD); Debian libc6 copyright copied into the runtime |
| libgcc_s | GPL-3.0 with GCC Runtime Library Exception 3.1; [upstream exception](https://gcc.gnu.org/onlinedocs/libstdc++/manual/license.html), [exception text](https://github.com/gcc-mirror/gcc/blob/master/COPYING.RUNTIME); Debian libgcc-s1 copyright copied into the runtime |
| CA certificate bundle | Mozilla CA data, MPL-2.0; Debian ca-certificates packaging notices copied into the runtime, [Debian source copyright](https://sources.debian.org/src/ca-certificates/20230311%2Bdeb12u1/debian/copyright/) |
| License texts, notices and dependency inventory | Cached upstream license/notice files, vendored Iceberg LICENSE and NOTICE, and facts from locked package manifests, under /usr/share/doc/opensnow |
| Writable empty /warehouse and /tmp directories | No software; warehouse owner 65532, tmp mode 1777 |

The locked features build SQLite bundled and LZMA static. The Dockerfile
prints ldd and rejects unexpected shared library names.
The local release binary's exact ldd output is in the verification
report. Runtime copies retain the library paths from the builder.
LGPL libraries remain dynamically linked and replaceable. Corresponding
Debian library source is available from the Debian package sources;
retain the release's exact Debian package versions and image digest
when distributing images. The GCC exception permits linking this
Apache-2.0 executable against libgcc. No AGPL or source-available
component is introduced. There is no shell, curl, package manager,
Postgres, Caddy, Rust compiler or Cargo in the app image.

## Locked dependency audit

[dependency-licenses.tsv](../deploy/dependency-licenses.tsv) records every
package returned by the Linux workspace metadata, including build and
test dependencies as a conservative superset of the linked executable.
Each entry has a declared license; none names AGPL, SSPL, BUSL or a
proprietary license. Dual-licensed packages may use their permissive
alternative. The table is from cached upstream manifests at the locked
versions, rather than inferred from package names. Reproduce with the
spike toolchain environment from PINNED-STACK.md:

```sh
cargo metadata --offline --locked --filter-platform x86_64-unknown-linux-gnu --format-version 1 > metadata.json
python3 - <<'PY'
import json
packages = sorted(json.load(open('metadata.json'))['packages'], key=lambda p: (p['name'], p['version']))
for p in packages:
    assert p['license'] and not any(x in p['license'].upper() for x in ('AGPL', 'SSPL', 'BUSL', 'PROPRIETARY')), p['name']
print('package\tversion\tlicense')
for p in packages:
    print(p['name'], p['version'], p['license'], sep='\t')
PY
```

This metadata command does not add packages or update Cargo.lock. The build also copies upstream registry license and notice files and
the vendored Iceberg LICENSE and NOTICE into the runtime. The inventory
is an audit, not replacement license text. Retain those notices on
redistribution.

## Browser renderer

The issue #38 D-04 check passed before implementation. The pinned Vega
5.33.0, Vega-Lite 5.23.0, and Vega-Embed 6.29.0 releases are
BSD-3-Clause. The browser bundles come from the npm releases. Their
source maps and release lockfiles identify the bundled dependencies.
The renderer inventory records their npm manifest licenses and notice
locations. Docker copies the license texts and inventory beside the
existing Rust notices.

## Adjacent services

Caddy 2.10.2 is Apache-2.0, verified from its
[upstream license](https://github.com/caddyserver/caddy/blob/v2.10.2/LICENSE).
It runs on the host, outside the app image. The local HTTPS test used
the official Caddy release, not a substitute TLS terminator.

The runbook and container test use `postgres:16-bookworm`, outside the
app image. PostgreSQL uses the permissive
[PostgreSQL License](https://www.postgresql.org/about/licence/); Debian
supplies its OS packages under their individual free-software terms.
Record its image digest during the real #17 run. Neither Caddy nor
Postgres adds a Rust dependency.
