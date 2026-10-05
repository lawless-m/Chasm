//! The native namespace: `/dev/cons`, `/dev/time`, `/file/...`,
//! `/mnt/<name>/...` (a local directory or a 9p server) and `/net/http`.

use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use wack_core::layout as L;

use crate::ninep;
use crate::Host;

/// What `/mnt/<name>` serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mount {
    /// A local directory.
    Dir(PathBuf),
    /// A 9P2000 file server at `host:port`, over TCP.
    NineP(String),
}

/// Where `/dev/cons` goes.
#[derive(Debug, Clone, Default)]
pub enum Console {
    /// The process's stdin and stdout.
    #[default]
    Std,
    /// In-memory: reads come from `input`, writes are captured in `output`.
    Capture {
        input: Vec<u8>,
        pos: usize,
        output: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub console: Console,
    /// Whether `/file/...` maps to the host filesystem.
    pub file: bool,
    /// `/mnt/<name>` to what it serves.
    pub mounts: BTreeMap<String, Mount>,
    /// Whether `/net/http` and `/net/https` reach the network.
    pub net: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            console: Console::Std,
            file: true,
            mounts: BTreeMap::new(),
            net: true,
        }
    }
}

enum Handle {
    Cons,
    Time {
        done: bool,
    },
    File(File),
    Dir {
        records: Vec<u8>,
        ends: Vec<usize>,
        pos: usize,
    },
    /// An open file on a 9p mount.
    NineP {
        mount: String,
        fid: u32,
        offset: u64,
    },
    /// An HTTP request: what was written until the first read sends it.
    Http {
        url: String,
        written: Vec<u8>,
        response: Option<Result<Vec<u8>, i32>>,
        pos: usize,
    },
}

pub struct NativeHost {
    pub config: Config,
    handles: HashMap<i32, Handle>,
    next: i32,
    /// 9p connections, made on the first open under each mount.
    ninep: BTreeMap<String, ninep::Client>,
}

impl NativeHost {
    pub fn new(config: Config) -> Self {
        NativeHost {
            config,
            handles: HashMap::new(),
            next: 3,
            ninep: BTreeMap::new(),
        }
    }

    /// Console output captured so far (empty unless `Console::Capture`).
    pub fn captured_output(&self) -> &[u8] {
        match &self.config.console {
            Console::Capture { output, .. } => output,
            Console::Std => &[],
        }
    }

    /// Take the console output captured so far (empty unless `Console::Capture`).
    pub fn take_output(&mut self) -> Vec<u8> {
        match &mut self.config.console {
            Console::Capture { output, .. } => std::mem::take(output),
            Console::Std => Vec::new(),
        }
    }

    fn add(&mut self, h: Handle) -> i32 {
        let id = self.next;
        self.next += 1;
        self.handles.insert(id, h);
        id
    }

    /// `sub` under the 9p mount `name`: mode 0 reads a file or directory,
    /// 1 creates or truncates, 2 creates or appends, 3 opens read-write.
    fn open_ninep(&mut self, name: &str, addr: &str, sub: &str, mode: i32) -> i32 {
        if sub.split('/').any(|c| c == "..") {
            return L::E_PERMISSION;
        }
        if !self.ninep.contains_key(name) {
            match ninep::Client::connect(addr) {
                Ok(c) => {
                    self.ninep.insert(name.to_string(), c);
                }
                Err(_) => return L::E_IO,
            }
        }
        let c = self.ninep.get_mut(name).unwrap();
        let opened = match mode {
            L::MODE_READ => c.walk(sub).and_then(|fid| {
                let qid = c.open(fid, ninep::OREAD)?;
                if !qid.is_dir() {
                    return Ok(Ok(fid));
                }
                let entries = c.read_dir(fid);
                let _ = c.clunk(fid);
                let mut list: Vec<(String, u64, bool)> = entries?
                    .into_iter()
                    .map(|s| (s.name, s.length, s.is_dir))
                    .collect();
                list.sort();
                Ok(Err(encode_records(list)))
            }),
            L::MODE_READ_WRITE => c
                .walk(sub)
                .and_then(|fid| c.open(fid, ninep::ORDWR).map(|_| Ok(fid))),
            _ => {
                let trunc = if mode == L::MODE_WRITE {
                    ninep::OTRUNC
                } else {
                    0
                };
                match c.walk(sub) {
                    Ok(fid) => c.open(fid, ninep::OWRITE | trunc).map(|_| Ok(fid)),
                    Err(ninep::Error::NotFound) => {
                        let (parent, leaf) = sub.rsplit_once('/').unwrap_or(("", sub));
                        c.walk(parent).and_then(|fid| {
                            c.create(fid, leaf, 0o644, ninep::OWRITE | trunc)
                                .map(|_| Ok(fid))
                        })
                    }
                    Err(e) => Err(e),
                }
            }
        };
        match opened {
            Ok(Ok(fid)) => {
                let offset = if mode == L::MODE_APPEND {
                    match c.stat(fid) {
                        Ok(s) => s.length,
                        Err(e) => return ninep_err(e),
                    }
                } else {
                    0
                };
                self.add(Handle::NineP {
                    mount: name.to_string(),
                    fid,
                    offset,
                })
            }
            Ok(Err((records, ends))) => self.add(Handle::Dir {
                records,
                ends,
                pos: 0,
            }),
            Err(e) => ninep_err(e),
        }
    }

    /// `/net/http/<host>[:port]/<path>` or `/net/https/...`.
    fn open_net(&mut self, rest: &str) -> i32 {
        if !self.config.net {
            return L::E_NOT_SUPPORTED;
        }
        let (scheme, rest) = rest.split_once('/').unwrap_or((rest, ""));
        if scheme != "http" && scheme != "https" {
            return L::E_NOT_SUPPORTED;
        }
        let (host, sub) = rest.split_once('/').unwrap_or((rest, ""));
        if host.is_empty() {
            return L::E_NOT_FOUND;
        }
        self.add(Handle::Http {
            url: format!("{scheme}://{host}/{sub}"),
            written: Vec::new(),
            response: None,
            pos: 0,
        })
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, i32> {
        let safe = |rest: &str| -> Result<PathBuf, i32> {
            let p = Path::new(rest);
            if p.components().any(|c| matches!(c, Component::ParentDir)) {
                return Err(L::E_PERMISSION);
            }
            Ok(p.to_path_buf())
        };
        if path == "/file" || path.starts_with("/file/") {
            if !self.config.file {
                return Err(L::E_NOT_SUPPORTED);
            }
            let rest = path.strip_prefix("/file").unwrap();
            return Ok(PathBuf::from(if rest.is_empty() { "/" } else { rest }));
        }
        if let Some(rest) = path.strip_prefix("/mnt/") {
            let (name, sub) = rest.split_once('/').unwrap_or((rest, ""));
            return match self.config.mounts.get(name) {
                Some(Mount::Dir(root)) => Ok(root.join(safe(sub)?)),
                _ => Err(L::E_NOT_FOUND),
            };
        }
        if path.starts_with("/net/") {
            return Err(L::E_NOT_SUPPORTED);
        }
        Err(L::E_NOT_FOUND)
    }
}

fn ninep_err(e: ninep::Error) -> i32 {
    match e {
        ninep::Error::NotFound => L::E_NOT_FOUND,
        ninep::Error::Permission => L::E_PERMISSION,
        ninep::Error::Io(_) => L::E_IO,
    }
}

fn io_err(e: std::io::Error) -> i32 {
    match e.kind() {
        ErrorKind::NotFound => L::E_NOT_FOUND,
        ErrorKind::PermissionDenied => L::E_PERMISSION,
        _ => L::E_IO,
    }
}

/// Encode directory records: `u32` name length, name bytes, `u64` size, `u8` is-dir.
fn dir_records(path: &Path) -> std::io::Result<(Vec<u8>, Vec<usize>)> {
    let mut entries: Vec<(String, u64, bool)> = Vec::new();
    for e in std::fs::read_dir(path)? {
        let e = e?;
        let md = e.metadata()?;
        entries.push((
            e.file_name().to_string_lossy().into_owned(),
            md.len(),
            md.is_dir(),
        ));
    }
    entries.sort();
    Ok(encode_records(entries))
}

/// Directory records for sorted entries, and where each record ends.
fn encode_records(entries: Vec<(String, u64, bool)>) -> (Vec<u8>, Vec<usize>) {
    let mut out = Vec::new();
    let mut ends = Vec::new();
    for (name, size, is_dir) in entries {
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.push(is_dir as u8);
        ends.push(out.len());
    }
    (out, ends)
}

impl Host for NativeHost {
    fn open(&mut self, path: &str, mode: i32) -> i32 {
        if !(L::MODE_READ..=L::MODE_READ_WRITE).contains(&mode) {
            return L::E_NOT_SUPPORTED;
        }
        match path {
            "/dev/cons" => return self.add(Handle::Cons),
            "/dev/time" => {
                return if mode == L::MODE_READ {
                    self.add(Handle::Time { done: false })
                } else {
                    L::E_PERMISSION
                }
            }
            _ => {}
        }
        if let Some(rest) = path.strip_prefix("/net/") {
            return self.open_net(rest);
        }
        if let Some(rest) = path.strip_prefix("/mnt/") {
            let (name, sub) = rest.split_once('/').unwrap_or((rest, ""));
            if let Some(Mount::NineP(addr)) = self.config.mounts.get(name) {
                let addr = addr.clone();
                return self.open_ninep(name, &addr, sub, mode);
            }
        }
        let p = match self.resolve(path) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if mode == L::MODE_READ && p.is_dir() {
            return match dir_records(&p) {
                Ok((records, ends)) => self.add(Handle::Dir {
                    records,
                    ends,
                    pos: 0,
                }),
                Err(e) => io_err(e),
            };
        }
        let mut o = OpenOptions::new();
        match mode {
            L::MODE_READ => o.read(true),
            L::MODE_WRITE => o.write(true).create(true).truncate(true),
            L::MODE_APPEND => o.append(true).create(true),
            _ => o.read(true).write(true),
        };
        match o.open(&p) {
            Ok(f) => self.add(Handle::File(f)),
            Err(e) => io_err(e),
        }
    }

    fn read(&mut self, handle: i32, buf: &mut [u8]) -> i32 {
        let console = &mut self.config.console;
        match self.handles.get_mut(&handle) {
            None => L::E_BAD_HANDLE,
            Some(Handle::Cons) => match console {
                Console::Std => match std::io::stdin().read(buf) {
                    Ok(n) => n as i32,
                    Err(e) => io_err(e),
                },
                Console::Capture { input, pos, .. } => {
                    let n = buf.len().min(input.len() - *pos);
                    buf[..n].copy_from_slice(&input[*pos..*pos + n]);
                    *pos += n;
                    n as i32
                }
            },
            Some(Handle::Time { done }) => {
                if *done {
                    return 0;
                }
                if buf.len() < 8 {
                    return L::E_IO;
                }
                let ns = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                buf[..8].copy_from_slice(&ns.to_le_bytes());
                *done = true;
                8
            }
            Some(Handle::File(f)) => match f.read(buf) {
                Ok(n) => n as i32,
                Err(e) => io_err(e),
            },
            Some(Handle::NineP { mount, fid, offset }) => {
                let Some(c) = self.ninep.get_mut(mount.as_str()) else {
                    return L::E_IO;
                };
                match c.read(*fid, *offset, buf.len() as u32) {
                    Ok(data) => {
                        buf[..data.len()].copy_from_slice(&data);
                        *offset += data.len() as u64;
                        data.len() as i32
                    }
                    Err(e) => ninep_err(e),
                }
            }
            Some(Handle::Http {
                url,
                written,
                response,
                pos,
            }) => {
                let body = match response.get_or_insert_with(|| crate::net::perform(url, written)) {
                    Ok(body) => body,
                    Err(e) => return *e,
                };
                let n = buf.len().min(body.len() - *pos);
                buf[..n].copy_from_slice(&body[*pos..*pos + n]);
                *pos += n;
                n as i32
            }
            Some(Handle::Dir { records, ends, pos }) => {
                if *pos >= records.len() {
                    return 0;
                }
                // Whole records only.
                let limit = *pos + buf.len();
                let end = ends
                    .iter()
                    .copied()
                    .filter(|&e| e > *pos && e <= limit)
                    .max();
                match end {
                    Some(end) => {
                        let n = end - *pos;
                        buf[..n].copy_from_slice(&records[*pos..end]);
                        *pos = end;
                        n as i32
                    }
                    None => L::E_IO,
                }
            }
        }
    }

    fn write(&mut self, handle: i32, buf: &[u8]) -> i32 {
        let console = &mut self.config.console;
        match self.handles.get_mut(&handle) {
            None => L::E_BAD_HANDLE,
            Some(Handle::Cons) => match console {
                Console::Std => {
                    let mut out = std::io::stdout().lock();
                    match out.write_all(buf).and_then(|_| out.flush()) {
                        Ok(()) => buf.len() as i32,
                        Err(e) => io_err(e),
                    }
                }
                Console::Capture { output, .. } => {
                    output.extend_from_slice(buf);
                    buf.len() as i32
                }
            },
            Some(Handle::File(f)) => match f.write_all(buf) {
                Ok(()) => buf.len() as i32,
                Err(e) => io_err(e),
            },
            Some(Handle::NineP { mount, fid, offset }) => {
                let Some(c) = self.ninep.get_mut(mount.as_str()) else {
                    return L::E_IO;
                };
                match c.write(*fid, *offset, buf) {
                    Ok(n) => {
                        *offset += n as u64;
                        n as i32
                    }
                    Err(e) => ninep_err(e),
                }
            }
            Some(Handle::Http {
                written,
                response: None,
                ..
            }) => {
                written.extend_from_slice(buf);
                buf.len() as i32
            }
            Some(_) => L::E_PERMISSION,
        }
    }

    fn close(&mut self, handle: i32) -> i32 {
        match self.handles.remove(&handle) {
            Some(Handle::NineP { mount, fid, .. }) => {
                if let Some(c) = self.ninep.get_mut(&mount) {
                    let _ = c.clunk(fid);
                }
                0
            }
            Some(_) => 0,
            None => L::E_BAD_HANDLE,
        }
    }
}
