pub mod fs;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: serde_json::Value,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    pub fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: serde_json::Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tool", content = "params")]
pub enum ToolCall {
    #[serde(rename = "screen_capture")]
    ScreenCapture {
        #[serde(skip_serializing_if = "Option::is_none")]
        region: Option<Rect>,
        #[serde(skip_serializing_if = "Option::is_none")]
        scale: Option<f32>,
    },
    #[serde(rename = "mouse_move")]
    MouseMove { x: i32, y: i32 },
    #[serde(rename = "mouse_click")]
    MouseClick { button: MouseButton },
    #[serde(rename = "keyboard_type")]
    KeyboardType { text: String },
    #[serde(rename = "keyboard_key")]
    KeyboardKey {
        key: String,
        #[serde(default)]
        modifiers: Vec<String>,
    },
    #[serde(rename = "window_list")]
    WindowList,
    #[serde(rename = "window_focus")]
    WindowFocus { id: u64 },
    #[serde(rename = "window_resize")]
    WindowResize { id: u64, width: u32, height: u32 },
    #[serde(rename = "window_move")]
    WindowMove { id: u64, x: i32, y: i32 },
    #[serde(rename = "window_open")]
    WindowOpen { cmd: String },
    #[serde(rename = "window_close")]
    WindowClose { id: u64 },
    #[serde(rename = "window_minimize")]
    WindowMinimize { id: u64 },
    #[serde(rename = "shell_exec")]
    ShellExec { cmd: String },
    #[serde(rename = "file_read")]
    FileRead {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<u32>,
    },
    #[serde(rename = "file_write")]
    FileWrite {
        path: String,
        data: Vec<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        offset: Option<u32>,
    },
    #[serde(rename = "fs_mount")]
    FsMount {
        host_path: String,
        #[serde(default = "default_guest_workspace")]
        guest_path: String,
    },
    #[serde(rename = "fs_unmount")]
    FsUnmount { guest_path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
    pub minimized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResult {
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub data: Vec<u8>,
}

fn default_guest_workspace() -> String {
    "/home/agentos/workspace".into()
}

pub const VSOCK_PORT: u32 = 9339;

pub fn mcp_tool_schemas() -> Vec<serde_json::Value> {
    serde_json::json!([
        {
            "name": "screen_capture",
            "description": "Capture the guest display as a PNG screenshot (full screen or a region).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "region": {
                        "type": "object",
                        "description": "Optional sub-region to capture",
                        "properties": {
                            "x": { "type": "integer" },
                            "y": { "type": "integer" },
                            "width": { "type": "integer" },
                            "height": { "type": "integer" }
                        },
                        "required": ["x", "y", "width", "height"]
                    },
                    "scale": { "type": "number", "description": "Scale factor for the capture" }
                }
            }
        },
        {
            "name": "mouse_move",
            "description": "Move the cursor to an absolute position.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": { "type": "integer" },
                    "y": { "type": "integer" }
                },
                "required": ["x", "y"]
            }
        },
        {
            "name": "mouse_click",
            "description": "Click a mouse button at the current position.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "button": { "type": "string", "enum": ["left", "right", "middle"], "default": "left" }
                },
                "required": ["button"]
            }
        },
        {
            "name": "keyboard_type",
            "description": "Type a string of text.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
                },
                "required": ["text"]
            }
        },
        {
            "name": "keyboard_key",
            "description": "Press a key with optional modifiers (shift, ctrl, alt, super).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "modifiers": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["key"]
            }
        },
        {
            "name": "window_list",
            "description": "List all open windows with their id, title, position, size, and focus state.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "window_focus",
            "description": "Focus and raise a window by its id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer" } },
                "required": ["id"]
            }
        },
        {
            "name": "window_resize",
            "description": "Resize a window by its id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" }
                },
                "required": ["id", "width", "height"]
            }
        },
        {
            "name": "window_move",
            "description": "Move a window to a new position.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "x": { "type": "integer" },
                    "y": { "type": "integer" }
                },
                "required": ["id", "x", "y"]
            }
        },
        {
            "name": "window_open",
            "description": "Launch a program in the guest.",
            "inputSchema": {
                "type": "object",
                "properties": { "cmd": { "type": "string" } },
                "required": ["cmd"]
            }
        },
        {
            "name": "window_close",
            "description": "Close a window by its id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer" } },
                "required": ["id"]
            }
        },
        {
            "name": "window_minimize",
            "description": "Minimize a window by its id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer" } },
                "required": ["id"]
            }
        },
        {
            "name": "shell_exec",
            "description": "Execute a shell command in the guest and return its output.",
            "inputSchema": {
                "type": "object",
                "properties": { "cmd": { "type": "string" } },
                "required": ["cmd"]
            }
        },
        {
            "name": "file_read",
            "description": "Read a file from the guest filesystem.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "line": { "type": "integer", "description": "Optional line number to jump to in editor" }
                },
                "required": ["path"]
            }
        },
        {
            "name": "file_write",
            "description": "Write data to a file on the guest filesystem.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "data": { "type": "array", "items": { "type": "integer" } },
                    "offset": { "type": "integer" }
                },
                "required": ["path", "data"]
            }
        },
        {
            "name": "fs_mount",
            "description": "Mount a host directory into the guest via FUSE-over-vsock. Default guest_path is /home/agentos/workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host_path": { "type": "string", "description": "Host directory path to mount" },
                    "guest_path": { "type": "string", "description": "Guest mount point (default: /home/agentos/workspace)", "default": "/home/agentos/workspace" }
                },
                "required": ["host_path"]
            }
        },
        {
            "name": "fs_unmount",
            "description": "Unmount a FUSE-mounted guest path.",
            "inputSchema": {
                "type": "object",
                "properties": { "guest_path": { "type": "string" } },
                "required": ["guest_path"]
            }
        }
    ])
    .as_array()
    .unwrap()
    .clone()
}

const UNIT_VARIANTS: &[&str] = &["window_list"];

pub fn toolcall_from_mcp(name: &str, args: &serde_json::Value) -> Result<ToolCall, String> {
    let mut obj = serde_json::Map::new();
    obj.insert("tool".into(), serde_json::Value::String(name.into()));
    if !UNIT_VARIANTS.contains(&name) {
        obj.insert("params".into(), args.clone());
    }
    serde_json::from_value(serde_json::Value::Object(obj))
        .map_err(|e| format!("invalid tool call: {e}"))
}
