//! A 9P2000 client over TCP: the operations the namespace needs to serve a
//! mounted file server (version, attach, walk, open, create, read, write,
//! clunk, stat). One request in flight at a time.

use std::io::{Read, Write};
use std::net::TcpStream;

const TVERSION: u8 = 100;
const TATTACH: u8 = 104;
const RERROR: u8 = 107;
const TWALK: u8 = 110;
const TOPEN: u8 = 112;
const TCREATE: u8 = 114;
const TREAD: u8 = 116;
const TWRITE: u8 = 118;
const TCLUNK: u8 = 120;
const TSTAT: u8 = 124;

const NOTAG: u16 = 0xFFFF;
const NOFID: u32 = 0xFFFF_FFFF;
const TAG: u16 = 1;
/// Walk at most this many names per message (the protocol's MAXWELEM).
const MAXWELEM: usize = 16;
/// Room for a read or write header within msize.
const IOHDR: u32 = 24;

pub const OREAD: u8 = 0;
pub const OWRITE: u8 = 1;
pub const ORDWR: u8 = 2;
pub const OTRUNC: u8 = 0x10;
const QTDIR: u8 = 0x80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Qid {
    pub ty: u8,
    pub version: u32,
    pub path: u64,
}

impl Qid {
    pub fn is_dir(&self) -> bool {
        self.ty & QTDIR != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    pub name: String,
    pub length: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    NotFound,
    Permission,
    Io(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

/// Little-endian 9P fields out of a reply.
struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let s = self
            .b
            .get(self.at..self.at + n)
            .ok_or_else(|| Error::Io("short 9p message".into()))?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn str(&mut self) -> Result<String, Error> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn qid(&mut self) -> Result<Qid, Error> {
        Ok(Qid {
            ty: self.u8()?,
            version: self.u32()?,
            path: self.u64()?,
        })
    }
    fn stat(&mut self) -> Result<Stat, Error> {
        let size = self.u16()? as usize;
        let end = self.at + size;
        self.take(2 + 4)?; // type, dev
        let qid = self.qid()?;
        let mode = self.u32()?;
        self.take(4 + 4)?; // atime, mtime
        let length = self.u64()?;
        let name = self.str()?;
        self.at = end;
        Ok(Stat {
            name,
            length,
            is_dir: qid.is_dir() || mode & 0x8000_0000 != 0,
        })
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

pub struct Client {
    stream: TcpStream,
    msize: u32,
    next_fid: u32,
}

impl Client {
    /// Connect to `host:port`, negotiate 9P2000 and attach to the root (fid 0).
    pub fn connect(addr: &str) -> std::io::Result<Client> {
        let stream = TcpStream::connect(addr)?;
        let mut c = Client {
            stream,
            msize: 8192,
            next_fid: 1,
        };
        let io = |e: Error| std::io::Error::other(format!("{e:?}"));
        let mut body = 8192u32.to_le_bytes().to_vec();
        put_str(&mut body, "9P2000");
        let r = c.rpc_tag(TVERSION, NOTAG, &body).map_err(io)?;
        let mut cur = Cursor { b: &r, at: 0 };
        let msize = cur.u32().map_err(io)?;
        if cur.str().map_err(io)? != "9P2000" {
            return Err(std::io::Error::other("server does not speak 9P2000"));
        }
        c.msize = msize.min(8192);
        let mut body = 0u32.to_le_bytes().to_vec();
        body.extend_from_slice(&NOFID.to_le_bytes());
        put_str(&mut body, "wack");
        put_str(&mut body, "");
        c.rpc(TATTACH, &body).map_err(io)?;
        Ok(c)
    }

    fn rpc(&mut self, ty: u8, body: &[u8]) -> Result<Vec<u8>, Error> {
        self.rpc_tag(ty, TAG, body)
    }

    fn rpc_tag(&mut self, ty: u8, tag: u16, body: &[u8]) -> Result<Vec<u8>, Error> {
        let mut msg = ((7 + body.len()) as u32).to_le_bytes().to_vec();
        msg.push(ty);
        msg.extend_from_slice(&tag.to_le_bytes());
        msg.extend_from_slice(body);
        self.stream.write_all(&msg)?;
        let mut size = [0u8; 4];
        self.stream.read_exact(&mut size)?;
        let size = u32::from_le_bytes(size) as usize;
        if !(7..=self.msize.max(8192) as usize + 64).contains(&size) {
            return Err(Error::Io(format!("bad 9p message size {size}")));
        }
        let mut rest = vec![0u8; size - 4];
        self.stream.read_exact(&mut rest)?;
        let (rty, body) = (rest[0], rest[3..].to_vec());
        if rty == RERROR {
            let msg = Cursor { b: &body, at: 0 }.str()?.to_lowercase();
            return Err(
                if ["not found", "does not exist", "no such"]
                    .iter()
                    .any(|p| msg.contains(p))
                {
                    Error::NotFound
                } else if msg.contains("permission") {
                    Error::Permission
                } else {
                    Error::Io(msg)
                },
            );
        }
        if rty != ty + 1 {
            return Err(Error::Io(format!("unexpected 9p reply {rty} to {ty}")));
        }
        Ok(body)
    }

    /// A new fid for `path` (`/`-separated, from the root).
    pub fn walk(&mut self, path: &str) -> Result<u32, Error> {
        let names: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let fid = self.next_fid;
        self.next_fid += 1;
        let mut from: u32 = 0;
        let mut chunks: Vec<&[&str]> = names.chunks(MAXWELEM).collect();
        if chunks.is_empty() {
            chunks.push(&[]);
        }
        for chunk in chunks {
            let mut body = from.to_le_bytes().to_vec();
            body.extend_from_slice(&fid.to_le_bytes());
            body.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
            for n in chunk {
                put_str(&mut body, n);
            }
            let r = self.rpc(TWALK, &body)?;
            if (Cursor { b: &r, at: 0 }.u16()? as usize) < chunk.len() {
                if from == fid {
                    let _ = self.clunk(fid);
                }
                return Err(Error::NotFound);
            }
            from = fid;
        }
        Ok(fid)
    }

    pub fn open(&mut self, fid: u32, mode: u8) -> Result<Qid, Error> {
        let mut body = fid.to_le_bytes().to_vec();
        body.push(mode);
        let r = self.rpc(TOPEN, &body)?;
        Cursor { b: &r, at: 0 }.qid()
    }

    /// Create `name` in directory `dirfid` and open it; `dirfid` then refers
    /// to the new file.
    pub fn create(&mut self, dirfid: u32, name: &str, perm: u32, mode: u8) -> Result<Qid, Error> {
        let mut body = dirfid.to_le_bytes().to_vec();
        put_str(&mut body, name);
        body.extend_from_slice(&perm.to_le_bytes());
        body.push(mode);
        let r = self.rpc(TCREATE, &body)?;
        Cursor { b: &r, at: 0 }.qid()
    }

    pub fn read(&mut self, fid: u32, offset: u64, count: u32) -> Result<Vec<u8>, Error> {
        let mut body = fid.to_le_bytes().to_vec();
        body.extend_from_slice(&offset.to_le_bytes());
        body.extend_from_slice(&count.min(self.msize - IOHDR).to_le_bytes());
        let r = self.rpc(TREAD, &body)?;
        let mut cur = Cursor { b: &r, at: 0 };
        let n = cur.u32()? as usize;
        Ok(cur.take(n)?.to_vec())
    }

    pub fn write(&mut self, fid: u32, offset: u64, data: &[u8]) -> Result<u32, Error> {
        let mut done = 0usize;
        for chunk in data.chunks((self.msize - IOHDR) as usize) {
            let mut body = fid.to_le_bytes().to_vec();
            body.extend_from_slice(&(offset + done as u64).to_le_bytes());
            body.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            body.extend_from_slice(chunk);
            let r = self.rpc(TWRITE, &body)?;
            let n = Cursor { b: &r, at: 0 }.u32()? as usize;
            done += n;
            if n < chunk.len() {
                break;
            }
        }
        Ok(done as u32)
    }

    pub fn clunk(&mut self, fid: u32) -> Result<(), Error> {
        self.rpc(TCLUNK, &fid.to_le_bytes()).map(|_| ())
    }

    pub fn stat(&mut self, fid: u32) -> Result<Stat, Error> {
        let r = self.rpc(TSTAT, &fid.to_le_bytes())?;
        let mut cur = Cursor { b: &r, at: 0 };
        cur.u16()?;
        cur.stat()
    }

    /// Every entry of an open directory.
    pub fn read_dir(&mut self, fid: u32) -> Result<Vec<Stat>, Error> {
        let mut bytes = Vec::new();
        loop {
            let chunk = self.read(fid, bytes.len() as u64, self.msize - IOHDR)?;
            if chunk.is_empty() {
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut cur = Cursor { b: &bytes, at: 0 };
        let mut out = Vec::new();
        while cur.at < bytes.len() {
            out.push(cur.stat()?);
        }
        Ok(out)
    }
}
