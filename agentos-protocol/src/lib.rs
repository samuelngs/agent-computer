use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
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
    FileRead { path: String },
    #[serde(rename = "file_write")]
    FileWrite { path: String, data: Vec<u8> },
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

pub const VSOCK_PORT: u32 = 9339;
