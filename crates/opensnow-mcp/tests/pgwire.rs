use std::ffi::OsString;
use std::io::{self, BufReader};
use std::process::Command;
use std::sync::Arc;

use opensnow_control::{ControlPlane, ControlStore, ProvisionedTenant};
use opensnow_mcp::pgwire::{load_tls_config, serve_pg};
use opensnow_storage::CatalogConfig;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use sqlx::{ConnectOptions, PgConnection};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
const LOGIN: &str = "analyst";
const PASSWORD: &str = "analyst-test-password";
const QUERY: &str = "SELECT d.name, AVG(f.amount) AS average FROM departments d
    JOIN facts f ON f.dept = d.id GROUP BY d.name ORDER BY d.name";

struct Fixture {
    plane: Arc<ControlPlane>,
    database: String,
    _temp: tempfile::TempDir,
}

async fn admin() -> Result<PgConnection, sqlx::Error> {
    sqlx::postgres::PgConnectOptions::new()
        .host("localhost")
        .port(5433)
        .username("postgres")
        .database("postgres")
        .connect()
        .await
}

impl Fixture {
    async fn start() -> TestResult<Option<Self>> {
        let mut admin = match admin().await {
            Ok(admin) => admin,
            Err(sqlx::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::NetworkUnreachable
                        | io::ErrorKind::HostUnreachable
                ) =>
            {
                println!("SKIP pgwire hosted fixture: {error}");
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let database = format!("opensnow_pgwire_{}", uuid::Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE DATABASE {database}"))
            .execute(&mut admin)
            .await?;
        let uri = format!("postgres://postgres@localhost:5433/{database}");
        let store = ControlStore::new(sqlx::PgPool::connect(&uri).await?);
        store.ensure_schema().await?;
        let temp = tempfile::tempdir()?;
        let storage = CatalogConfig::postgres("pgwire-test", uri, temp.path().join("warehouse"))
            .connect()
            .await?;
        Ok(Some(Self {
            plane: Arc::new(ControlPlane::new(store, Arc::new(storage))),
            database,
            _temp: temp,
        }))
    }

    async fn tenant(&self, name: &str) -> TestResult<ProvisionedTenant> {
        Ok(self
            .plane
            .provision_tenant(name, LOGIN, PASSWORD, None, None)
            .await?)
    }

    async fn listener(&self, tls: Option<Arc<ServerConfig>>) -> TestResult<Listener> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        Ok(Listener {
            address,
            task: tokio::spawn(serve_pg(listener, self.plane.clone(), tls)),
        })
    }

    async fn finish(self) -> TestResult {
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.database))
            .execute(&mut admin().await?)
            .await?;
        Ok(())
    }

    async fn costs(&self, tenant: &ProvisionedTenant) -> usize {
        self.plane.cost_records(&tenant.token).await.unwrap().len()
    }
}

struct Listener {
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<io::Result<()>>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Listener {
    async fn connect(&self, database: &str, password: &str) -> TestResult<Client<TcpStream>> {
        let stream = TcpStream::connect(self.address).await?;
        let mut client = Client(stream);
        let frames = client.auth(database, LOGIN, password).await?;
        assert_eq!(frames.last().unwrap().kind, b'Z', "{frames:?}");
        Ok(client)
    }
}

#[derive(Debug)]
struct Frame {
    kind: u8,
    body: Vec<u8>,
}

struct Client<T>(T);

fn push_string(out: &mut Vec<u8>, value: &str) {
    out.extend(value.as_bytes());
    out.push(0);
}

fn frame(out: &mut Vec<u8>, kind: u8, body: &[u8]) {
    out.push(kind);
    out.extend((body.len() as u32 + 4).to_be_bytes());
    out.extend(body);
}

impl<T: AsyncRead + AsyncWrite + Unpin> Client<T> {
    async fn send_startup(&mut self, body: &[u8]) -> TestResult {
        self.0.write_u32(body.len() as u32 + 4).await?;
        self.0.write_all(body).await?;
        self.0.flush().await?;
        Ok(())
    }

    async fn request(&mut self, code: u32) -> TestResult<u8> {
        self.send_startup(&code.to_be_bytes()).await?;
        Ok(self.0.read_u8().await?)
    }

    async fn receive(&mut self) -> TestResult<Frame> {
        let kind = self.0.read_u8().await?;
        let len = self.0.read_u32().await?;
        let mut body = vec![0; (len - 4) as usize];
        self.0.read_exact(&mut body).await?;
        Ok(Frame { kind, body })
    }

    async fn until(&mut self, last: u8) -> TestResult<Vec<Frame>> {
        let mut frames = Vec::new();
        loop {
            let reply =
                tokio::time::timeout(std::time::Duration::from_secs(30), self.receive()).await??;
            let done = reply.kind == last;
            if last != b'Z' {
                assert_ne!(reply.kind, b'E', "{reply:?}");
            }
            frames.push(reply);
            if done {
                return Ok(frames);
            }
        }
    }

    async fn send(&mut self, kind: u8, body: &[u8]) -> TestResult {
        let mut out = Vec::new();
        frame(&mut out, kind, body);
        self.0.write_all(&out).await?;
        self.0.flush().await?;
        Ok(())
    }

    async fn auth(
        &mut self,
        database: &str,
        login: &str,
        password: &str,
    ) -> TestResult<Vec<Frame>> {
        let mut body = 196608u32.to_be_bytes().to_vec();
        for value in [
            "user",
            login,
            "database",
            database,
            "client_encoding",
            "UTF8",
            "",
        ] {
            push_string(&mut body, value);
        }
        self.send_startup(&body).await?;
        let challenge = self.receive().await?;
        assert_eq!(challenge.kind, b'R');
        assert_eq!(challenge.body, 3u32.to_be_bytes());
        let mut secret = Vec::new();
        push_string(&mut secret, password);
        self.send(b'p', &secret).await?;
        let first = self.receive().await?;
        if first.kind == b'E' {
            return Ok(vec![first]);
        }
        assert_eq!(first.kind, b'R');
        assert_eq!(first.body, [0; 4]);
        let mut frames = self.until(b'Z').await?;
        frames.insert(0, first);
        Ok(frames)
    }

    async fn query(&mut self, query: &str) -> TestResult<Vec<Frame>> {
        let mut body = Vec::new();
        push_string(&mut body, query);
        self.send(b'Q', &body).await?;
        self.until(b'Z').await
    }

    async fn write_extended(
        &mut self,
        sql: &str,
        values: &[Option<&str>],
        binary: bool,
        execute: bool,
    ) -> TestResult {
        let mut messages = Vec::new();
        let mut parse = vec![0];
        push_string(&mut parse, sql);
        parse.extend(0u16.to_be_bytes());
        frame(&mut messages, b'P', &parse);

        let mut bind = vec![0, 0];
        bind.extend((values.len() as u16).to_be_bytes());
        bind.resize(bind.len() + values.len() * 2, 0); // pg sends one text format per value
        bind.extend((values.len() as u16).to_be_bytes());
        for value in values {
            match value {
                Some(value) => {
                    bind.extend((value.len() as i32).to_be_bytes());
                    bind.extend(value.as_bytes());
                }
                None => bind.extend((-1i32).to_be_bytes()),
            }
        }
        bind.extend(1u16.to_be_bytes());
        bind.extend(u16::from(binary).to_be_bytes());
        frame(&mut messages, b'B', &bind);
        frame(&mut messages, b'D', b"P\0");
        if execute {
            frame(&mut messages, b'E', &[0; 5]);
            frame(&mut messages, b'S', &[]);
        } else {
            frame(&mut messages, b'H', &[]);
        }
        self.0.write_all(&messages).await?;
        self.0.flush().await?;
        Ok(())
    }
}

fn rows(frames: &[Frame]) -> Vec<Vec<Option<String>>> {
    frames
        .iter()
        .filter(|reply| reply.kind == b'D')
        .map(|reply| {
            let count = u16::from_be_bytes(reply.body[..2].try_into().unwrap());
            let mut body = &reply.body[2..];
            (0..count)
                .map(|_| {
                    let len = i32::from_be_bytes(body[..4].try_into().unwrap());
                    body = &body[4..];
                    if len == -1 {
                        return None;
                    }
                    let (value, next) = body.split_at(len as usize);
                    body = next;
                    Some(std::str::from_utf8(value).unwrap().to_owned())
                })
                .collect()
        })
        .collect()
}

fn texts(frames: &[Frame]) -> Vec<String> {
    rows(frames)
        .into_iter()
        .flatten()
        .map(|value| value.unwrap())
        .collect()
}

fn assert_error(frames: &[Frame], code: &str, severity: &str) {
    let error = frames
        .iter()
        .find(|reply| reply.kind == b'E')
        .unwrap_or_else(|| panic!("{frames:?}"));
    let fields: Vec<_> = error.body.split(|byte| *byte == 0).collect();
    assert!(
        fields.contains(&format!("C{code}").as_bytes()),
        "{frames:?}"
    );
    assert!(
        fields.contains(&format!("S{severity}").as_bytes()),
        "{frames:?}"
    );
}

fn kinds(frames: &[Frame]) -> Vec<u8> {
    frames.iter().map(|reply| reply.kind).collect()
}

async fn seed(client: &mut Client<TcpStream>) -> TestResult {
    let frames = client
        .query(
            "CREATE TABLE departments (id BIGINT NOT NULL, name VARCHAR);
        CREATE TABLE facts (dept BIGINT NOT NULL, amount BIGINT);
        INSERT INTO departments VALUES (1, 'Engine;ering'), (2, 'Sales');
        INSERT INTO facts VALUES (1, 10), (1, 30), (2, 50);",
        )
        .await?;
    assert_eq!(kinds(&frames), b"CCCCZ", "{frames:?}");
    assert_eq!(frames[0].body, b"CREATE TABLE\0");
    assert_eq!(frames[3].body, b"INSERT 0 3\0");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authentication_and_negotiation() -> TestResult {
    let Some(fixture) = Fixture::start().await? else {
        return Ok(());
    };
    let a = fixture.tenant("tenant_a").await?;
    fixture.tenant("tenant_b").await?;
    let listener = fixture.listener(None).await?;
    let mut client = Client(TcpStream::connect(listener.address).await?);
    assert_eq!(client.request(80877104).await?, b'N');
    assert_eq!(client.request(80877103).await?, b'N');
    let connected = client.auth("tenant_a", LOGIN, &a.token).await?;
    assert_eq!(kinds(&connected), b"RSSSSSSSKZ");
    let key = &connected
        .iter()
        .find(|reply| reply.kind == b'K')
        .unwrap()
        .body;
    let pid = u32::from_be_bytes(key[..4].try_into().unwrap());
    assert_eq!(
        texts(&client.query("SELECT pg_backend_pid()").await?),
        [pid.to_string()]
    );

    let mut password_client = listener.connect("tenant_a", PASSWORD).await?;
    assert_eq!(
        texts(&password_client.query("SELECT current_user").await?),
        [LOGIN]
    );
    assert_error(
        &password_client
            .query("CREATE TABLE denied (id BIGINT)")
            .await?,
        "42501",
        "ERROR",
    );
    for (database, login, password) in [
        ("tenant_a", LOGIN, "wrong"),
        ("tenant_b", LOGIN, a.token.as_str()),
        ("tenant_a", "wrong-login", a.token.as_str()),
    ] {
        let mut client = Client(TcpStream::connect(listener.address).await?);
        let frames = client.auth(database, login, password).await?;
        assert_eq!(kinds(&frames), b"E");
        assert_error(&frames, "28P01", "FATAL");
        assert!(frames[0]
            .body
            .windows(b"Mauthentication failed\0".len())
            .any(|window| window == b"Mauthentication failed\0"));
        assert_eq!(client.0.read(&mut [0]).await?, 0);
    }
    let mut cancelled = Client(TcpStream::connect(listener.address).await?);
    let mut cancel = 80877102u32.to_be_bytes().to_vec();
    cancel.extend([0; 8]);
    cancelled.send_startup(&cancel).await?;
    assert_eq!(cancelled.0.read(&mut [0]).await?, 0);
    let mut missing_database = Client(TcpStream::connect(listener.address).await?);
    let mut startup = 196608u32.to_be_bytes().to_vec();
    for value in ["user", LOGIN, ""] {
        push_string(&mut startup, value);
    }
    missing_database.send_startup(&startup).await?;
    assert_error(&[missing_database.receive().await?], "28P01", "FATAL");

    fixture.plane.suspend_tenant(&a.tenant.id).await?;
    assert_error(&client.query("SELECT 1").await?, "28000", "ERROR");
    let mut suspended = Client(TcpStream::connect(listener.address).await?);
    assert_error(
        &suspended.auth("tenant_a", LOGIN, PASSWORD).await?,
        "28P01",
        "FATAL",
    );
    fixture.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simple_query_matches_http_execution_and_meters_each_statement() -> TestResult {
    let Some(fixture) = Fixture::start().await? else {
        return Ok(());
    };
    let tenant = fixture.tenant("tenant_a").await?;
    let listener = fixture.listener(None).await?;
    let mut client = listener.connect("tenant_a", &tenant.token).await?;
    let before = fixture.costs(&tenant).await;
    seed(&mut client).await?;
    let actual = client.query(QUERY).await?;
    assert_eq!(kinds(&actual), b"TDDCZ", "{actual:?}");
    assert_eq!(actual[3].body, b"SELECT 2\0");
    assert_eq!(fixture.costs(&tenant).await, before + 5);
    let result = fixture
        .plane
        .execute_authenticated(&tenant.token, QUERY)
        .await?;
    let http = opensnow_mcp::query_result_to_json(&result);
    assert_eq!(serde_json::json!(rows(&actual)), http["rows"]);
    let updated = client
        .query("UPDATE facts SET amount = amount + 1 WHERE dept = 1")
        .await?;
    assert_eq!(kinds(&updated), b"CZ", "{updated:?}");
    assert_eq!(updated[0].body, b"UPDATE 2\0");
    assert_eq!(fixture.costs(&tenant).await, before + 7);

    let stopped = client
        .query("SELECT 1; COPY facts TO STDOUT; SELECT 2")
        .await?;
    assert_eq!(texts(&stopped), ["1"]);
    assert_error(&stopped, "0A000", "ERROR");
    assert_eq!(fixture.costs(&tenant).await, before + 8);
    fixture.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extended_describe_executes_only_once() -> TestResult {
    let Some(fixture) = Fixture::start().await? else {
        return Ok(());
    };
    let tenant = fixture.tenant("tenant_a").await?;
    let listener = fixture.listener(None).await?;
    let mut client = listener.connect("tenant_a", &tenant.token).await?;
    seed(&mut client).await?;
    let before = fixture.costs(&tenant).await;
    client
        .write_extended(
            "SELECT amount FROM facts WHERE dept = $1 ORDER BY amount",
            &[Some("1")],
            false,
            false,
        )
        .await?;
    let description = client.until(b'T').await?;
    assert_eq!(kinds(&description), b"12T");
    let field = &description[2].body;
    assert_eq!(&field[..9], b"\0\x01amount\0");
    assert_eq!(&field[15..19], &20u32.to_be_bytes());
    assert_eq!(fixture.costs(&tenant).await, before + 1);
    client.send(b'E', &[0; 5]).await?;
    client.send(b'S', &[]).await?;
    let result = client.until(b'Z').await?;
    assert_eq!(kinds(&result), b"DDCZ");
    assert_eq!(texts(&result), ["10", "30"]);
    assert_eq!(fixture.costs(&tenant).await, before + 1);

    client
        .write_extended(
            "SELECT '$2;''quoted' AS literal, $1 AS parameter, $2 AS missing",
            &[Some("a' OR 'b"), None],
            false,
            true,
        )
        .await?;
    let result = client.until(b'Z').await?;
    assert_eq!(kinds(&result), b"12TDCZ", "{result:?}");
    assert_eq!(
        rows(&result),
        [vec![
            Some("$2;'quoted".into()),
            Some("a' OR 'b".into()),
            None
        ]]
    );
    assert_eq!(fixture.costs(&tenant).await, before + 2);

    client
        .write_extended(
            "SELECT amount FROM facts WHERE dept = $1",
            &[Some("-1")],
            false,
            true,
        )
        .await?;
    let empty = client.until(b'Z').await?;
    assert_eq!(kinds(&empty), b"12TCZ", "{empty:?}");
    assert_eq!(empty[3].body, b"SELECT 0\0");
    assert_eq!(fixture.costs(&tenant).await, before + 3);

    let mut parse = b"probe\0".to_vec();
    push_string(&mut parse, "SELECT pg_catalog.format_type($1, -1)");
    parse.extend(0u16.to_be_bytes());
    client.send(b'P', &parse).await?;
    client.send(b'D', b"Sprobe\0").await?;
    client.send(b'C', b"Sprobe\0").await?;
    client.send(b'S', &[]).await?;
    let description = client.until(b'Z').await?;
    assert_eq!(kinds(&description), b"1tT3Z", "{description:?}");
    assert_eq!(description[1].body, [0, 1, 0, 0, 0, 0]);
    assert_eq!(fixture.costs(&tenant).await, before + 3);
    fixture.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_statements_and_error_recovery() -> TestResult {
    let Some(fixture) = Fixture::start().await? else {
        return Ok(());
    };
    let tenant = fixture.tenant("tenant_a").await?;
    let listener = fixture.listener(None).await?;
    let mut client = listener.connect("tenant_a", &tenant.token).await?;
    let before = fixture.costs(&tenant).await;
    let probes = client.query("SET TIME ZONE 'UTC'; SELECT version(); SELECT current_database();
        SHOW TimeZone; SELECT current_schema(); SELECT pg_catalog.set_config('TimeZone', 'UTC', false);
        SELECT pg_catalog.format_type(20, -1)").await?;
    assert_eq!(
        texts(&probes),
        [
            "PostgreSQL 16.0 (opensnow 0.1.0)",
            "tenant_a",
            "UTC",
            tenant.tenant.namespace.as_str(),
            "false",
            "bigint"
        ]
    );
    assert_eq!(fixture.costs(&tenant).await, before);
    assert_eq!(kinds(&client.query(" ; ").await?), b"IZ");
    assert_eq!(client.query("BEGIN").await?.last().unwrap().body, b"T");
    let error = client.query("SHOW default_transaction_isolation").await?;
    assert_error(&error, "0A000", "ERROR");
    assert_eq!(error.last().unwrap().body, b"E");
    assert_error(&client.query("SELECT 1").await?, "25P02", "ERROR");
    assert_eq!(client.query("ROLLBACK").await?.last().unwrap().body, b"I");
    assert_eq!(
        client
            .query("START TRANSACTION")
            .await?
            .last()
            .unwrap()
            .body,
        b"T"
    );
    assert_eq!(client.query("END").await?.last().unwrap().body, b"I");
    client
        .write_extended("SELECT $1", &[Some("1")], true, true)
        .await?;
    let binary = client.until(b'Z').await?;
    assert_eq!(kinds(&binary), b"1EZ");
    assert_error(&binary, "0A000", "ERROR");
    assert_error(
        &client.query("SELECT * FROM pg_catalog.pg_type").await?,
        "0A000",
        "ERROR",
    );
    client.send(b'F', &[]).await?;
    assert_error(&client.until(b'Z').await?, "0A000", "ERROR");
    client.write_extended("SELECT $1", &[], false, true).await?;
    assert_error(&client.until(b'Z').await?, "08P01", "ERROR");
    assert_eq!(texts(&client.query("SELECT 1").await?), ["1"]);
    assert_eq!(fixture.costs(&tenant).await, before + 1);
    fixture.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalogs_are_tenant_scoped_and_unmetered() -> TestResult {
    let Some(fixture) = Fixture::start().await? else {
        return Ok(());
    };
    let a = fixture.tenant("tenant_a").await?;
    let b = fixture.tenant("tenant_b").await?;
    let listener = fixture.listener(None).await?;
    let mut first = listener.connect("tenant_a", &a.token).await?;
    let mut second = listener.connect("tenant_b", &b.token).await?;
    seed(&mut first).await?;
    second.query("CREATE TABLE other (id BIGINT)").await?;
    let before = fixture.costs(&a).await;
    let other_before = fixture.costs(&b).await;
    let names =
        "SELECT table_name FROM information_schema.tables WHERE table_schema = current_schema()
        AND table_name LIKE 'f_ct%' ORDER BY table_name";
    assert_eq!(texts(&first.query(names).await?), ["facts"]);
    let other = second.query(names).await?;
    assert_eq!(kinds(&other), b"TCZ", "{other:?}");
    let classes = "SELECT c.relname, c.oid FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace ORDER BY c.relname";
    assert_eq!(
        texts(&first.query(classes).await?),
        ["departments", "16384", "facts", "16385"]
    );
    assert_eq!(texts(&second.query(classes).await?), ["other", "16384"]);
    let columns = first
        .query(
            "SELECT column_name, is_nullable, data_type FROM information_schema.columns
        WHERE table_name = 'facts' ORDER BY ordinal_position",
        )
        .await?;
    assert_eq!(
        texts(&columns),
        ["dept", "NO", "bigint", "amount", "YES", "bigint"]
    );
    let attrs = first
        .query(
            "SELECT attname, atttypid, attnum FROM pg_catalog.pg_attribute
        WHERE attrelid = 16385 AND attnum > 0 ORDER BY attnum",
        )
        .await?;
    assert_eq!(texts(&attrs), ["dept", "20", "1", "amount", "20", "2"]);
    assert_eq!(fixture.costs(&a).await, before);
    assert_eq!(fixture.costs(&b).await, other_before);
    assert_error(
        &second
            .query(&format!("SELECT * FROM {}.facts", a.tenant.namespace))
            .await?,
        "42501",
        "ERROR",
    );
    assert_error(
        &second
            .query(&format!(
                r#"SELECT * FROM "{}"."facts""#,
                a.tenant.namespace
            ))
            .await?,
        "42501",
        "ERROR",
    );
    assert_error(
        &second.query("SELECT * FROM facts").await?,
        "58000",
        "ERROR",
    );
    assert_eq!(fixture.costs(&b).await, other_before + 1);
    fixture.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_uses_a_verified_certificate() -> TestResult {
    let temp = tempfile::tempdir()?;
    let cert = temp.path().join("cert.pem");
    let key = temp.path().join("key.pem");
    let result = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-keyout",
        ])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .output();
    let result = match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            println!("SKIP pgwire TLS: openssl unavailable");
            return Ok(());
        }
        result => result?,
    };
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let Some(fixture) = Fixture::start().await? else {
        return Ok(());
    };
    let tenant = fixture.tenant("tenant_a").await?;
    let listener = fixture
        .listener(Some(Arc::new(load_tls_config(&cert, &key)?)))
        .await?;
    let mut socket = Client(TcpStream::connect(listener.address).await?);
    assert_eq!(socket.request(80877103).await?, b'S');
    let mut roots = RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut BufReader::new(std::fs::File::open(cert)?)) {
        roots.add(cert?)?;
    }
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect("localhost".try_into()?, socket.0)
        .await?;
    let mut client = Client(stream);
    assert_eq!(
        client
            .auth("tenant_a", LOGIN, &tenant.token)
            .await?
            .last()
            .unwrap()
            .kind,
        b'Z'
    );
    assert_eq!(texts(&client.query("SELECT 42").await?), ["42"]);
    assert_eq!(fixture.costs(&tenant).await, 1);
    fixture.finish().await
}

// Opt-in smoke of the standalone driver proof against the same hosted fixture.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual stock pg driver proof"]
async fn stock_pg_driver() -> TestResult {
    let Some(fixture) = Fixture::start().await? else {
        return Ok(());
    };
    let a = fixture.tenant("tenant_a").await?;
    let listener = fixture.listener(None).await?;
    seed(&mut listener.connect("tenant_a", &a.token).await?).await?;
    let before = fixture.costs(&a).await;
    let uri = format!(
        "postgresql://{LOGIN}:{}@{}/tenant_a?sslmode=disable",
        a.token, listener.address
    );
    let driver = tokio::task::spawn_blocking(move || {
        let script = OsString::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/pgwire-driver-check.mjs"
        ));
        Command::new("node")
            .arg(script)
            .env("OPENSNOW_PG_URL", uri)
            .output()
    })
    .await??;
    assert!(
        driver.status.success(),
        "{}",
        String::from_utf8_lossy(&driver.stderr)
    );
    print!("{}", String::from_utf8_lossy(&driver.stdout));
    assert_eq!(fixture.costs(&a).await, before + 1);
    fixture.finish().await
}
