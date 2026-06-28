#[cfg(target_os = "linux")]
use anyhow::Result;
#[cfg(target_os = "linux")]
use apc_protocol::{JsonRpcRequest, JsonRpcResponse, ToolCall, VSOCK_PORT};

#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader, Write};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(target_os = "linux")]
use std::sync::mpsc;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use calloop::channel;
#[cfg(target_os = "linux")]
use calloop::generic::Generic;
#[cfg(target_os = "linux")]
use calloop::{Interest, Mode, PostAction};

#[cfg(target_os = "linux")]
pub enum McpCommand {
    Tool {
        tool: ToolCall,
        reply: mpsc::SyncSender<serde_json::Value>,
    },
    BrowserTool {
        name: String,
        arguments: serde_json::Value,
        reply: mpsc::SyncSender<serde_json::Value>,
    },
}

#[cfg(target_os = "linux")]
pub struct McpServer {
    _listener_fd: OwnedFd,
}

#[cfg(target_os = "linux")]
pub fn start(
    loop_handle: calloop::LoopHandle<'static, super::state::CalloopData>,
) -> Result<(McpServer, calloop::channel::Sender<McpCommand>)> {
    let listener_fd = vsock_listen(VSOCK_PORT)?;
    tracing::info!(port = VSOCK_PORT, "MCP vsock listener bound");

    let raw_fd = listener_fd.as_raw_fd();

    let (cmd_tx, cmd_rx) = channel::channel::<McpCommand>();

    loop_handle
        .insert_source(cmd_rx, |event, _, data| {
            if let channel::Event::Msg(cmd) = event {
                match cmd {
                    McpCommand::Tool { tool, reply } => {
                        let result = super::mcp_dispatch::handle_mcp_tool(
                            &mut data.state,
                            &mut data.display,
                            tool,
                            reply.clone(),
                        );
                        if let Some(value) = result {
                            let _ = reply.send(value);
                        }
                    }
                    McpCommand::BrowserTool {
                        name,
                        arguments,
                        reply,
                    } => {
                        super::mcp_dispatch::handle_browser_tool(
                            &mut data.state,
                            name,
                            arguments,
                            reply,
                        );
                    }
                }
            }
        })
        .map_err(|e| anyhow::anyhow!("mcp channel source: {}", e.error))?;

    let cmd_tx_clone = cmd_tx.clone();

    let source = Generic::new(
        unsafe { OwnedFd::from_raw_fd(raw_fd) },
        Interest::READ,
        Mode::Level,
    );

    loop_handle
        .insert_source(source, move |_, fd, _data| {
            let conn_fd = match vsock_accept(fd.as_raw_fd()) {
                Ok(fd) => fd,
                Err(e) => {
                    tracing::error!("vsock accept failed: {e}");
                    return Ok(PostAction::Continue);
                }
            };
            tracing::info!("MCP client connected via vsock");
            let tx = cmd_tx_clone.clone();
            std::thread::spawn(move || {
                if let Err(e) = handle_connection(conn_fd, tx) {
                    tracing::warn!("MCP connection ended: {e}");
                }
            });
            Ok(PostAction::Continue)
        })
        .map_err(|e| anyhow::anyhow!("mcp vsock source: {}", e.error))?;

    std::mem::forget(listener_fd);

    Ok((
        McpServer {
            _listener_fd: unsafe { OwnedFd::from_raw_fd(raw_fd) },
        },
        cmd_tx,
    ))
}

#[cfg(target_os = "linux")]
fn vsock_listen(port: u32) -> Result<OwnedFd> {
    unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(anyhow::anyhow!(
                "socket(AF_VSOCK) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let fd = OwnedFd::from_raw_fd(fd);

        let mut addr: libc::sockaddr_vm = std::mem::zeroed();
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_cid = libc::VMADDR_CID_ANY;
        addr.svm_port = port;

        if libc::bind(
            fd.as_raw_fd(),
            &addr as *const libc::sockaddr_vm as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
        ) < 0
        {
            return Err(anyhow::anyhow!(
                "bind vsock port {port}: {}",
                std::io::Error::last_os_error()
            ));
        }

        if libc::listen(fd.as_raw_fd(), 4) < 0 {
            return Err(anyhow::anyhow!(
                "listen: {}",
                std::io::Error::last_os_error()
            ));
        }

        let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);

        Ok(fd)
    }
}

#[cfg(target_os = "linux")]
fn vsock_accept(listener_fd: i32) -> Result<OwnedFd> {
    unsafe {
        let fd = libc::accept(listener_fd, std::ptr::null_mut(), std::ptr::null_mut());
        if fd < 0 {
            return Err(anyhow::anyhow!(
                "accept: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(OwnedFd::from_raw_fd(fd))
    }
}

#[cfg(target_os = "linux")]
fn handle_connection(fd: OwnedFd, cmd_tx: channel::Sender<McpCommand>) -> Result<()> {
    let raw = fd.as_raw_fd();
    std::mem::forget(fd);
    let stream = unsafe { std::net::TcpStream::from_raw_fd(raw) };
    if let Err(e) = stream.set_write_timeout(Some(Duration::from_secs(5))) {
        tracing::warn!(%e, "failed to set MCP connection write timeout");
    }
    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let err = JsonRpcResponse::error(
                    serde_json::Value::Null,
                    -32700,
                    format!("parse error: {e}"),
                );
                let mut resp = serde_json::to_vec(&err)?;
                resp.push(b'\n');
                writer.write_all(&resp)?;
                continue;
            }
        };

        tracing::info!(method = %request.method, "MCP request");
        let response = handle_request(&request, &cmd_tx);

        if let Some(resp) = response {
            let mut buf = serde_json::to_vec(&resp)?;
            buf.push(b'\n');
            writer.write_all(&buf)?;
            writer.flush()?;
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn handle_request(
    request: &JsonRpcRequest,
    cmd_tx: &channel::Sender<McpCommand>,
) -> Option<JsonRpcResponse> {
    match request.method.as_str() {
        "initialize" => Some(JsonRpcResponse::success(
            request.id.clone(),
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "apc",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )),

        "notifications/initialized" => None,

        "tools/list" => Some(JsonRpcResponse::success(
            request.id.clone(),
            serde_json::json!({
                "tools": apc_protocol::mcp_tool_schemas()
            }),
        )),

        "tools/call" => {
            let params = request.params.clone().unwrap_or(serde_json::Value::Null);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::json!({}));

            if apc_protocol::is_browser_tool_name(name) {
                return Some(dispatch_browser_tool(
                    request.id.clone(),
                    name.to_string(),
                    arguments,
                    cmd_tx,
                ));
            }

            match apc_protocol::toolcall_from_mcp(name, &arguments) {
                Ok(call) => {
                    let rpc = dispatch_tool(request.id.clone(), call, cmd_tx);
                    Some(toolcall_result_to_mcp(rpc))
                }
                Err(e) => Some(JsonRpcResponse::error(request.id.clone(), -32602, e)),
            }
        }

        _ => Some(JsonRpcResponse::error(
            request.id.clone(),
            -32601,
            format!("method not found: {}", request.method),
        )),
    }
}

#[cfg(target_os = "linux")]
fn toolcall_result_to_mcp(rpc: JsonRpcResponse) -> JsonRpcResponse {
    if let Some(err) = &rpc.error {
        JsonRpcResponse::success(
            rpc.id,
            serde_json::json!({
                "content": [{ "type": "text", "text": err.message }],
                "isError": true
            }),
        )
    } else {
        let result = rpc.result.unwrap_or(serde_json::Value::Null);
        if let Some(error) = result.get("error").and_then(|v| v.as_str()) {
            return JsonRpcResponse::success(
                rpc.id,
                serde_json::json!({
                    "content": [{ "type": "text", "text": error }],
                    "isError": true
                }),
            );
        }

        let content = if result.get("format").is_some() {
            if let Some(b64) = result.get("data").and_then(|d| d.as_str()) {
                serde_json::json!([{ "type": "image", "data": b64, "mimeType": "image/png" }])
            } else {
                serde_json::json!([{ "type": "text", "text": serde_json::to_string(&result).unwrap_or_default() }])
            }
        } else {
            serde_json::json!([{ "type": "text", "text": serde_json::to_string(&result).unwrap_or_default() }])
        };
        JsonRpcResponse::success(rpc.id, serde_json::json!({ "content": content }))
    }
}

#[cfg(target_os = "linux")]
fn dispatch_tool(
    id: serde_json::Value,
    tool: ToolCall,
    cmd_tx: &channel::Sender<McpCommand>,
) -> JsonRpcResponse {
    let timeout = match &tool {
        ToolCall::ShellExec { .. } => std::time::Duration::from_secs(300),
        ToolCall::FsMount { .. } => std::time::Duration::from_secs(30),
        _ => std::time::Duration::from_secs(5),
    };
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    let cmd = McpCommand::Tool {
        tool,
        reply: reply_tx,
    };
    if cmd_tx.send(cmd).is_err() {
        return JsonRpcResponse::error(id, -32000, "compositor channel closed");
    }
    match reply_rx.recv_timeout(timeout) {
        Ok(result) => JsonRpcResponse::success(id, result),
        Err(_) => JsonRpcResponse::error(id, -32000, "compositor timeout"),
    }
}

#[cfg(target_os = "linux")]
fn dispatch_browser_tool(
    id: serde_json::Value,
    name: String,
    arguments: serde_json::Value,
    cmd_tx: &channel::Sender<McpCommand>,
) -> JsonRpcResponse {
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    let cmd = McpCommand::BrowserTool {
        name,
        arguments,
        reply: reply_tx,
    };
    if cmd_tx.send(cmd).is_err() {
        return JsonRpcResponse::error(id, -32000, "compositor channel closed");
    }
    match reply_rx.recv_timeout(std::time::Duration::from_secs(180)) {
        Ok(result) => JsonRpcResponse::success(id, result),
        Err(_) => JsonRpcResponse::error(id, -32000, "browserd timeout"),
    }
}
