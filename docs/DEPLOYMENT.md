# Deployment

This is a local packaging release for issue #19. No AWS account was
accessed and no resource was created. Tom runs the AWS steps below only
after the AWS go in issue #17. Tags [#17] identify the shared cost-run path.

## Shape and limits [#17]

Use us-east-1, Linux x86_64, on-demand. Keep the reference prices from
[COST-MODEL.md](COST-MODEL.md): one always-on m7i.large (2 vCPU, 8 GiB),
$0.1008/hour for 730 hours, plus 100 GB gp3 at $8/month, about $82/month.
It runs Postgres 16, the packaged server, and Caddy. The warehouse lives
on that encrypted gp3 volume at /srv/opensnow/warehouse. Back up both
Postgres and this directory together. Losing either can lose tables.

The Gate 1 workload machines are m7i.xlarge ($0.2016/hour), m7i.2xlarge
($0.4032/hour), and m7i.4xlarge ($0.8064/hour). Launch them only for the
cost run and stop or terminate them afterward. W1 and W5 use 4xlarge;
W2, W3, and W4 use 2xlarge; W6 and the small W5 run use xlarge.

The current server is a single process containing control plane and
query engine. It has no remote worker scheduler. The separate workload
machines run the Gate 1 harness, not workers registered with this server.
Do not count this first hosted boot as a measurement of distributed
serving or full-scale Gate 1. Record that limit in #17.

The model puts durable object storage in the customer's own S3 bucket.
This HTTP binary currently accepts a local warehouse path only. There
is no bucket environment setting to enable. Keep the customer's bucket
and its IAM policy separate and intact; do not point a local path at
an S3 URI or claim this package tests S3. The first boot below uses gp3.

Security group: public inbound TCP 443 only, to Caddy. No public 80,
8080, or 5432. Use Systems Manager Session Manager for administration,
with an instance role and outbound access for SSM. Workload nodes need
no inbound rule. The app and Postgres bind loopback via host networking;
Caddy is the sole public listener. Allow outbound DNS, HTTPS for image
pulls, SSM and ACME, and required OS updates. Do not add a public SSH rule.

Caddy terminates TLS to keep certificates out of the Rust server and
avoid a new Rust TLS dependency. It supplies automatic renewal with
TLS-ALPN-01 on 443, so port 80 stays closed. The shipped
[configuration](../deploy/Caddyfile) preserves request JSON and the
Authorization and Host headers. It buffers request bodies for this
Content-Length-only listener and uses HTTP/1.1 upstream. Caddy adds its
usual forwarding headers; the server does not use them for auth.

## 1. Prepare the release locally [#17]

Use a clean committed checkout on an x86_64 Linux Docker host. Rust
1.99.1 is installed inside the build stage. Cargo.lock and the vendored
Iceberg patch are copied from the checkout; spikes and local data are not.
Build on a machine with enough memory and disk, rather than on the
m7i.large serving host. Do not put credentials in the build context.

```sh
git status --porcelain                 # must be empty
export RELEASE=$(git rev-parse --short=12 HEAD)
docker build --pull --platform linux/amd64 -t opensnow:$RELEASE .
./deploy/verify-container.sh
# The test builds from git archive HEAD and destroys its DB and volume.
docker save opensnow:$RELEASE | gzip > opensnow-$RELEASE.tar.gz
sha256sum opensnow-$RELEASE.tar.gz > opensnow-$RELEASE.tar.gz.sha256
```

Ship the archive, checksum, deploy/ directory, and this runbook through
Tom's existing authenticated artifact channel. An alternative is a
private registry: tag as REGISTRY/opensnow:$RELEASE, docker login to that
registry, docker push, then docker pull on the instance. REGISTRY is a
placeholder, not an account or a new registry this issue creates.
Retain the previous image and its release SHA for rollback.

The scratch runtime has no shell or HTTP client. Its equivalent external
health check is `curl --fail --silent --show-error http://127.0.0.1:8080/health`
on the host. Expect `{"status":"ok"}`. Monitor this and the HTTPS check
below. Docker does not supply an internal HEALTHCHECK status.

## 2. Allocate the host and record the meter [#17]

In the EC2 console, select us-east-1, Launch instance, Ubuntu Server
24.04 LTS x86_64, m7i.large, on-demand. Select Tom's approved VPC and
subnet with outbound internet access. Attach the SSM instance role. Add
100 GB encrypted gp3, delete on termination only after backups are
exported. Apply the security group above and tag the instance and volume
with the issue and release. Record instance ID, volume ID, AZ, launch
time in UTC, and the selected public IPv4 charge in #17. These charges
are not included in the reference $82 estimate. Do not allocate an ALB,
RDS, NAT gateway, or Elastic IP for this shape.

Point Tom's chosen DNS hostname at the instance's public address. DNS
must resolve before certificate issuance. The host must be reachable
on 443 from the internet for the ACME TLS challenge. A stop/start can
change the address; update DNS before serving again.

Connect through Session Manager, then prepare the host:

```sh
sudo apt-get update
sudo apt-get install -y docker.io curl ca-certificates caddy
sudo systemctl enable --now docker
sudo systemctl stop caddy
sudo install -d -m 700 /etc/opensnow
sudo install -d -o 65532 -g 65532 -m 700 /srv/opensnow/warehouse
```

Use Caddy 2.10.2, because the distro's 2.6.2 lacks request_buffers.
Fetch the official binary and verify it against the release checksums:

```sh
curl -fLO https://github.com/caddyserver/caddy/releases/download/v2.10.2/caddy_2.10.2_linux_amd64.tar.gz
curl -fLO https://github.com/caddyserver/caddy/releases/download/v2.10.2/caddy_2.10.2_checksums.txt
grep ' caddy_2.10.2_linux_amd64.tar.gz$' caddy_2.10.2_checksums.txt | sha512sum -c -
tar -xzf caddy_2.10.2_linux_amd64.tar.gz caddy
sudo install -m 755 caddy /usr/local/bin/caddy
```

A checksum failure stops the deploy. Keep Caddy security updates and image rebuilds
in the host maintenance schedule. For builder OS security fixes, update
and audit the Dockerfile base digest before rebuilding; the current
digest deliberately fixes the audited OS contents.

## 3. Create environment files [#17]

Use `sudoedit` to create these root-owned mode 600 files. Values never
belong in git, image layers, command history, or the issue report.
Docker administrators can read container environments, so restrict
membership of the docker group to administrators.

| File | Names | Set it to / failure if absent |
| --- | --- | --- |
| /etc/opensnow/postgres.env | POSTGRES_USER, POSTGRES_DB, POSTGRES_PASSWORD | Create a dedicated database owner, database and random password. The official Postgres image refuses an empty password. |
| /etc/opensnow/server.env | OPENSNOW_CONTROL_DATABASE_URL, OPENSNOW_WAREHOUSE | URI for the loopback Postgres database using the same owner/password; warehouse path /warehouse inside the container. Missing URI prevents boot; wrong path hides existing data. Percent-encode URI password characters. |
| /etc/opensnow/proxy.env | OPENSNOW_DOMAIN, OPENSNOW_TLS_EMAIL | Public DNS hostname and certificate contact address. Missing hostname prevents correct site routing/cert issuance. |

```sh
sudoedit /etc/opensnow/postgres.env /etc/opensnow/server.env /etc/opensnow/proxy.env
sudo chmod 600 /etc/opensnow/*.env
```

Use plain KEY=value lines in these files, with no shell expansion or
quotes. These proxy names configure Caddy, not the Rust server. Caddy's
certificate keys and account persist in the caddy user's application
data directory under /var/lib/caddy; protect and back up that directory.

## 4. First boot [#17]

On the instance, verify the transferred checksum, load the image and
set RELEASE to the shipped release SHA:

```sh
sha256sum -c opensnow-$RELEASE.tar.gz.sha256
gunzip -c opensnow-$RELEASE.tar.gz | sudo docker load
sudo docker volume create opensnow-postgres
sudo docker run -d --name opensnow-postgres --restart unless-stopped \
  --network host --env-file /etc/opensnow/postgres.env \
  -v opensnow-postgres:/var/lib/postgresql/data \
  postgres:16-bookworm -c listen_addresses=127.0.0.1
sudo docker exec opensnow-postgres sh -c 'pg_isready -h 127.0.0.1 -U "$POSTGRES_USER" -d "$POSTGRES_DB"'
```

Wait for pg_isready to say accepting connections. For a reproducible
cost run, record the resolved Postgres image digest (`sudo docker image
inspect postgres:16-bookworm --format '{{json .RepoDigests}}'`). Its data
volume holds tenants, user/password hashes, token hashes, worksheets,
metering records, and the catalog. Do not delete it when swapping the app.

```sh
sudo docker run -d --name opensnow --restart unless-stopped \
  --network host --env-file /etc/opensnow/server.env \
  -v /srv/opensnow/warehouse:/warehouse \
  opensnow:$RELEASE --http 127.0.0.1:8080
curl --fail --silent --show-error http://127.0.0.1:8080/health
sudo install -m 644 deploy/Caddyfile /etc/caddy/Caddyfile
sudo mkdir -p /etc/systemd/system/caddy.service.d
sudo tee /etc/systemd/system/caddy.service.d/opensnow.conf >/dev/null <<'UNIT'
[Service]
EnvironmentFile=/etc/opensnow/proxy.env
ExecStart=
ExecStart=/usr/local/bin/caddy run --environ --config /etc/caddy/Caddyfile
ExecReload=
ExecReload=/usr/local/bin/caddy reload --config /etc/caddy/Caddyfile --force
UNIT
sudo sh -c 'set -a; . /etc/opensnow/proxy.env; /usr/local/bin/caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile'
sudo systemctl daemon-reload
sudo systemctl enable --now caddy
```

Expect health status ok, a running container, and Caddy certificate
issuance in `sudo journalctl -u caddy`. Check that `sudo ss -lntp` shows
8080 and 5432 only on 127.0.0.1. TCP 443 belongs to Caddy. Do not proceed
if either backend binds a public interface.

On an operator machine, set OPENSNOW_DOMAIN to the chosen hostname:

```sh
curl --fail --silent --show-error "https://$OPENSNOW_DOMAIN/health"
python3 deploy/round-trip.py "https://$OPENSNOW_DOMAIN"
```

Expect health ok, provisioning with a token returned, CREATE TABLE,
INSERT, and SELECT returning row 19. The script creates a fresh test
tenant and does not print credentials. This tenant remains until data
cleanup. For the first real tenant, use the UI at that HTTPS address or
POST /api/provision with name, admin_login and admin_password from a
mode 600 JSON file: `curl --fail --json @first-tenant.json
"https://$OPENSNOW_DOMAIN/api/provision"`. Store its one-time token in
Tom's secret store, then delete the JSON file. Query with POST
/api/execute, Authorization: Bearer, and a JSON sql field as documented
in [MCP.md](MCP.md). Never paste a real token into the cost report.

Waiting list: issue #16 has not shipped. There is no mode, flag or
environment variable to set. Insert its configuration here, before
opening enrollment, once #16 implements it. Until then paid conversion
is manual, as described in #16. Provisioning is currently public; a
private pilot should restrict access at the security group to pilot
source addresses until Tom approves public enrollment.

## 5. Run the cost session [#17]

Follow steps 1 through 4 for the common control-plane path. Launch the
workload instance sizes from the shape section with the same OS and
security rules. On a build host at the same release, build the existing
harness with `cargo build --release --locked -p opensnow-gate1`. Ship
`target/release/opensnow-gate1` plus its checksum to each workload node. Run
`ldd ./opensnow-gate1` there first; no dependency may be missing.

```sh
date -u
./opensnow-gate1 --divisor 1000 --json gate1.json > gate1.md
date -u
```

This is the existing scaled harness, not a full-scale or S3 validation.
Use docs/GATE1-EVALUATION.md and docs/workloads/ for interpretation.
Save the reports, release SHA, hardware size, instance and volume IDs,
UTC start/stop times, image digests, bytes stored and actual billed
compute/storage/network charges in #17. Compare measured runtime and
cost with COST-MODEL.md without silently re-pricing its reference table.
Issue #17 owns any larger-scale measurement and the final AWS verdict.
Finish step 7 the same session to stop the meter.

## 6. Upgrade and roll back

Build and verify a new committed release with step 1. Take a consistent
Postgres dump and warehouse snapshot while writes are stopped, plus a
backup of Caddy state and the protected environment files. Record the
old image SHA. Stop writes and make the matched backup before swapping:

```sh
umask 077
sudo docker stop opensnow
sudo docker exec opensnow-postgres sh -c 'pg_dump -U "$POSTGRES_USER" -d "$POSTGRES_DB" --format=custom' > opensnow-$RELEASE.pgdump
sudo sh -c 'umask 077; tar -C / -czf /root/opensnow-backup.tar.gz srv/opensnow/warehouse etc/opensnow var/lib/caddy'
```

Export both files through the protected backup channel, and check that
the dump can be listed with pg_restore --list and the archive with
tar --list before continuing. Load the new image, then:

```sh
sudo docker stop opensnow
sudo docker rename opensnow opensnow-previous
# Repeat step 4's app docker run with the new RELEASE.
curl --fail --silent --show-error "https://$OPENSNOW_DOMAIN/health"
```

Use an existing tenant token to SELECT a known table after restart;
health alone does not check durable data. Only remove opensnow-previous
after that check succeeds. Boot calls ensure_schema, which creates
missing control tables and indexes. It is not a general migration
framework, a backup, or a guarantee that an older binary can read a
future schema. Review future schema changes before upgrading.

For this packaging release, rollback is `sudo docker rm -f opensnow`
then `sudo docker rename opensnow-previous opensnow` and
`sudo docker start opensnow`. Keep the same env files and warehouse.
For an incompatible future migration restore the matched Postgres and
warehouse backup before restarting the old version. Never restore only
one side of the catalog/data pair.

## 7. Teardown and meter stop [#17]

Export reports and approved backups first. Name what will be lost:
Postgres contains tenants, users, credential hashes, worksheets, cost
records and catalog metadata; gp3 contains local table files. Caddy
state contains certificate private keys. Keep their backups protected
and delete them only under Tom's retention decision.

```sh
sudo systemctl disable --now caddy
sudo docker rm -f opensnow
sudo docker rm -f opensnow-postgres
# Irreversible, only after the backups above have been checked:
sudo docker volume rm opensnow-postgres
```

In EC2, terminate all issue-tagged workload and control-plane instances.
Check Volumes for detached gp3 disks and delete those selected for
removal. Remove the DNS record, issue-only security group and instance
role if no other service uses them. Release any address accidentally
allocated during the session. Confirm every instance is terminated,
every intended volume is gone, and record the UTC meter-stop time and
remaining billable resources in #17. The customer's own S3 bucket and
its objects survive on purpose; do not delete them. Verify actual
billing after its normal reporting delay.

The #17 checklist is: reference shape and limits; step 1 build identity;
step 2 allocations and meter start; step 3 secret setup; step 4 HTTPS
boot and round-trip; step 5 measurements; step 7 teardown and billing.

## Local verification and licenses

See [deployment verification](DEPLOYMENT-VERIFICATION.md) for exact local
commands, outputs, counts, and remaining container execution gap, and
[the D-04 inventory](DEPLOYMENT-LICENSES.md) for the image content check.
To repeat HTTPS locally, start the real server on 127.0.0.1:8080 with a
throwaway Postgres database and fresh warehouse, put Caddy 2.10.2 on
PATH, then run `./deploy/verify-tls.sh`. It generates a one-day localhost
certificate with openssl, validates the shipped config with real Caddy,
and runs health, provision and SQL through real Caddy over HTTPS. The
certificate is trusted only by that test client; production uses ACME.
