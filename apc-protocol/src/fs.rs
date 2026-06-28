use serde::{Deserialize, Serialize};
use std::io;

pub const VSOCK_FS_PORT: u32 = 9340;

const MAX_FRAME_LEN: u32 = 64 * 1024 * 1024;

pub fn write_frame(w: &mut impl io::Write, payload: &[u8]) -> io::Result<()> {
    let len = payload.len() as u32;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

pub fn read_frame(r: &mut impl io::Read) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {len} bytes"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

pub fn send<T: Serialize>(w: &mut impl io::Write, msg: &T) -> io::Result<()> {
    let payload =
        serde_json::to_vec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_frame(w, &payload)
}

pub fn recv<T: for<'de> Deserialize<'de>>(r: &mut impl io::Read) -> io::Result<T> {
    let payload = read_frame(r)?;
    serde_json::from_slice(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsRequest {
    pub id: u64,
    #[serde(flatten)]
    pub op: FsOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum FsOp {
    #[serde(rename = "init")]
    Init { root: String },

    #[serde(rename = "stat")]
    Stat { path: String },

    #[serde(rename = "readdir")]
    Readdir { path: String },

    #[serde(rename = "open")]
    Open { path: String, flags: u32 },

    #[serde(rename = "read")]
    Read {
        path: String,
        offset: u64,
        size: u32,
    },

    #[serde(rename = "write")]
    Write {
        path: String,
        offset: u64,
        data: String,
    },

    #[serde(rename = "create")]
    Create { path: String, mode: u32, flags: u32 },

    #[serde(rename = "unlink")]
    Unlink { path: String },

    #[serde(rename = "mkdir")]
    Mkdir { path: String, mode: u32 },

    #[serde(rename = "rmdir")]
    Rmdir { path: String },

    #[serde(rename = "rename")]
    Rename { from: String, to: String },

    #[serde(rename = "truncate")]
    Truncate { path: String, size: u64 },

    #[serde(rename = "chmod")]
    Chmod { path: String, mode: u32 },

    #[serde(rename = "symlink")]
    Symlink { target: String, linkpath: String },

    #[serde(rename = "readlink")]
    Readlink { path: String },

    #[serde(rename = "release")]
    Release { path: String },

    #[serde(rename = "flush")]
    Flush { path: String },

    #[serde(rename = "statfs")]
    Statfs { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsResponse {
    pub id: u64,
    #[serde(flatten)]
    pub result: FsResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum FsResult {
    #[serde(rename = "ok")]
    Ok {
        #[serde(flatten)]
        body: FsBody,
    },
    #[serde(rename = "err")]
    Err { errno: i32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FsBody {
    #[serde(rename = "attr")]
    Attr(FileAttr),

    #[serde(rename = "dir")]
    Dir { entries: Vec<DirEntry> },

    #[serde(rename = "data")]
    Data { data: String, size: u32 },

    #[serde(rename = "written")]
    Written { size: u32 },

    #[serde(rename = "link")]
    Link { target: String },

    #[serde(rename = "statvfs")]
    StatVfs(StatVfs),

    #[serde(rename = "empty")]
    Empty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAttr {
    pub size: u64,
    pub blocks: u64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
}

impl FileAttr {
    pub fn is_dir(&self) -> bool {
        (self.mode & 0o170000) == 0o040000
    }

    pub fn is_symlink(&self) -> bool {
        (self.mode & 0o170000) == 0o120000
    }

    pub fn is_file(&self) -> bool {
        (self.mode & 0o170000) == 0o100000
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub file_type: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatVfs {
    pub bsize: u64,
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub namelen: u32,
}

impl FsResponse {
    pub fn ok(id: u64, body: FsBody) -> Self {
        Self {
            id,
            result: FsResult::Ok { body },
        }
    }

    pub fn err(id: u64, errno: i32) -> Self {
        Self {
            id,
            result: FsResult::Err { errno },
        }
    }
}
