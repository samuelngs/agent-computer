use std::io::{BufRead, BufReader, BufWriter, Write};
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

fn set_blocking(fd: i32) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 && (flags & libc::O_NONBLOCK) != 0 {
            libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
        }
    }
}

pub fn run_stdio_proxy(socket_path: &str) -> anyhow::Result<()> {
    let stream = connect_with_retry(socket_path)?;
    let mut sock_reader = BufReader::new(stream.try_clone()?);
    let mut sock_writer = BufWriter::new(stream);

    // libkrun sets inherited fds non-blocking. Dup AFTER VM start (connect_with_retry
    // waits for the VM), then force blocking mode on the new fds.
    let stdin_fd = unsafe { libc::dup(libc::STDIN_FILENO) };
    let stdout_fd = unsafe { libc::dup(libc::STDOUT_FILENO) };
    if stdin_fd < 0 || stdout_fd < 0 {
        anyhow::bail!("failed to dup stdin/stdout");
    }
    set_blocking(stdin_fd);
    set_blocking(stdout_fd);

    let stdin_file = unsafe { std::fs::File::from_raw_fd(stdin_fd) };
    let stdin = BufReader::new(stdin_file);
    let stdout_file = unsafe { std::fs::File::from_raw_fd(stdout_fd) };
    let mut stdout = BufWriter::new(stdout_file);

    for line_result in stdin.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("failed to read from stdin: {e}");
                break;
            }
        };

        let parsed_message = serde_json::from_str::<serde_json::Value>(&line).ok();
        let is_notification = parsed_message
            .as_ref()
            .map(|v| v.get("id").is_none())
            .unwrap_or(false);

        if let Some(message) = parsed_message.as_ref() {
            match crate::mcp_capture::try_handle_screen_capture(message) {
                Ok(Some(crate::mcp_capture::InterceptedResponse::Response(response))) => {
                    if let Err(e) = stdout
                        .write_all(&response)
                        .and_then(|_| stdout.write_all(b"\n"))
                        .and_then(|_| stdout.flush())
                    {
                        tracing::error!("failed to write to stdout: {e}");
                        break;
                    }
                    continue;
                }
                Ok(Some(crate::mcp_capture::InterceptedResponse::NoResponse)) => continue,
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(%e, "host screen capture failed; forwarding to guest");
                }
            }
        }

        if let Err(e) = sock_writer
            .write_all(line.as_bytes())
            .and_then(|_| sock_writer.write_all(b"\n"))
            .and_then(|_| sock_writer.flush())
        {
            tracing::error!("failed to write to socket: {e}");
            break;
        }

        if is_notification {
            continue;
        }

        let mut response = String::new();
        match sock_reader.read_line(&mut response) {
            Ok(0) => {
                tracing::info!("socket closed by guest");
                break;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("failed to read from socket: {e}");
                break;
            }
        }

        if let Err(e) = stdout
            .write_all(response.as_bytes())
            .and_then(|_| {
                if !response.ends_with('\n') {
                    stdout.write_all(b"\n")
                } else {
                    Ok(())
                }
            })
            .and_then(|_| stdout.flush())
        {
            tracing::error!("failed to write to stdout: {e}");
            break;
        }
    }

    tracing::info!("stdio proxy shutting down");
    Ok(())
}

/// Connect to the Unix socket, retrying every 2 seconds for up to 60 seconds.
fn connect_with_retry(socket_path: &str) -> anyhow::Result<UnixStream> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;
        tracing::info!(attempt, socket_path, "connecting to MCP socket");

        match UnixStream::connect(socket_path) {
            Ok(stream) => {
                tracing::info!(socket_path, "MCP stdio proxy connected");
                return Ok(stream);
            }
            Err(e) => {
                if Instant::now() >= deadline {
                    return Err(anyhow::anyhow!(
                        "failed to connect to {socket_path} after {attempt} attempts: {e}"
                    ));
                }
                tracing::info!(attempt, "connection failed ({e}), retrying in 2s");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}
