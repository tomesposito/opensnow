use std::fmt;
use std::io;

use opensnow_control::ControlError;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::Stream;

const MAX_BODY: u32 = 16 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct Error {
    code: &'static str,
    message: String,
    io: bool,
}

pub(super) type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            io: false,
        }
    }

    pub fn protocol() -> Self {
        Self::new("08P01", "invalid protocol message")
    }
    pub fn auth() -> Self {
        Self::new("28P01", "authentication failed")
    }
    pub fn unsupported() -> Self {
        Self::new("0A000", "not supported by opensnow")
    }
    pub fn statement() -> Self {
        Self::new("26000", "prepared statement does not exist")
    }
    pub fn portal() -> Self {
        Self::new("34000", "portal does not exist")
    }
    pub fn is_io(&self) -> bool {
        self.io
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self {
            io: true,
            ..Self::new("58000", error.to_string())
        }
    }
}

impl From<ControlError> for Error {
    fn from(error: ControlError) -> Self {
        let code = match error {
            ControlError::AuthFailed => "28000",
            ControlError::AccessDenied(_) => "42501",
            ControlError::InvalidInput(_) => "42601",
            ControlError::CostCapExceeded => "53400",
            ControlError::NotFound => "42P01",
            _ => "58000",
        };
        Self::new(code, error.to_string())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

pub(super) struct Input<'a>(&'a [u8]);

impl<'a> Input<'a> {
    pub fn new(body: &'a [u8]) -> Self {
        Self(body)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let (part, rest) = self.0.split_at_checked(count).ok_or_else(Error::protocol)?;
        self.0 = rest;
        Ok(part)
    }

    pub fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn string(&mut self) -> Result<&'a str> {
        let len = self
            .0
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(Error::protocol)?;
        std::str::from_utf8(&self.take(len + 1)?[..len]).map_err(|_| Error::protocol())
    }

    pub fn parameter(&mut self) -> Result<Option<&'a str>> {
        let len = self.u32()? as i32;
        if len == -1 {
            return Ok(None);
        }
        let len = usize::try_from(len).map_err(|_| Error::protocol())?;
        let value = self.take(len)?;
        if value.contains(&0) {
            return Err(Error::protocol());
        }
        Ok(Some(
            std::str::from_utf8(value).map_err(|_| Error::protocol())?,
        ))
    }

    pub fn end(&self) -> Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Error::protocol())
        }
    }

    pub fn text_formats(&mut self) -> Result<u16> {
        let count = self.u16()?;
        for _ in 0..count {
            match self.u16()? {
                0 => (),
                1 => return Err(Error::new("0A000", "binary format is not supported")),
                _ => return Err(Error::protocol()),
            }
        }
        Ok(count)
    }
}

pub(super) async fn read_body(
    stream: &mut (impl AsyncRead + Unpin + ?Sized),
) -> io::Result<Vec<u8>> {
    let length = stream.read_u32().await?;
    let size = length
        .checked_sub(4)
        .filter(|size| *size <= MAX_BODY)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid pgwire frame length"))?;
    let mut bytes = vec![0; size as usize];
    stream.read_exact(&mut bytes).await?;
    Ok(bytes)
}

pub(super) async fn read_message(stream: &mut dyn Stream) -> io::Result<(u8, Vec<u8>)> {
    let kind = stream.read_u8().await?;
    Ok((kind, read_body(stream).await?))
}

pub(super) fn cstring(out: &mut Vec<u8>, value: &str) {
    out.extend(value.as_bytes().iter().copied().filter(|byte| *byte != 0));
    out.push(0);
}

pub(super) async fn send(stream: &mut dyn Stream, kind: u8, body: &[u8]) -> Result<()> {
    let length = u32::try_from(body.len() + 4).map_err(|_| Error::protocol())?;
    let mut header = [kind, 0, 0, 0, 0];
    header[1..].copy_from_slice(&length.to_be_bytes());
    stream.write_all(&header).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

pub(super) async fn send_error(stream: &mut dyn Stream, error: &Error, fatal: bool) -> Result<()> {
    let mut body = vec![b'S'];
    cstring(&mut body, if fatal { "FATAL" } else { "ERROR" });
    body.push(b'C');
    cstring(&mut body, error.code);
    body.push(b'M');
    cstring(&mut body, &error.message);
    body.push(0);
    send(stream, b'E', &body).await
}

pub(super) type Field = (String, u32);

pub(super) async fn describe(stream: &mut dyn Stream, fields: &[Field]) -> Result<()> {
    let count = u16::try_from(fields.len()).map_err(|_| Error::protocol())?;
    let mut body = count.to_be_bytes().to_vec();
    for (name, oid) in fields {
        let width: i16 = match oid {
            16 => 1,
            21 => 2,
            23 | 26 | 700 | 1082 => 4,
            20 | 701 | 1083 | 1114 => 8,
            _ => -1,
        };
        cstring(&mut body, name);
        body.extend([0; 6]); // table OID and column number are unspecified
        body.extend(oid.to_be_bytes());
        body.extend(width.to_be_bytes());
        body.extend((-1i32).to_be_bytes());
        body.extend([0; 2]); // text format
    }
    send(stream, b'T', &body).await
}

pub(super) async fn row(stream: &mut dyn Stream, values: Vec<Option<String>>) -> Result<()> {
    let count = u16::try_from(values.len()).map_err(|_| Error::protocol())?;
    let mut body = count.to_be_bytes().to_vec();
    for value in values {
        match value {
            None => body.extend((-1i32).to_be_bytes()),
            Some(value) => {
                let length = i32::try_from(value.len()).map_err(|_| Error::protocol())?;
                body.extend(length.to_be_bytes());
                body.extend(value.as_bytes());
            }
        }
    }
    send(stream, b'D', &body).await
}
