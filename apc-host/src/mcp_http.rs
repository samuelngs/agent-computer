#[cfg(target_os = "macos")]
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct McpHttpConfig {
    pub host: String,
    pub port: u16,
    pub token: String,
}

#[cfg(target_os = "macos")]
pub fn start_server(socket_path: String, config: McpHttpConfig) -> anyhow::Result<()> {
    validate_listen_host(&config.host)?;
    let listener = TcpListener::bind((config.host.as_str(), config.port))?;
    let local_addr = listener.local_addr()?;
    tracing::info!(
        url = format!("http://{}/mcp", local_addr),
        "MCP HTTP proxy listening"
    );

    std::thread::Builder::new()
        .name("mcp-http".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let socket_path = socket_path.clone();
                        let config = config.clone();
                        let _ = std::thread::Builder::new()
                            .name("mcp-http-client".into())
                            .spawn(move || {
                                if let Err(e) = handle_connection(stream, &socket_path, &config) {
                                    tracing::warn!(%e, "MCP HTTP request failed");
                                }
                            });
                    }
                    Err(e) => tracing::warn!(%e, "MCP HTTP accept failed"),
                }
            }
        })?;

    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn start_server(_socket_path: String, _config: McpHttpConfig) -> anyhow::Result<()> {
    anyhow::bail!("MCP HTTP proxy is only supported on macOS")
}

#[cfg(target_os = "macos")]
fn handle_connection(
    mut stream: TcpStream,
    socket_path: &str,
    config: &McpHttpConfig,
) -> anyhow::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;

    let request = read_http_request(&stream)?;
    if request.path != "/mcp" {
        return send_text(&mut stream, 404, "Not Found", "Not Found");
    }
    if !is_allowed_origin(request.headers.get("origin").map(String::as_str)) {
        return send_text(&mut stream, 403, "Forbidden", "Forbidden");
    }
    if !is_authorized(
        request.headers.get("authorization").map(String::as_str),
        &config.token,
    ) {
        return send_unauthorized(&mut stream);
    }
    if request.method != "POST" {
        return send_with_headers(
            &mut stream,
            405,
            "Method Not Allowed",
            &[("Allow", "POST")],
            b"Method Not Allowed",
            "text/plain; charset=utf-8",
        );
    }
    if !accepts_json(request.headers.get("accept").map(String::as_str)) {
        return send_text(&mut stream, 406, "Not Acceptable", "Not Acceptable");
    }

    let message = match serde_json::from_slice::<serde_json::Value>(&request.body) {
        Ok(value) => value,
        Err(_) => return send_text(&mut stream, 400, "Bad Request", "Invalid JSON"),
    };
    if let Err(message) = validate_http_message(&message) {
        return send_text(&mut stream, 400, "Bad Request", message);
    }

    match forward_http_message(socket_path, message) {
        Ok(Some(response)) => send_json(&mut stream, &response),
        Ok(None) => send_with_headers(
            &mut stream,
            202,
            "Accepted",
            &[],
            b"",
            "text/plain; charset=utf-8",
        ),
        Err(e) => send_text(
            &mut stream,
            502,
            "Bad Gateway",
            &format!("MCP guest proxy failed: {e}"),
        ),
    }
}

#[cfg(target_os = "macos")]
struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[cfg(target_os = "macos")]
fn read_http_request(stream: &TcpStream) -> anyhow::Result<HttpRequest> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        anyhow::bail!("empty HTTP request");
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    if method.is_empty() || path.is_empty() {
        anyhow::bail!("invalid HTTP request line");
    }

    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            anyhow::bail!("unexpected EOF reading HTTP headers");
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    let mut body = vec![0; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

#[cfg(target_os = "macos")]
fn validate_http_message(message: &serde_json::Value) -> Result<(), &'static str> {
    match message {
        serde_json::Value::Object(_) => Ok(()),
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                return Err("Empty JSON-RPC batch");
            }
            if items.iter().all(serde_json::Value::is_object) {
                Ok(())
            } else {
                Err("JSON-RPC batch items must be objects")
            }
        }
        _ => Err("Expected JSON-RPC object or batch array"),
    }
}

#[cfg(target_os = "macos")]
fn forward_http_message(
    socket_path: &str,
    message: serde_json::Value,
) -> anyhow::Result<Option<Vec<u8>>> {
    match message {
        serde_json::Value::Object(_) => forward_one_message(socket_path, message),
        serde_json::Value::Array(items) => {
            let mut responses = Vec::new();
            for item in items {
                if let Some(response) = forward_one_message(socket_path, item)? {
                    let value: serde_json::Value = serde_json::from_slice(&response)?;
                    responses.push(value);
                }
            }

            if responses.is_empty() {
                return Ok(None);
            }
            Ok(Some(serde_json::to_vec(&serde_json::Value::Array(
                responses,
            ))?))
        }
        _ => unreachable!("HTTP message shape is validated before forwarding"),
    }
}

#[cfg(target_os = "macos")]
fn forward_one_message(
    socket_path: &str,
    message: serde_json::Value,
) -> anyhow::Result<Option<Vec<u8>>> {
    if let Some(response) = crate::mcp_capture::try_handle_screen_capture(&message)? {
        return match response {
            crate::mcp_capture::InterceptedResponse::Response(body) => Ok(Some(body)),
            crate::mcp_capture::InterceptedResponse::NoResponse => Ok(None),
        };
    }

    let expect_response = message.get("id").is_some();
    let body = serde_json::to_vec(&message)?;
    forward_to_guest(socket_path, &body, expect_response)
}

#[cfg(target_os = "macos")]
fn forward_to_guest(
    socket_path: &str,
    body: &[u8],
    expect_response: bool,
) -> anyhow::Result<Option<Vec<u8>>> {
    let mut stream = connect_guest(socket_path)?;
    stream.set_read_timeout(Some(Duration::from_secs(180)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(body)?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    if !expect_response {
        return Ok(None);
    }

    let mut response = String::new();
    let mut reader = BufReader::new(stream);
    reader.read_line(&mut response)?;
    if response.is_empty() {
        anyhow::bail!("guest MCP socket closed without response");
    }
    Ok(Some(response.trim_end().as_bytes().to_vec()))
}

#[cfg(target_os = "macos")]
fn connect_guest(socket_path: &str) -> anyhow::Result<UnixStream> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match UnixStream::connect(socket_path) {
            Ok(stream) => return Ok(stream),
            Err(e) if Instant::now() < deadline => {
                tracing::debug!(%e, socket_path, "MCP socket not ready");
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => {
                return Err(anyhow::anyhow!("connect {socket_path}: {e}"));
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn validate_listen_host(host: &str) -> anyhow::Result<()> {
    match host {
        "127.0.0.1" | "localhost" | "::1" | "0.0.0.0" | "::" => Ok(()),
        _ => anyhow::bail!(
            "invalid --mcp-http-host; expected localhost, 127.0.0.1, ::1, 0.0.0.0, or ::"
        ),
    }
}

#[cfg(target_os = "macos")]
fn is_authorized(header: Option<&str>, token: &str) -> bool {
    let Some(header) = header else {
        return false;
    };
    let prefix = "Bearer ";
    header.len() >= prefix.len()
        && header[..prefix.len()].eq_ignore_ascii_case(prefix)
        && &header[prefix.len()..] == token
}

#[cfg(target_os = "macos")]
fn is_allowed_origin(origin: Option<&str>) -> bool {
    let Some(origin) = origin.filter(|value| !value.is_empty()) else {
        return true;
    };
    let Some(after_scheme) = origin.split_once("://").map(|(_, rest)| rest) else {
        return false;
    };
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or(stripped)
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

#[cfg(target_os = "macos")]
fn accepts_json(accept: Option<&str>) -> bool {
    let Some(accept) = accept.filter(|value| !value.is_empty()) else {
        return true;
    };
    accept.split(',').any(|item| {
        let media_type = item.split(';').next().unwrap_or("").trim();
        media_type.eq_ignore_ascii_case("application/json")
            || media_type.eq_ignore_ascii_case("*/*")
            || media_type.eq_ignore_ascii_case("application/*")
    })
}

#[cfg(target_os = "macos")]
fn send_json(stream: &mut TcpStream, body: &[u8]) -> anyhow::Result<()> {
    send_with_headers(stream, 200, "OK", &[], body, "application/json")
}

#[cfg(target_os = "macos")]
fn send_text(stream: &mut TcpStream, status: u16, reason: &str, text: &str) -> anyhow::Result<()> {
    send_with_headers(
        stream,
        status,
        reason,
        &[],
        text.as_bytes(),
        "text/plain; charset=utf-8",
    )
}

#[cfg(target_os = "macos")]
fn send_unauthorized(stream: &mut TcpStream) -> anyhow::Result<()> {
    send_with_headers(
        stream,
        401,
        "Unauthorized",
        &[("WWW-Authenticate", "Bearer")],
        b"Unauthorized",
        "text/plain; charset=utf-8",
    )
}

#[cfg(target_os = "macos")]
fn send_with_headers(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    content_type: &str,
) -> anyhow::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    )?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}
