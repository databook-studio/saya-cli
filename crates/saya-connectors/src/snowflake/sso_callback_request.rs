use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use saya_types::ConnectionError;

use super::{errors, sso_form};

const MAX_HEADERS: usize = 16 * 1024;
const MAX_BODY: usize = 8 * 1024;

pub(crate) async fn read(stream: &mut TcpStream) -> Result<Option<String>, ConnectionError> {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk).await.map_err(|_| errors::auth())?;
        if count == 0 {
            return Ok(None);
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > MAX_HEADERS + MAX_BODY {
            return Err(errors::auth());
        }
        if let Some(end) = bytes.windows(4).position(|item| item == b"\r\n\r\n") {
            if end > MAX_HEADERS {
                return Err(errors::auth());
            }
            break end;
        }
        if bytes.len() > MAX_HEADERS {
            return Err(errors::auth());
        }
    };
    let header_len = header_end + 4;
    let text = std::str::from_utf8(&bytes[..header_end]).map_err(|_| errors::auth())?;
    let mut lines = text.split("\r\n");
    let Some(first) = lines.next() else {
        return Err(errors::auth());
    };
    let mut parts = first.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(errors::auth());
    };
    let method = method.to_owned();
    let target = target.to_owned();
    let version = version.to_owned();
    if version != "HTTP/1.1" || parts.next().is_some() {
        return Err(errors::auth());
    }
    let mut content_length = None;
    let mut content_type = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(errors::auth());
        };
        match name.to_ascii_lowercase().as_str() {
            "content-length" => {
                if content_length.is_some() {
                    return Err(errors::auth());
                }
                content_length = Some(value.trim().parse::<usize>().map_err(|_| errors::auth())?);
            }
            "content-type" => {
                if content_type.is_some() {
                    return Err(errors::auth());
                }
                content_type = Some(value.trim().to_ascii_lowercase());
            }
            "transfer-encoding" => return Err(errors::auth()),
            _ => {}
        }
    }
    let body_len = content_length.unwrap_or(0);
    if body_len > MAX_BODY || method == "GET" && body_len != 0 {
        return Err(errors::auth());
    }
    if method == "POST"
        && !content_type.as_deref().is_some_and(|value| {
            value.split(';').next().unwrap_or("") == "application/x-www-form-urlencoded"
        })
    {
        return Err(errors::auth());
    }
    if bytes.len() > header_len + body_len {
        return Err(errors::auth());
    }
    while bytes.len() < header_len + body_len {
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk).await.map_err(|_| errors::auth())?;
        if count == 0 {
            return Err(errors::auth());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > header_len + body_len {
            return Err(errors::auth());
        }
    }
    let body = &bytes[header_len..header_len + body_len];
    match method.as_str() {
        "GET" => sso_form::token(target.split_once('?').map(|(_, value)| value).unwrap_or("")),
        "POST" => sso_form::token(std::str::from_utf8(body).map_err(|_| errors::auth())?),
        _ => Err(errors::auth()),
    }
    .map(Some)
}

pub(crate) async fn reply(stream: &mut TcpStream, status: u16, message: &str) -> Result<(), ()> {
    let body = format!("<html><body>{message}</body></html>");
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.map_err(|_| ())
}
