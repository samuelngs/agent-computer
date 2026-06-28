pub mod fs;
pub mod screenshot;

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
    "/home/agent/workspace".into()
}

pub const VSOCK_PORT: u32 = 9339;

pub fn mcp_tool_schemas() -> Vec<serde_json::Value> {
    let mut tools = serde_json::json!([
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
            "description": "Mount a host directory into the guest via FUSE-over-vsock. Default guest_path is /home/agent/workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host_path": { "type": "string", "description": "Host directory path to mount" },
                    "guest_path": { "type": "string", "description": "Guest mount point (default: /home/agent/workspace)", "default": "/home/agent/workspace" }
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
    .clone();
    tools.extend(browser_tool_schemas());
    tools
}

pub const BROWSER_TOOL_NAMES: &[&str] = &[
    "browser_navigate",
    "browser_navigate_back",
    "browser_navigate_forward",
    "browser_reload",
    "browser_snapshot",
    "browser_take_screenshot",
    "browser_click",
    "browser_hover",
    "browser_type",
    "browser_press_key",
    "browser_select_option",
    "browser_drag",
    "browser_scroll",
    "browser_evaluate",
    "browser_console_messages",
    "browser_wait_for",
    "browser_tab_list",
    "browser_tab_new",
    "browser_close",
    "browser_resize",
    "browser_cookie_list",
    "browser_cookie_get",
    "browser_cookie_set",
    "browser_cookie_delete",
    "browser_cookie_clear",
];

pub fn is_browser_tool_name(name: &str) -> bool {
    BROWSER_TOOL_NAMES.contains(&name)
}

pub fn browser_tool_schemas() -> Vec<serde_json::Value> {
    serde_json::json!([
        {
            "name": "browser_navigate",
            "description": "Navigate to a URL",
            "inputSchema": {
                "type": "object",
                "properties": { "url": { "type": "string", "description": "URL to navigate to" } },
                "required": ["url"]
            }
        },
        {
            "name": "browser_navigate_back",
            "description": "Go back in browser history",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "browser_navigate_forward",
            "description": "Go forward in browser history",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "browser_reload",
            "description": "Reload the current page",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "browser_snapshot",
            "description": "Capture an accessibility snapshot of the current page",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "browser_take_screenshot",
            "description": "Take a screenshot of the current page",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "browser_click",
            "description": "Click an element on the page using its accessibility ref",
            "inputSchema": {
                "type": "object",
                "properties": { "ref": { "type": "string", "description": "Element ref from accessibility snapshot" } },
                "required": ["ref"]
            }
        },
        {
            "name": "browser_hover",
            "description": "Hover over an element on the page",
            "inputSchema": {
                "type": "object",
                "properties": { "ref": { "type": "string", "description": "Element ref from accessibility snapshot" } },
                "required": ["ref"]
            }
        },
        {
            "name": "browser_type",
            "description": "Type text into a focused element",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "Element ref from accessibility snapshot" },
                    "text": { "type": "string", "description": "Text to type" },
                    "clear": { "type": "boolean", "description": "Clear existing text before typing" }
                },
                "required": ["ref", "text"]
            }
        },
        {
            "name": "browser_press_key",
            "description": "Press a keyboard key or key combination",
            "inputSchema": {
                "type": "object",
                "properties": { "key": { "type": "string", "description": "Key to press (e.g. Enter, Tab, ArrowDown)" } },
                "required": ["key"]
            }
        },
        {
            "name": "browser_select_option",
            "description": "Select option(s) in a select element",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "Element ref from accessibility snapshot" },
                    "values": {
                        "type": "array",
                        "description": "Option values to select",
                        "items": { "type": "string" }
                    }
                },
                "required": ["ref", "values"]
            }
        },
        {
            "name": "browser_drag",
            "description": "Drag and drop from one element to another",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "startRef": { "type": "string", "description": "Source element ref" },
                    "endRef": { "type": "string", "description": "Target element ref" }
                },
                "required": ["startRef", "endRef"]
            }
        },
        {
            "name": "browser_scroll",
            "description": "Scroll the page",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "deltaX": { "type": "integer", "description": "Horizontal scroll amount in pixels" },
                    "deltaY": { "type": "integer", "description": "Vertical scroll amount in pixels" }
                }
            }
        },
        {
            "name": "browser_evaluate",
            "description": "Execute JavaScript in the browser console",
            "inputSchema": {
                "type": "object",
                "properties": { "expression": { "type": "string", "description": "JavaScript expression to evaluate" } },
                "required": ["expression"]
            }
        },
        {
            "name": "browser_console_messages",
            "description": "Get console messages from the page",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "browser_wait_for",
            "description": "Wait for a selector to appear or text to be present on the page",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector to wait for" },
                    "text": { "type": "string", "description": "Text to wait for on the page" },
                    "timeout": { "type": "integer", "description": "Timeout in milliseconds (default 30000)" }
                }
            }
        },
        {
            "name": "browser_tab_list",
            "description": "List all open browser tabs",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "browser_tab_new",
            "description": "Open a new browser tab",
            "inputSchema": {
                "type": "object",
                "properties": { "url": { "type": "string", "description": "URL to open in the new tab" } }
            }
        },
        {
            "name": "browser_close",
            "description": "Close a browser tab",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "targetId": {
                        "type": "string",
                        "description": "Target ID of the tab to close (closes current if omitted)"
                    }
                }
            }
        },
        {
            "name": "browser_resize",
            "description": "Resize the browser viewport",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "description": "Viewport width in pixels" },
                    "height": { "type": "integer", "description": "Viewport height in pixels" }
                }
            }
        },
        {
            "name": "browser_cookie_list",
            "description": "List cookies, optionally filtered by URL",
            "inputSchema": {
                "type": "object",
                "properties": { "url": { "type": "string", "description": "URL to filter cookies by" } }
            }
        },
        {
            "name": "browser_cookie_get",
            "description": "Get a specific cookie by name",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string", "description": "Cookie name" } },
                "required": ["name"]
            }
        },
        {
            "name": "browser_cookie_set",
            "description": "Set a cookie",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Cookie name" },
                    "value": { "type": "string", "description": "Cookie value" },
                    "domain": { "type": "string", "description": "Cookie domain" },
                    "path": { "type": "string", "description": "Cookie path" },
                    "url": { "type": "string", "description": "URL to associate cookie with" },
                    "secure": { "type": "boolean", "description": "Secure flag" },
                    "httpOnly": { "type": "boolean", "description": "HttpOnly flag" }
                },
                "required": ["name", "value"]
            }
        },
        {
            "name": "browser_cookie_delete",
            "description": "Delete a cookie by name",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Cookie name to delete" },
                    "url": { "type": "string", "description": "URL the cookie belongs to" },
                    "domain": { "type": "string", "description": "Cookie domain" }
                },
                "required": ["name"]
            }
        },
        {
            "name": "browser_cookie_clear",
            "description": "Clear all cookies",
            "inputSchema": { "type": "object", "properties": {} }
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
