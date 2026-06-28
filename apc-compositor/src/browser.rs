#[cfg(target_os = "linux")]
use anyhow::{Context, Result, anyhow};

#[cfg(target_os = "linux")]
use serde_json::json;

#[cfg(target_os = "linux")]
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    os::{unix::fs::PermissionsExt, unix::net::UnixStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
enum BrowserCommand {
    Call {
        name: String,
        arguments: serde_json::Value,
        reply: mpsc::SyncSender<serde_json::Value>,
    },
}

#[cfg(target_os = "linux")]
pub(crate) struct BrowserService {
    tx: mpsc::Sender<BrowserCommand>,
}

#[cfg(target_os = "linux")]
impl BrowserService {
    pub(crate) fn new(wayland_display: String) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || BrowserWorker::new(wayland_display).run(rx));
        Self { tx }
    }

    pub(crate) fn handle_tool_async(
        &self,
        name: String,
        arguments: serde_json::Value,
        reply: mpsc::SyncSender<serde_json::Value>,
    ) {
        let fallback_reply = reply.clone();
        if self
            .tx
            .send(BrowserCommand::Call {
                name,
                arguments,
                reply,
            })
            .is_err()
        {
            let _ = fallback_reply.send(text_result("browser service is not available", true));
        }
    }
}

#[cfg(target_os = "linux")]
struct BrowserWorker {
    wayland_display: String,
    xdg_runtime_dir: PathBuf,
    runtime_dir: PathBuf,
    profile_dir: PathBuf,
    socket_path: PathBuf,
    browserd_bin: PathBuf,
    child: Option<Child>,
    connection: Option<BrowserConnection>,
}

#[cfg(target_os = "linux")]
impl BrowserWorker {
    fn new(wayland_display: String) -> Self {
        let uid = unsafe { libc::geteuid() };
        let xdg_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")));
        let runtime_dir = xdg_runtime_dir.join("apc-browserd");
        let profile_dir = PathBuf::from("/home/agent/.config/apc-browserd/profile");
        let socket_path = runtime_dir.join("browserd.sock");
        let browserd_bin = std::env::var_os("APC_BROWSERD_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/opt/apc-browserd/browserd"));

        Self {
            wayland_display,
            xdg_runtime_dir,
            runtime_dir,
            profile_dir,
            socket_path,
            browserd_bin,
            child: None,
            connection: None,
        }
    }

    fn run(mut self, rx: mpsc::Receiver<BrowserCommand>) {
        while let Ok(command) = rx.recv() {
            match command {
                BrowserCommand::Call {
                    name,
                    arguments,
                    reply,
                } => {
                    let result = self.handle_call(&name, arguments);
                    let _ = reply.send(result);
                }
            }
        }
    }

    fn handle_call(&mut self, name: &str, arguments: serde_json::Value) -> serde_json::Value {
        if name == "browser_tab_list" && !self.is_browserd_running() {
            return text_result("No tabs open", false);
        }

        let browserd_running = self.is_browserd_running();
        if !browserd_running && !matches!(name, "browser_navigate" | "browser_tab_new") {
            return text_result("No browser tabs open; open a tab first", true);
        }

        let startup_url =
            if !browserd_running && matches!(name, "browser_navigate" | "browser_tab_new") {
                arguments
                    .get("url")
                    .and_then(|url| url.as_str())
                    .map(str::to_string)
            } else {
                None
            };

        match self.call_tool(name, arguments, startup_url.as_deref()) {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(tool = name, %e, "browserd tool call failed");
                self.connection = None;
                text_result(format!("browserd tool call failed: {e}"), true)
            }
        }
    }

    fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
        startup_url: Option<&str>,
    ) -> Result<serde_json::Value> {
        let cold_start = !self.is_browserd_running();
        self.ensure_browserd_ready(startup_url)?;
        if cold_start {
            return self.rpc(
                "tools/call",
                Some(json!({
                    "name": "browser_tab_list",
                    "arguments": {},
                })),
                Duration::from_secs(180),
            );
        }

        self.rpc(
            "tools/call",
            Some(json!({
                "name": name,
                "arguments": arguments,
            })),
            Duration::from_secs(180),
        )
    }

    fn ensure_browserd_ready(&mut self, startup_url: Option<&str>) -> Result<()> {
        if self.connection.is_some() && self.is_browserd_running() {
            return Ok(());
        }

        self.connection = None;
        if !self.is_browserd_running() {
            self.start_browserd(startup_url)?;
        }

        self.connect_ipc(Duration::from_secs(30))?;
        self.initialize_mcp()?;
        Ok(())
    }

    fn is_browserd_running(&mut self) -> bool {
        let Some(child) = self.child.as_mut() else {
            return false;
        };

        match child.try_wait() {
            Ok(Some(status)) => {
                tracing::warn!(%status, "browserd exited");
                self.child = None;
                self.connection = None;
                false
            }
            Ok(None) => true,
            Err(e) => {
                tracing::warn!(%e, "failed to poll browserd process");
                self.child = None;
                self.connection = None;
                false
            }
        }
    }

    fn start_browserd(&mut self, startup_url: Option<&str>) -> Result<()> {
        if !self.browserd_bin.exists() {
            return Err(anyhow!(
                "browserd binary not found at {}",
                self.browserd_bin.display()
            ));
        }

        ensure_private_dir(&self.runtime_dir)?;
        ensure_private_dir(&self.profile_dir)?;

        let log_path = self.runtime_dir.join("browserd.log");
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map(Stdio::from)
            .unwrap_or_else(|_| Stdio::null());

        let mut command = Command::new(&self.browserd_bin);
        command
            .arg("--gui")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg(format!("--mcp-ipc-path={}", self.socket_path.display()))
            .arg(format!("--user-data-dir={}", self.profile_dir.display()))
            .current_dir(
                self.browserd_bin
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("/opt/apc-browserd")),
            )
            .env("HOME", "/home/agent")
            .env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
            .env("WAYLAND_DISPLAY", &self.wayland_display)
            .env("XDG_RUNTIME_DIR", &self.xdg_runtime_dir)
            .env("XDG_SESSION_TYPE", "wayland")
            .env("LD_LIBRARY_PATH", browserd_library_path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr);

        if std::path::Path::new("/dev/dri/renderD128").exists() {
            command.arg("--render-node-override=/dev/dri/renderD128");
        } else if std::path::Path::new("/dev/dri/card0").exists() {
            command.arg("--render-node-override=/dev/dri/card0");
        }
        if let Some(url) = startup_url {
            command.arg(url);
        } else {
            command.arg("about:blank");
        }

        tracing::info!(
            binary = %self.browserd_bin.display(),
            socket = %self.socket_path.display(),
            wayland_display = %self.wayland_display,
            startup_url = startup_url.unwrap_or("about:blank"),
            "starting browserd"
        );
        let child = command.spawn().context("spawn browserd")?;
        self.child = Some(child);
        Ok(())
    }

    fn connect_ipc(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut last_error = None;

        while Instant::now() < deadline {
            if !self.is_browserd_running() {
                return Err(anyhow!("browserd exited before IPC became ready"));
            }

            match UnixStream::connect(&self.socket_path) {
                Ok(stream) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(180)))
                        .context("set browserd IPC read timeout")?;
                    stream
                        .set_write_timeout(Some(Duration::from_secs(10)))
                        .context("set browserd IPC write timeout")?;
                    let writer = stream.try_clone().context("clone browserd IPC stream")?;
                    self.connection = Some(BrowserConnection {
                        reader: BufReader::new(stream),
                        writer,
                        next_id: 0,
                    });
                    return Ok(());
                }
                Err(e) => {
                    last_error = Some(e);
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }

        Err(anyhow!(
            "timed out connecting to browserd IPC at {}: {}",
            self.socket_path.display(),
            last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "socket not available".to_string())
        ))
    }

    fn initialize_mcp(&mut self) -> Result<()> {
        self.rpc(
            "initialize",
            Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "apc-compositor",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
            Duration::from_secs(20),
        )?;
        self.notify("notifications/initialized", None)
    }

    fn rpc(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        let Some(connection) = self.connection.as_mut() else {
            return Err(anyhow!("browserd IPC is not connected"));
        };

        connection
            .reader
            .get_ref()
            .set_read_timeout(Some(timeout))
            .context("set browserd RPC timeout")?;
        connection.next_id += 1;
        let id = serde_json::Value::from(connection.next_id);
        let mut request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if let Some(params) = params {
            request["params"] = params;
        }

        write_json_line(&mut connection.writer, &request)?;

        let mut line = String::new();
        loop {
            line.clear();
            let read = connection
                .reader
                .read_line(&mut line)
                .with_context(|| format!("read browserd response for {method}"))?;
            if read == 0 {
                return Err(anyhow!("browserd IPC closed while waiting for {method}"));
            }

            let response: serde_json::Value = serde_json::from_str(line.trim_end())
                .with_context(|| format!("parse browserd response for {method}"))?;
            if response.get("id") != Some(&id) {
                tracing::debug!(method, response = %response, "ignoring unmatched browserd response");
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(anyhow!(
                    "{}",
                    error
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("browserd JSON-RPC error")
                ));
            }
            return Ok(response
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null));
        }
    }

    fn notify(&mut self, method: &str, params: Option<serde_json::Value>) -> Result<()> {
        let Some(connection) = self.connection.as_mut() else {
            return Err(anyhow!("browserd IPC is not connected"));
        };

        let mut notification = json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        if let Some(params) = params {
            notification["params"] = params;
        }
        write_json_line(&mut connection.writer, &notification)
    }
}

#[cfg(target_os = "linux")]
impl Drop for BrowserWorker {
    fn drop(&mut self) {
        self.connection = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}

#[cfg(target_os = "linux")]
struct BrowserConnection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

#[cfg(target_os = "linux")]
fn ensure_private_dir(path: &PathBuf) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    let mut permissions = fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("chmod 0700 {}", path.display()))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn browserd_library_path() -> String {
    let base = "/opt/apc-browserd:/opt/apc-browserd/lib";
    match std::env::var("LD_LIBRARY_PATH") {
        Ok(existing) if !existing.is_empty() => format!("{base}:{existing}"),
        _ => base.to_string(),
    }
}

#[cfg(target_os = "linux")]
fn write_json_line(stream: &mut UnixStream, value: &serde_json::Value) -> Result<()> {
    let mut data = serde_json::to_vec(value).context("serialize browserd request")?;
    data.push(b'\n');
    stream.write_all(&data).context("write browserd request")?;
    stream.flush().context("flush browserd request")
}

#[cfg(target_os = "linux")]
fn text_result(text: impl Into<String>, is_error: bool) -> serde_json::Value {
    let mut result = json!({
        "content": [{
            "type": "text",
            "text": text.into(),
        }]
    });
    if is_error {
        result["isError"] = serde_json::Value::Bool(true);
    }
    result
}
