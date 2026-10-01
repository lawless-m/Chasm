//! The native namespace: `/dev/cons`, `/dev/time`, `/file/...`, `/mnt/<name>/...`.

use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chasm_core::layout as L;

use crate::Host;

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
    /// `/mnt/<name>` to local directory.
    pub mounts: BTreeMap<String, PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            console: Console::Std,
            file: true,
            mounts: BTreeMap::new(),
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
}

pub struct NativeHost {
    pub config: Config,
    handles: HashMap<i32, Handle>,
    next: i32,
}

impl NativeHost {
    pub fn new(config: Config) -> Self {
        NativeHost {
            config,
            handles: HashMap::new(),
            next: 3,
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
            let root = self.config.mounts.get(name).ok_or(L::E_NOT_FOUND)?;
            return Ok(root.join(safe(sub)?));
        }
        if path.starts_with("/net/") {
            return Err(L::E_NOT_SUPPORTED);
        }
        Err(L::E_NOT_FOUND)
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
    let mut out = Vec::new();
    let mut ends = Vec::new();
    for (name, size, is_dir) in entries {
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.push(is_dir as u8);
        ends.push(out.len());
    }
    Ok((out, ends))
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
            Some(_) => L::E_PERMISSION,
        }
    }

    fn close(&mut self, handle: i32) -> i32 {
        match self.handles.remove(&handle) {
            Some(_) => 0,
            None => L::E_BAD_HANDLE,
        }
    }
}
