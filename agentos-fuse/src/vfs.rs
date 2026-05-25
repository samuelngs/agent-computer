use agentos_protocol::fs::*;
use base64::Engine;
use fuser::{
    FileAttr as FuseFileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request,
    TimeOrNow,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{BufReader, BufWriter};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

const TTL: Duration = Duration::from_secs(1);
const BLOCK_SIZE: u32 = 512;

struct InodeTable {
    next_ino: u64,
    ino_to_path: HashMap<u64, String>,
    path_to_ino: HashMap<String, u64>,
}

impl InodeTable {
    fn new() -> Self {
        let mut t = Self {
            next_ino: 2,
            ino_to_path: HashMap::new(),
            path_to_ino: HashMap::new(),
        };
        t.ino_to_path.insert(1, "".into());
        t.path_to_ino.insert("".into(), 1);
        t
    }

    fn get_path(&self, ino: u64) -> Option<&str> {
        self.ino_to_path.get(&ino).map(|s| s.as_str())
    }

    fn get_or_insert(&mut self, path: &str) -> u64 {
        if let Some(&ino) = self.path_to_ino.get(path) {
            return ino;
        }
        let ino = self.next_ino;
        self.next_ino += 1;
        self.ino_to_path.insert(ino, path.to_string());
        self.path_to_ino.insert(path.to_string(), ino);
        ino
    }

    fn child_path(&self, parent_ino: u64, name: &str) -> Option<String> {
        let parent = self.get_path(parent_ino)?;
        if parent.is_empty() {
            Some(name.to_string())
        } else {
            Some(format!("{parent}/{name}"))
        }
    }
}

pub struct HostFs {
    inodes: Mutex<InodeTable>,
    conn: Mutex<Connection>,
}

struct Connection {
    writer: BufWriter<Box<dyn std::io::Write + Send>>,
    reader: BufReader<Box<dyn std::io::Read + Send>>,
    next_id: u64,
}

impl Connection {
    fn rpc(&mut self, op: FsOp) -> std::io::Result<FsResponse> {
        let id = self.next_id;
        self.next_id += 1;
        let req = FsRequest { id, op };
        send(&mut self.writer, &req)?;
        recv(&mut self.reader)
    }
}

impl HostFs {
    pub fn listen_and_init(port: u32, host_path: &str) -> anyhow::Result<Self> {
        eprintln!("agentos-fuse: listening on vsock port {port}");
        let stream = vsock_accept(port)?;
        eprintln!("agentos-fuse: host connected");

        let reader: Box<dyn std::io::Read + Send> = Box::new(stream.try_clone()?);
        let writer: Box<dyn std::io::Write + Send> = Box::new(stream);
        let mut conn = Connection {
            writer: BufWriter::new(writer),
            reader: BufReader::new(reader),
            next_id: 1,
        };

        let resp = conn.rpc(FsOp::Init {
            root: host_path.to_string(),
        })?;
        match resp.result {
            FsResult::Ok { .. } => {
                eprintln!("agentos-fuse: init ok for {host_path}");
            }
            FsResult::Err { errno } => {
                anyhow::bail!("init rejected: errno {errno}");
            }
        }

        Ok(Self {
            inodes: Mutex::new(InodeTable::new()),
            conn: Mutex::new(conn),
        })
    }
}

fn vsock_accept(port: u32) -> std::io::Result<std::net::TcpStream> {
    unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let mut addr: libc::sockaddr_vm = std::mem::zeroed();
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_cid = libc::VMADDR_CID_ANY;
        addr.svm_port = port;

        if libc::bind(
            fd,
            &addr as *const libc::sockaddr_vm as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
        ) < 0
        {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }

        if libc::listen(fd, 1) < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }

        let conn = libc::accept(fd, std::ptr::null_mut(), std::ptr::null_mut());
        if conn < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        libc::close(fd);

        use std::os::unix::io::FromRawFd;
        Ok(std::net::TcpStream::from_raw_fd(conn))
    }
}

fn proto_attr_to_fuse(ino: u64, attr: &FileAttr) -> FuseFileAttr {
    let kind = if attr.is_dir() {
        FileType::Directory
    } else if attr.is_symlink() {
        FileType::Symlink
    } else {
        FileType::RegularFile
    };

    FuseFileAttr {
        ino,
        size: attr.size,
        blocks: attr.blocks,
        atime: UNIX_EPOCH + Duration::from_secs(attr.atime.max(0) as u64),
        mtime: UNIX_EPOCH + Duration::from_secs(attr.mtime.max(0) as u64),
        ctime: UNIX_EPOCH + Duration::from_secs(attr.ctime.max(0) as u64),
        crtime: UNIX_EPOCH,
        kind,
        perm: (attr.mode & 0o7777) as u16,
        nlink: attr.nlink,
        uid: attr.uid,
        gid: attr.gid,
        rdev: 0,
        blksize: BLOCK_SIZE,
        flags: 0,
    }
}

fn fuse_err(errno: i32) -> libc::c_int {
    errno
}

impl Filesystem for HostFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_str = name.to_string_lossy();
        let mut inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.child_path(parent, &name_str) else {
            reply.error(libc::ENOENT);
            return;
        };
        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Stat { path: path.clone() }) {
            Ok(FsResponse {
                result: FsResult::Ok { body: FsBody::Attr(attr) },
                ..
            }) => {
                let ino = inodes.get_or_insert(&path);
                reply.entry(&TTL, &proto_attr_to_fuse(ino, &attr), 0);
            }
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        let inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.get_path(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let path = path.to_string();
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Stat { path }) {
            Ok(FsResponse {
                result: FsResult::Ok { body: FsBody::Attr(attr) },
                ..
            }) => reply.attr(&TTL, &proto_attr_to_fuse(ino, &attr)),
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let inodes_guard = self.inodes.lock().unwrap();
        let Some(path) = inodes_guard.get_path(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let path = path.to_string();
        drop(inodes_guard);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Readdir { path: path.clone() }) {
            Ok(FsResponse {
                result: FsResult::Ok { body: FsBody::Dir { entries } },
                ..
            }) => {
                drop(conn);
                let mut inodes = self.inodes.lock().unwrap();
                for (i, entry) in entries.iter().enumerate().skip(offset as usize) {
                    let kind = if entry.file_type == 0o040000 {
                        FileType::Directory
                    } else if entry.file_type == 0o120000 {
                        FileType::Symlink
                    } else {
                        FileType::RegularFile
                    };

                    let child_ino = if entry.name == "." {
                        ino
                    } else if entry.name == ".." {
                        1
                    } else {
                        let child_path = if path.is_empty() {
                            entry.name.clone()
                        } else {
                            format!("{path}/{}", entry.name)
                        };
                        inodes.get_or_insert(&child_path)
                    };

                    if reply.add(child_ino, (i + 1) as i64, kind, &entry.name) {
                        break;
                    }
                }
                reply.ok();
            }
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        let inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.get_path(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let path = path.to_string();
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Open {
            path,
            flags: flags as u32,
        }) {
            Ok(FsResponse {
                result: FsResult::Ok { .. },
                ..
            }) => reply.opened(0, 0),
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.get_path(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let path = path.to_string();
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Read {
            path,
            offset: offset as u64,
            size,
        }) {
            Ok(FsResponse {
                result: FsResult::Ok { body: FsBody::Data { data, .. } },
                ..
            }) => {
                let b64 = base64::engine::general_purpose::STANDARD;
                match b64.decode(&data) {
                    Ok(bytes) => reply.data(&bytes),
                    Err(_) => reply.error(libc::EIO),
                }
            }
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.get_path(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let path = path.to_string();
        drop(inodes);

        let b64 = base64::engine::general_purpose::STANDARD;
        let encoded = b64.encode(data);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Write {
            path,
            offset: offset as u64,
            data: encoded,
        }) {
            Ok(FsResponse {
                result: FsResult::Ok { body: FsBody::Written { size } },
                ..
            }) => reply.written(size),
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn create(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let name_str = name.to_string_lossy();
        let mut inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.child_path(parent, &name_str) else {
            reply.error(libc::ENOENT);
            return;
        };
        let ino = inodes.get_or_insert(&path);
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Create {
            path,
            mode,
            flags: flags as u32,
        }) {
            Ok(FsResponse {
                result: FsResult::Ok { body: FsBody::Attr(attr) },
                ..
            }) => reply.created(&TTL, &proto_attr_to_fuse(ino, &attr), 0, 0, 0),
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let name_str = name.to_string_lossy();
        let inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.child_path(parent, &name_str) else {
            reply.error(libc::ENOENT);
            return;
        };
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Unlink { path }) {
            Ok(FsResponse {
                result: FsResult::Ok { .. },
                ..
            }) => reply.ok(),
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name_str = name.to_string_lossy();
        let mut inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.child_path(parent, &name_str) else {
            reply.error(libc::ENOENT);
            return;
        };
        let ino = inodes.get_or_insert(&path);
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Mkdir { path, mode }) {
            Ok(FsResponse {
                result: FsResult::Ok { body: FsBody::Attr(attr) },
                ..
            }) => reply.entry(&TTL, &proto_attr_to_fuse(ino, &attr), 0),
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let name_str = name.to_string_lossy();
        let inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.child_path(parent, &name_str) else {
            reply.error(libc::ENOENT);
            return;
        };
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Rmdir { path }) {
            Ok(FsResponse {
                result: FsResult::Ok { .. },
                ..
            }) => reply.ok(),
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        let name_str = name.to_string_lossy();
        let newname_str = newname.to_string_lossy();
        let inodes = self.inodes.lock().unwrap();
        let from = inodes.child_path(parent, &name_str);
        let to = inodes.child_path(newparent, &newname_str);
        drop(inodes);

        let (Some(from), Some(to)) = (from, to) else {
            reply.error(libc::ENOENT);
            return;
        };

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Rename { from, to }) {
            Ok(FsResponse {
                result: FsResult::Ok { .. },
                ..
            }) => reply.ok(),
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<std::time::SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<std::time::SystemTime>,
        _chgtime: Option<std::time::SystemTime>,
        _bkuptime: Option<std::time::SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        let inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.get_path(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let path = path.to_string();
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();

        if let Some(new_size) = size {
            match conn.rpc(FsOp::Truncate {
                path: path.clone(),
                size: new_size,
            }) {
                Ok(FsResponse {
                    result: FsResult::Err { errno },
                    ..
                }) => {
                    reply.error(fuse_err(errno));
                    return;
                }
                Err(_) => {
                    reply.error(libc::EIO);
                    return;
                }
                _ => {}
            }
        }

        if let Some(new_mode) = mode {
            match conn.rpc(FsOp::Chmod {
                path: path.clone(),
                mode: new_mode,
            }) {
                Ok(FsResponse {
                    result: FsResult::Err { errno },
                    ..
                }) => {
                    reply.error(fuse_err(errno));
                    return;
                }
                Err(_) => {
                    reply.error(libc::EIO);
                    return;
                }
                _ => {}
            }
        }

        match conn.rpc(FsOp::Stat { path }) {
            Ok(FsResponse {
                result: FsResult::Ok { body: FsBody::Attr(attr) },
                ..
            }) => reply.attr(&TTL, &proto_attr_to_fuse(ino, &attr)),
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        let inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.get_path(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let path = path.to_string();
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Readlink { path }) {
            Ok(FsResponse {
                result: FsResult::Ok { body: FsBody::Link { target } },
                ..
            }) => reply.data(target.as_bytes()),
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn symlink(
        &mut self,
        _req: &Request,
        parent: u64,
        link_name: &OsStr,
        target: &std::path::Path,
        reply: ReplyEntry,
    ) {
        let name_str = link_name.to_string_lossy();
        let target_str = target.to_string_lossy().to_string();
        let mut inodes = self.inodes.lock().unwrap();
        let Some(linkpath) = inodes.child_path(parent, &name_str) else {
            reply.error(libc::ENOENT);
            return;
        };
        let ino = inodes.get_or_insert(&linkpath);
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Symlink {
            target: target_str,
            linkpath: linkpath.clone(),
        }) {
            Ok(FsResponse {
                result: FsResult::Ok { .. },
                ..
            }) => match conn.rpc(FsOp::Stat { path: linkpath }) {
                Ok(FsResponse {
                    result: FsResult::Ok { body: FsBody::Attr(attr) },
                    ..
                }) => reply.entry(&TTL, &proto_attr_to_fuse(ino, &attr), 0),
                _ => reply.error(libc::EIO),
            },
            Ok(FsResponse {
                result: FsResult::Err { errno },
                ..
            }) => reply.error(fuse_err(errno)),
            _ => reply.error(libc::EIO),
        }
    }

    fn statfs(&mut self, _req: &Request, ino: u64, reply: ReplyStatfs) {
        let inodes = self.inodes.lock().unwrap();
        let path = inodes.get_path(ino).unwrap_or("").to_string();
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        match conn.rpc(FsOp::Statfs { path }) {
            Ok(FsResponse {
                result: FsResult::Ok { body: FsBody::StatVfs(sv) },
                ..
            }) => reply.statfs(
                sv.blocks,
                sv.bfree,
                sv.bavail,
                sv.files,
                sv.ffree,
                sv.bsize as u32,
                sv.namelen,
                sv.bsize as u32,
            ),
            _ => reply.error(libc::EIO),
        }
    }

    fn flush(&mut self, _req: &Request, ino: u64, _fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        let inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.get_path(ino) else {
            reply.ok();
            return;
        };
        let path = path.to_string();
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        let _ = conn.rpc(FsOp::Flush { path });
        reply.ok();
    }

    fn release(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let inodes = self.inodes.lock().unwrap();
        let Some(path) = inodes.get_path(ino) else {
            reply.ok();
            return;
        };
        let path = path.to_string();
        drop(inodes);

        let mut conn = self.conn.lock().unwrap();
        let _ = conn.rpc(FsOp::Release { path });
        reply.ok();
    }

    fn opendir(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        let inodes = self.inodes.lock().unwrap();
        if inodes.get_path(ino).is_some() {
            reply.opened(0, 0);
        } else {
            reply.error(libc::ENOENT);
        }
    }

    fn releasedir(&mut self, _req: &Request, _ino: u64, _fh: u64, _flags: i32, reply: ReplyEmpty) {
        reply.ok();
    }
}
