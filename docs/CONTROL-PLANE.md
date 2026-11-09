# Control plane

Slice 1.6, issue #9. The `opensnow-control` crate holds tenants, users,
and tokens in Postgres, and it owns the one authenticated execute path.
The in-process SQL endpoint and any future UI caller share the same auth
check. There was no HTTP transport and no UI in this slice. Transport
shipped in slice 1.9, issue #12; see docs/MCP.md.

## Schema

Schema setup is plain SQL in the crate, applied by `ensure_schema`. It is
idempotent and uses no migration framework. Five tables:

- tenants: id (text primary key), name (unique), status (active,
  suspended, or deleted), cost_cap_units (nullable bigint), namespace
  (unique), created_at, updated_at.
- users: id (text primary key), tenant_id (references tenants), login,
  credential_hash, role (admin or member), status, created_at,
  updated_at. Logins are unique within a tenant.
- tokens: id (text primary key), tenant_id (references tenants), user_id
  (references users), token_hash (unique), scopes (read, write, or both),
  revoked flag, expires_at, created_at, updated_at.
- query_cost_records: tenant_id (references tenants) and seq, unique
  together, SQL text, kind, status, rows and bytes returned, elapsed
  milliseconds, vCPU-seconds, cost in dollars, start time. Added in
  slice 1.7, issue #10. See docs/METERING.md.
- worksheets: id (text primary key), tenant_id (references tenants),
  user_id (references users), name, sql_text, created_at, updated_at.
  Added in slice 1.8, issue #11, for server-side saved worksheets.
  See docs/WORKSHEETS-UI.md.

Every tenant-scoped read filters by tenant id in the query itself.

## Provisioning

One call creates a tenant, its first admin user, and a first token with
write scope, in one transaction. It then creates the tenant's catalog
namespace and registers the tenant with the engine, using the cost cap
from the tenant record. If namespace creation or engine registration
fails, the tenant is marked deleted, so nothing half-provisioned can
authenticate.

The measured provisioning time in the acceptance test was 0.20 seconds
in the final verification run on this machine, well under the 5 second
bar from the design. The test asserts the bar on every run. Password
hashing is CPU-bound by design, so the dev profile builds the control
crate and its crypto crates optimized. See PINNED-STACK.md.

## Credentials and tokens

Passwords are never stored in plaintext. A password is stored as
PBKDF2-HMAC-SHA256 with 100,000 iterations and a random 16 byte salt, in
the form `pbkdf2-sha256$<iterations>$<salt hex>$<hash hex>`. The scheme
is built on the already locked hmac and sha2 crates. Verification uses a
constant time compare. A missing login still pays one verification
against a dummy hash.

Tokens are 32 random bytes, shown once at issue time with an `osn_`
prefix. Only the SHA-256 hash of a token is stored. The plaintext is
never stored or logged. A token has scopes and an expiry. The default
expiry is 24 hours. A token issued with no scopes requested gets read
only. Write scope implies read on the execute path. Tokens can be
revoked, and revocation takes effect on the next call.

Every authentication failure returns the same error. A wrong password, a
wrong token, an expired token, a revoked token, and a token or user on a
suspended or deleted tenant all fail the same way, so an unauthenticated
caller cannot tell which check failed or whether a login exists.

## Lifecycle

- Create: see Provisioning. Duplicate tenant names are rejected, and a
  duplicate user login within a tenant is rejected. A rejected duplicate
  does not disturb the existing records.
- Suspend: the tenant record is marked suspended and the tenant is
  unregistered from the engine. Authentication and query execution fail
  at once. Data and records stay intact. Unsuspend marks the tenant
  active and registers it with the engine again, with its data intact.
- Delete: the tenant record is marked deleted, all of its tokens are
  revoked in the same transaction, and the tenant is unregistered from
  the engine. Other tenants are unaffected.

Delete does not erase storage. The tenant row stays as a deleted
tombstone, so a deleted tenant's name stays taken. Its namespace, tables,
and snapshots in the catalog are not dropped. Full erase across
snapshots, clones, and shares is a later gap named in the design. This
slice does not claim it.

## Isolation

Each tenant gets its own catalog namespace at provisioning. The
namespace is `t` plus the first 16 characters of the tenant id with
dashes removed. Tables live under `ice.<namespace>.<table>`.

The authenticated execute path checks the token, the tenant status, and
the scope before the engine runs anything. It then rewrites the SQL:
every table name after FROM, JOIN, INTO, or TABLE is qualified to the
caller's own namespace. A name that points at another namespace is
rejected. Input rules fail closed: exactly one statement, plain
identifiers only, no comments, and only query, insert, create table,
drop, delete, and update statements. Listing tables goes through the
catalog scoped to the caller's namespace.

The isolation test proves the shape. Two tenants each create a table
named events and load different rows. Each reads back only its own rows.
Tenant B cannot list or query tenant A's table by a qualified name, and
A's token used against B's scope fails.

## Cost caps

The tenant record's cost_cap_units is the value passed to the engine's
ResourceGroup at registration. A tenant provisioned with a small cap
hits CostCapExceeded through the authenticated path. Changing the cap on
the record re-registers an active tenant with the new value. That
re-registration resets the engine's cumulative cost counter, which lives
only for the registration. Dollar metering and per-query cost records
shipped in slice 1.7, issue #10: the authenticated execute path syncs
each tenant's records to the query_cost_records table, and cost_records
and cost_summary read them back per tenant. See docs/METERING.md.

## Limits

- HTTP transport and the MCP server shipped in slice 1.9, issue #12
  (docs/MCP.md). The worksheets and navigation UI shipped in slice
  1.8, issue #11 (docs/WORKSHEETS-UI.md); it is served by the
  transport, not by this crate.
- Dollar metering records live in query_cost_records and are read back
  through cost_records and cost_summary (docs/METERING.md).
  Dollar-denominated budgets are not built; the enforced cap is still
  cost_cap_units, counted in rows.
- No erase across snapshots, clones, and shares. Delete is a tombstone
  plus token revocation, as described above.
- The SQL guard is a guard, not a full SQL firewall. It accepts one
  statement with plain identifiers and qualifies the table names it
  finds. Anything it cannot parse or qualify is rejected, but exotic
  SQL shapes are refused rather than rewritten.
- Password authentication returns a read-scope principal. Write access
  needs a write-scope token.
- A tenant context in the engine is still a resource boundary, not a
  security sandbox. The control plane adds the auth and namespace layer
  on top of it. It does not change what the engine context itself is.

## Tests

The control crate has 3 unit tests for credentials and 8 integration
tests against throwaway Postgres databases on localhost:5433:
provisioning under five seconds, token and password auth outcomes,
tenant isolation, suspend and unsuspend, delete, cost cap enforcement,
duplicate rejection, and metering records read back in dollars per
tenant. The stored-token assertion checks that the
database holds a hash, not the returned plaintext.
