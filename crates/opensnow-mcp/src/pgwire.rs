//! PostgreSQL v3 transport for the hosted control plane.

mod protocol;
mod sql;

use std::collections::HashMap;
use std::io::{self, BufReader};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use opensnow_control::{ControlPlane, Principal};
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use self::protocol::{cstring, read_body, read_message, send, send_error, Error, Input, Result};
use self::sql::{Prepared, Reply, Shape};

const VERSION: &str = "16.0 (opensnow 0.1.0)";
const PROTOCOL_V3: u32 = 196608;
const CANCEL_REQUEST: [u8; 4] = 80877102u32.to_be_bytes();
const SSL_REQUEST: [u8; 4] = 80877103u32.to_be_bytes();
const GSS_REQUEST: [u8; 4] = 80877104u32.to_be_bytes();
static NEXT_PID: AtomicU32 = AtomicU32::new(1);

pub fn load_tls_config(cert: &Path, key: &Path) -> io::Result<ServerConfig> {
    let mut cert = BufReader::new(std::fs::File::open(cert)?);
    let chain = rustls_pemfile::certs(&mut cert).collect::<io::Result<Vec<_>>>()?;
    let mut key = BufReader::new(std::fs::File::open(key)?);
    let key = rustls_pemfile::private_key(&mut key)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "PEM file contains no private key",
        )
    })?;
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(io::Error::other)
}

pub async fn serve_pg(
    listener: TcpListener,
    plane: Arc<ControlPlane>,
    tls_config: Option<Arc<ServerConfig>>,
) -> io::Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let plane = plane.clone();
        let tls = tls_config.clone();
        tokio::spawn(async move {
            let _ = connection(stream, plane, tls).await;
        });
    }
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

async fn negotiate(
    mut tcp: TcpStream,
    tls: Option<Arc<ServerConfig>>,
) -> io::Result<(Box<dyn Stream>, Vec<u8>, bool)> {
    loop {
        let startup = read_body(&mut tcp).await?;
        match startup.as_slice() {
            request if request == GSS_REQUEST => tcp.write_all(b"N").await?,
            request if request == SSL_REQUEST => {
                if let Some(config) = tls {
                    tcp.write_all(b"S").await?;
                    let mut stream = TlsAcceptor::from(config).accept(tcp).await?;
                    let startup = read_body(&mut stream).await?;
                    return Ok((Box::new(stream), startup, true));
                }
                tcp.write_all(b"N").await?;
            }
            _ => return Ok((Box::new(tcp), startup, false)),
        }
    }
}

async fn connection(
    tcp: TcpStream,
    plane: Arc<ControlPlane>,
    tls: Option<Arc<ServerConfig>>,
) -> Result<()> {
    tcp.set_nodelay(true)?;
    let require_tls = tls.is_some();
    let (mut stream, mut session) = tokio::time::timeout(Duration::from_secs(30), async {
        let (mut stream, startup, encrypted) = negotiate(tcp, tls).await?;
        if startup.starts_with(&CANCEL_REQUEST) {
            return Ok::<_, Error>((stream, None));
        }
        let authenticated = if require_tls && !encrypted {
            Err(Error::auth())
        } else {
            authenticate(&mut *stream, &startup, &plane).await
        };
        match authenticated {
            Ok(session) => Ok((stream, Some(session))),
            Err(_) => {
                send_error(&mut *stream, &Error::auth(), true).await?;
                Ok((stream, None))
            }
        }
    })
    .await
    .map_err(io::Error::other)??;
    if let Some(session) = &mut session {
        session.run(&mut *stream, &plane).await?;
    }
    Ok(())
}

async fn authenticate(
    stream: &mut dyn Stream,
    startup: &[u8],
    plane: &ControlPlane,
) -> Result<Session> {
    let mut input = Input::new(startup);
    if input.u32()? != PROTOCOL_V3 {
        return Err(Error::auth());
    }
    let mut params = HashMap::new();
    loop {
        let key = input.string()?;
        if key.is_empty() {
            break;
        }
        params.insert(key, input.string()?);
    }
    input.end()?;
    let user = params
        .get("user")
        .filter(|s| !s.is_empty())
        .ok_or_else(Error::auth)?;
    let database = params
        .get("database")
        .filter(|s| !s.is_empty())
        .ok_or_else(Error::auth)?;
    let encoding = params.get("client_encoding").copied().unwrap_or("UTF8");
    if !matches!(
        encoding.to_ascii_uppercase().as_str(),
        "UTF8" | "UTF-8" | "UNICODE"
    ) {
        return Err(Error::auth());
    }
    send(stream, b'R', &3u32.to_be_bytes()).await?;
    let (kind, body) = read_message(stream).await?;
    if kind != b'p' {
        return Err(Error::auth());
    }
    let mut input = Input::new(&body);
    let password = input.string()?;
    input.end()?;
    let principal = match plane.authenticate_token(password).await {
        Ok(principal) if principal.tenant_name == *database && principal.login == *user => {
            principal
        }
        _ => {
            plane
                .authenticate_password(database, user, password)
                .await?
        }
    };
    let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
    let session = Session {
        principal,
        pid,
        encoding: encoding.into(),
        transaction: b'I',
        failed_sequence: false,
        statements: HashMap::new(),
        portals: HashMap::new(),
    };
    send(stream, b'R', &0u32.to_be_bytes()).await?;
    for name in [
        "server_version",
        "server_encoding",
        "client_encoding",
        "DateStyle",
        "TimeZone",
        "integer_datetimes",
        "standard_conforming_strings",
    ] {
        let mut body = Vec::new();
        cstring(&mut body, name);
        cstring(&mut body, session.setting(name).unwrap());
        send(stream, b'S', &body).await?;
    }
    let mut key = pid.to_be_bytes().to_vec();
    key.extend(rand::random::<u32>().to_be_bytes());
    send(stream, b'K', &key).await?;
    send(stream, b'Z', b"I").await?;
    Ok(session)
}

struct Portal {
    sql: String,
    reply: Option<Reply>,
    position: usize,
}

struct Session {
    principal: Principal,
    pid: u32,
    encoding: String,
    transaction: u8,
    failed_sequence: bool,
    statements: HashMap<String, Prepared>,
    portals: HashMap<String, Portal>,
}

impl Session {
    fn setting(&self, name: &str) -> Option<&str> {
        Some(match name.to_ascii_lowercase().as_str() {
            "server_version" => VERSION,
            "server_encoding" => "UTF8",
            "client_encoding" => &self.encoding,
            "datestyle" => "ISO, MDY",
            "timezone" => "UTC",
            "integer_datetimes" | "standard_conforming_strings" => "on",
            _ => return None,
        })
    }

    async fn run(&mut self, stream: &mut dyn Stream, plane: &ControlPlane) -> Result<()> {
        loop {
            let (kind, body) = read_message(stream).await?;
            if kind == b'X' {
                return Ok(());
            }
            if self.failed_sequence && kind != b'S' {
                continue;
            }
            let result = if kind == b'Q' {
                self.portals.remove("");
                self.statements.remove("");
                self.simple(stream, plane, &body).await
            } else {
                self.extended(stream, plane, kind, &body).await
            };
            if let Err(error) = result {
                if error.is_io() {
                    return Err(error);
                }
                if self.transaction == b'T' {
                    self.transaction = b'E';
                }
                self.failed_sequence = !matches!(kind, b'Q' | b'S' | b'F' | b'd' | b'c' | b'f');
                send_error(stream, &error, false).await?;
            }
            if matches!(kind, b'Q' | b'S' | b'F') {
                if self.transaction == b'I' && kind != b'F' {
                    self.portals.clear();
                }
                send(stream, b'Z', &[self.transaction]).await?;
            }
        }
    }

    async fn simple(
        &mut self,
        stream: &mut dyn Stream,
        plane: &ControlPlane,
        body: &[u8],
    ) -> Result<()> {
        let mut input = Input::new(body);
        let query = input.string()?;
        input.end()?;
        let (statements, lexed) = sql::split(query);
        let mut empty = true;
        for statement in statements {
            empty = false;
            let reply = sql::execute(self, plane, statement).await?;
            if let Some(fields) = &reply.fields {
                protocol::describe(stream, fields).await?;
            }
            reply.send(stream, 0, 0).await?;
        }
        lexed?;
        if empty {
            send(stream, b'I', &[]).await?;
        }
        Ok(())
    }

    async fn extended(
        &mut self,
        stream: &mut dyn Stream,
        plane: &ControlPlane,
        kind: u8,
        body: &[u8],
    ) -> Result<()> {
        let mut input = Input::new(body);
        match kind {
            b'P' => {
                let name = input.string()?;
                let query = input.string()?;
                let count = input.u16()?;
                for _ in 0..count {
                    if input.u32()? != 0 {
                        return Err(Error::new(
                            "0A000",
                            "explicit parameter types are not supported",
                        ));
                    }
                }
                input.end()?;
                if !name.is_empty() && self.statements.contains_key(name) {
                    return Err(Error::new("42P05", "prepared statement already exists"));
                }
                if name.is_empty() {
                    self.statements.remove("");
                    self.portals.remove("");
                }
                let prepared = Prepared::parse(query, &self.principal)?;
                if usize::from(count) > prepared.parameter_count() {
                    return Err(Error::protocol());
                }
                self.statements.insert(name.into(), prepared);
                send(stream, b'1', &[]).await?;
            }
            b'B' => {
                let name = input.string()?;
                let statement = input.string()?;
                let formats = input.text_formats()?;
                let count = input.u16()?;
                if formats > 1 && formats != count {
                    return Err(Error::protocol());
                }
                let params = (0..count)
                    .map(|_| input.parameter())
                    .collect::<Result<Vec<_>>>()?;
                input.text_formats()?;
                input.end()?;
                let prepared = self
                    .statements
                    .get(statement)
                    .ok_or_else(Error::statement)?;
                let query = prepared.bind(&params)?;
                if !name.is_empty() && self.portals.contains_key(name) {
                    return Err(Error::new("42P03", "portal already exists"));
                }
                self.portals.insert(
                    name.into(),
                    Portal {
                        sql: query,
                        reply: None,
                        position: 0,
                    },
                );
                send(stream, b'2', &[]).await?;
            }
            b'D' => {
                let target = input.byte()?;
                let name = input.string()?;
                input.end()?;
                let fields = match target {
                    b'S' => {
                        let prepared = self.statements.get(name).ok_or_else(Error::statement)?;
                        let count = prepared.parameter_count();
                        let mut params = (count as u16).to_be_bytes().to_vec();
                        params.resize(2 + count * 4, 0);
                        send(stream, b't', &params).await?;
                        match sql::shape(self, prepared.query())? {
                            Shape::Known(fields) => fields,
                            Shape::WarehouseQuery => {
                                return Err(Error::new(
                                    "0A000",
                                    "describe a bound portal to obtain query columns",
                                ))
                            }
                        }
                    }
                    b'P' => {
                        let portal = self.portals.get(name).ok_or_else(Error::portal)?;
                        if let Some(reply) = &portal.reply {
                            reply.fields.clone()
                        } else {
                            match sql::shape(self, &portal.sql)? {
                                Shape::Known(fields) => fields,
                                Shape::WarehouseQuery => {
                                    self.populate(plane, name).await?;
                                    self.portals[name].reply.as_ref().unwrap().fields.clone()
                                }
                            }
                        }
                    }
                    _ => return Err(Error::protocol()),
                };
                match fields {
                    Some(fields) => protocol::describe(stream, &fields).await?,
                    None => send(stream, b'n', &[]).await?,
                }
            }
            b'E' => {
                let name = input.string()?;
                let max_rows = input.u32()?;
                input.end()?;
                self.populate(plane, name).await?;
                let portal = self.portals.get_mut(name).unwrap();
                portal.position = portal
                    .reply
                    .as_ref()
                    .unwrap()
                    .send(stream, portal.position, max_rows)
                    .await?;
            }
            b'C' => {
                let target = input.byte()?;
                let name = input.string()?;
                input.end()?;
                match target {
                    b'S' => {
                        self.statements.remove(name);
                    }
                    b'P' => {
                        self.portals.remove(name);
                    }
                    _ => return Err(Error::protocol()),
                }
                send(stream, b'3', &[]).await?;
            }
            b'S' => {
                input.end()?;
                self.failed_sequence = false;
            }
            b'H' => {
                input.end()?;
                stream.flush().await?;
            }
            b'F' | b'd' | b'c' | b'f' => return Err(Error::unsupported()),
            _ => return Err(Error::protocol()),
        }
        Ok(())
    }

    async fn populate(&mut self, plane: &ControlPlane, name: &str) -> Result<()> {
        let portal = self.portals.get(name).ok_or_else(Error::portal)?;
        if portal.reply.is_some() && self.transaction == b'E' {
            return Err(Error::new(
                "25P02",
                "transaction block is in a failed state",
            ));
        }
        if portal.reply.is_none() {
            let query = portal.sql.clone();
            let reply = sql::execute(self, plane, &query).await?;
            self.portals.get_mut(name).unwrap().reply = Some(reply);
        }
        Ok(())
    }
}
