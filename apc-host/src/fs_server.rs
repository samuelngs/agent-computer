#[cfg(target_os = "macos")]
use apc_protocol::fs::*;
#[cfg(target_os = "macos")]
use std::io::{BufReader, BufWriter};
#[cfg(target_os = "macos")]
use std::os::unix::fs::MetadataExt;
#[cfg(target_os = "macos")]
use std::os::unix::net::UnixStream;
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::Arc;

#[cfg(target_os = "macos")]
pub struct FsServer {
    socket_path: String,
    allow_patterns: Arc<Vec<String>>,
}

#[cfg(target_os = "macos")]
impl FsServer {
    pub fn new(socket_path: &str, allow_patterns: Vec<String>) -> Self {
        Self {
            socket_path: socket_path.to_string(),
            allow_patterns: Arc::new(allow_patterns),
        }
    }

    pub fn start(&self) {
        let path = self.socket_path.clone();
        let patterns = self.allow_patterns.clone();

        std::thread::Builder::new()
            .name("fs-server".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    match UnixStream::connect(&path) {
                        Ok(stream) => {
                            tracing::info!("FS server connected to guest via {path}");
                            let patterns = patterns.clone();
                            if let Err(e) = handle_connection(stream, &patterns) {
                                tracing::debug!("FS connection ended: {e}");
                            }
                        }
                        Err(_) => continue,
                    }
                }
            })
            .expect("failed to spawn fs-server thread");
    }
}

#[cfg(target_os = "macos")]
fn is_path_allowed(path: &str, patterns: &[String]) -> bool {
    if patterns.iter().any(|p| p == "*") {
        return true;
    }
    let canon = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => PathBuf::from(path),
    };
    let canon_str = canon.to_string_lossy();
    patterns.iter().any(|p| {
        let prefix = match std::fs::canonicalize(p) {
            Ok(c) => c,
            Err(_) => PathBuf::from(p),
        };
        canon_str.starts_with(&prefix.to_string_lossy().as_ref())
    })
}

#[cfg(target_os = "macos")]
fn handle_connection(stream: UnixStream, patterns: &[String]) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = BufWriter::new(stream);
    let mut root: Option<PathBuf> = None;

    loop {
        let req: FsRequest = match recv(&mut reader) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        let resp = match &req.op {
            FsOp::Init { root: req_root } => {
                if !is_path_allowed(req_root, patterns) {
                    tracing::warn!(path = req_root, "FS mount denied by allowlist");
                    FsResponse::err(req.id, libc::EACCES)
                } else if !Path::new(req_root).is_dir() {
                    FsResponse::err(req.id, libc::ENOENT)
                } else {
                    let canon = std::fs::canonicalize(req_root)?;
                    tracing::info!(root = %canon.display(), "FS session initialized");
                    root = Some(canon);
                    FsResponse::ok(req.id, FsBody::Empty)
                }
            }
            _ => {
                let Some(ref base) = root else {
                    send(&mut writer, &FsResponse::err(req.id, libc::EACCES))?;
                    continue;
                };
                dispatch(req.id, &req.op, base)
            }
        };

        send(&mut writer, &resp)?;
    }
}

#[cfg(target_os = "macos")]
fn resolve(base: &Path, rel: &str) -> PathBuf {
    if rel.is_empty() || rel == "." || rel == "/" {
        return base.to_path_buf();
    }
    let rel = rel.strip_prefix('/').unwrap_or(rel);
    base.join(rel)
}

#[cfg(target_os = "macos")]
fn dispatch(id: u64, op: &FsOp, base: &Path) -> FsResponse {
    match op {
        FsOp::Init { .. } => unreachable!(),

        FsOp::Stat { path } => {
            let full = resolve(base, path);
            match std::fs::symlink_metadata(&full) {
                Ok(m) => FsResponse::ok(id, FsBody::Attr(meta_to_attr(&m))),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Readdir { path } => {
            let full = resolve(base, path);
            match std::fs::read_dir(&full) {
                Ok(rd) => {
                    let mut entries = vec![
                        DirEntry {
                            name: ".".into(),
                            file_type: 0o040000,
                        },
                        DirEntry {
                            name: "..".into(),
                            file_type: 0o040000,
                        },
                    ];
                    for entry in rd.flatten() {
                        let ft = entry
                            .file_type()
                            .map(|t| {
                                if t.is_dir() {
                                    0o040000u32
                                } else if t.is_symlink() {
                                    0o120000
                                } else {
                                    0o100000
                                }
                            })
                            .unwrap_or(0o100000);
                        entries.push(DirEntry {
                            name: entry.file_name().to_string_lossy().into(),
                            file_type: ft,
                        });
                    }
                    FsResponse::ok(id, FsBody::Dir { entries })
                }
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Open { path, .. } => {
            let full = resolve(base, path);
            match std::fs::symlink_metadata(&full) {
                Ok(m) => FsResponse::ok(id, FsBody::Attr(meta_to_attr(&m))),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Read { path, offset, size } => {
            let full = resolve(base, path);
            match read_file_range(&full, *offset, *size) {
                Ok((data, len)) => FsResponse::ok(
                    id,
                    FsBody::Data {
                        data,
                        size: len as u32,
                    },
                ),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Write { path, offset, data } => {
            let full = resolve(base, path);
            match write_file_range(&full, *offset, data) {
                Ok(n) => FsResponse::ok(id, FsBody::Written { size: n as u32 }),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Create {
            path,
            mode,
            flags: _,
        } => {
            let full = resolve(base, path);
            match create_file(&full, *mode) {
                Ok(m) => FsResponse::ok(id, FsBody::Attr(meta_to_attr(&m))),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Unlink { path } => {
            let full = resolve(base, path);
            match std::fs::remove_file(&full) {
                Ok(()) => FsResponse::ok(id, FsBody::Empty),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Mkdir { path, mode: _ } => {
            let full = resolve(base, path);
            match std::fs::create_dir(&full) {
                Ok(()) => match std::fs::symlink_metadata(&full) {
                    Ok(m) => FsResponse::ok(id, FsBody::Attr(meta_to_attr(&m))),
                    Err(e) => FsResponse::err(id, errno_from(e)),
                },
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Rmdir { path } => {
            let full = resolve(base, path);
            match std::fs::remove_dir(&full) {
                Ok(()) => FsResponse::ok(id, FsBody::Empty),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Rename { from, to } => {
            let full_from = resolve(base, from);
            let full_to = resolve(base, to);
            match std::fs::rename(&full_from, &full_to) {
                Ok(()) => FsResponse::ok(id, FsBody::Empty),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Truncate { path, size } => {
            let full = resolve(base, path);
            match truncate_file(&full, *size) {
                Ok(()) => FsResponse::ok(id, FsBody::Empty),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Chmod { path, mode } => {
            let full = resolve(base, path);
            match set_permissions(&full, *mode) {
                Ok(()) => FsResponse::ok(id, FsBody::Empty),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Symlink { target, linkpath } => {
            let full_link = resolve(base, linkpath);
            match std::os::unix::fs::symlink(target, &full_link) {
                Ok(()) => FsResponse::ok(id, FsBody::Empty),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Readlink { path } => {
            let full = resolve(base, path);
            match std::fs::read_link(&full) {
                Ok(target) => FsResponse::ok(
                    id,
                    FsBody::Link {
                        target: target.to_string_lossy().into(),
                    },
                ),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }

        FsOp::Release { .. } | FsOp::Flush { .. } => FsResponse::ok(id, FsBody::Empty),

        FsOp::Statfs { path } => {
            let full = resolve(base, path);
            match statfs_path(&full) {
                Ok(sv) => FsResponse::ok(id, FsBody::StatVfs(sv)),
                Err(e) => FsResponse::err(id, errno_from(e)),
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn meta_to_attr(m: &std::fs::Metadata) -> FileAttr {
    FileAttr {
        size: m.len(),
        blocks: m.blocks(),
        atime: m.atime(),
        mtime: m.mtime(),
        ctime: m.ctime(),
        mode: m.mode(),
        nlink: m.nlink() as u32,
        uid: m.uid(),
        gid: m.gid(),
    }
}

#[cfg(target_os = "macos")]
fn errno_from(e: std::io::Error) -> i32 {
    e.raw_os_error().unwrap_or(libc::EIO)
}

#[cfg(target_os = "macos")]
fn read_file_range(path: &Path, offset: u64, size: u32) -> std::io::Result<(String, usize)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; size as usize];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    use base64::Engine;
    Ok((base64::engine::general_purpose::STANDARD.encode(&buf), n))
}

#[cfg(target_os = "macos")]
fn write_file_range(path: &Path, offset: u64, data_b64: &str) -> std::io::Result<usize> {
    use base64::Engine;
    use std::io::{Seek, SeekFrom, Write};
    let data = base64::engine::general_purpose::STANDARD
        .decode(data_b64)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    f.write_all(&data)?;
    Ok(data.len())
}

#[cfg(target_os = "macos")]
fn create_file(path: &Path, mode: u32) -> std::io::Result<std::fs::Metadata> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(mode & 0o7777)
        .open(path)?;
    std::fs::symlink_metadata(path)
}

#[cfg(target_os = "macos")]
fn truncate_file(path: &Path, size: u64) -> std::io::Result<()> {
    let f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.set_len(size)
}

#[cfg(target_os = "macos")]
fn set_permissions(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(mode & 0o7777);
    std::fs::set_permissions(path, perms)
}

#[cfg(target_os = "macos")]
fn statfs_path(path: &Path) -> std::io::Result<StatVfs> {
    use std::ffi::CString;
    let c_path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    unsafe {
        let mut buf: libc::statfs = std::mem::zeroed();
        if libc::statfs(c_path.as_ptr(), &mut buf) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(StatVfs {
            bsize: buf.f_bsize as u64,
            blocks: buf.f_blocks,
            bfree: buf.f_bfree,
            bavail: buf.f_bavail,
            files: buf.f_files,
            ffree: buf.f_ffree,
            namelen: 255,
        })
    }
}
