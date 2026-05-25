#[cfg(target_os = "macos")]
use std::io::{BufRead, BufReader, Write};
#[cfg(target_os = "macos")]
use std::os::unix::net::UnixStream;

#[cfg(target_os = "macos")]
use agentos_protocol::{JsonRpcRequest, JsonRpcResponse};

#[cfg(target_os = "macos")]
pub struct McpClient {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

#[cfg(target_os = "macos")]
impl McpClient {
    pub fn connect(socket_path: &str) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket_path)?;
        let reader = BufReader::new(stream.try_clone()?);
        tracing::info!(socket_path, "MCP client connected");
        Ok(Self {
            writer: stream,
            reader,
        })
    }

    pub fn call(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<JsonRpcResponse> {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::Value::Number(
                ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed).into(),
            ),
            method: method.into(),
            params,
        };

        let mut data = serde_json::to_vec(&request)?;
        data.push(b'\n');
        self.writer.write_all(&data)?;
        self.writer.flush()?;

        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        let response: JsonRpcResponse = serde_json::from_str(&line)?;
        Ok(response)
    }
}

#[cfg(target_os = "macos")]
pub fn run_mcp_test(socket_path: &str) {
    let path = socket_path.to_string();
    std::thread::Builder::new()
        .name("mcp-test".into())
        .spawn(move || {
            for attempt in 0..12 {
                std::thread::sleep(std::time::Duration::from_secs(5));
                tracing::info!(attempt = attempt + 1, "MCP test: connecting");
                match McpClient::connect(&path) {
                    Ok(mut client) => {
                        run_test_suite(&mut client);
                        return;
                    }
                    Err(e) => tracing::info!(attempt = attempt + 1, "MCP connect failed: {e}"),
                }
            }
            tracing::error!("MCP test: gave up after 60s");
        })
        .expect("failed to spawn MCP test thread");
}

#[cfg(target_os = "macos")]
fn run_one(client: &mut McpClient, name: &str, params: &serde_json::Value) {
    match client.call("tool_call", Some(params.clone())) {
        Ok(resp) => {
            if let Some(result) = &resp.result {
                tracing::info!(test = name, result = %serde_json::to_string(result).unwrap_or_default(), "ok");
            } else if let Some(error) = &resp.error {
                tracing::error!(test = name, code = error.code, msg = %error.message, "failed");
            }
        }
        Err(e) => tracing::error!(test = name, "transport error: {e}"),
    }
}

#[cfg(target_os = "macos")]
fn run_test_suite(client: &mut McpClient) {
    tracing::info!("starting MCP test suite");

    let tests: &[(&str, serde_json::Value)] = &[
        ("shell_exec hostname", serde_json::json!({"tool": "shell_exec", "params": {"cmd": "hostname"}})),
        ("shell_exec uname", serde_json::json!({"tool": "shell_exec", "params": {"cmd": "uname -a"}})),
        ("window_list", serde_json::json!({"tool": "window_list"})),
        ("mouse_move", serde_json::json!({"tool": "mouse_move", "params": {"x": 500, "y": 300}})),
        ("screen_capture", serde_json::json!({"tool": "screen_capture", "params": {}})),
        ("window_open foot", serde_json::json!({"tool": "window_open", "params": {"cmd": "foot"}})),
    ];

    for (name, params) in tests {
        run_one(client, name, params);
    }

    std::thread::sleep(std::time::Duration::from_secs(2));

    let post_tests: &[(&str, serde_json::Value)] = &[
        ("window_list post", serde_json::json!({"tool": "window_list"})),
        ("window_move", serde_json::json!({"tool": "window_move", "params": {"id": 0, "x": 100, "y": 50}})),
        ("window_resize", serde_json::json!({"tool": "window_resize", "params": {"id": 0, "width": 800, "height": 600}})),
        ("window_focus", serde_json::json!({"tool": "window_focus", "params": {"id": 0}})),
        ("mouse_click left", serde_json::json!({"tool": "mouse_click", "params": {"button": "left"}})),
        ("file_write", serde_json::json!({"tool": "file_write", "params": {"path": "/tmp/mcp_test.txt", "data": [104,101,108,108,111]}})),
        ("file_read", serde_json::json!({"tool": "file_read", "params": {"path": "/tmp/mcp_test.txt"}})),
    ];

    for (name, params) in post_tests {
        run_one(client, name, params);
    }

    tracing::info!("MCP test suite complete");
}
