//! A minimal in-memory 9P2000 file server for offline tests. The tree is
//! `hello.txt` (`hello\n`) and `sub/a.txt` (`a`); files may be created and
//! written. Anything else is answered with an Rerror.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

enum Node {
    File(Vec<u8>),
    Dir,
}

type Tree = Arc<Mutex<BTreeMap<String, Node>>>;

pub fn start() -> SocketAddr {
    let mut t = BTreeMap::new();
    t.insert(String::new(), Node::Dir);
    t.insert("hello.txt".into(), Node::File(b"hello\n".to_vec()));
    t.insert("sub".into(), Node::Dir);
    t.insert("sub/a.txt".into(), Node::File(b"a".to_vec()));
    let tree: Tree = Arc::new(Mutex::new(t));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tree = tree.clone();
            std::thread::spawn(move || serve(stream, tree));
        }
    });
    addr
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn qid(tree: &BTreeMap<String, Node>, path: &str) -> Vec<u8> {
    let dir = matches!(tree.get(path), Some(Node::Dir));
    let id = tree.keys().position(|k| k == path).unwrap_or(0) as u64;
    let mut q = vec![if dir { 0x80 } else { 0 }];
    q.extend_from_slice(&0u32.to_le_bytes());
    q.extend_from_slice(&id.to_le_bytes());
    q
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn stat(tree: &BTreeMap<String, Node>, path: &str) -> Vec<u8> {
    let (len, mode) = match tree.get(path) {
        Some(Node::File(b)) => (b.len() as u64, 0o644u32),
        _ => (0, 0x8000_0000 | 0o755),
    };
    let name = path.rsplit('/').next().unwrap_or("");
    let mut s = Vec::new();
    s.extend_from_slice(&0u16.to_le_bytes());
    s.extend_from_slice(&0u32.to_le_bytes());
    s.extend_from_slice(&qid(tree, path));
    s.extend_from_slice(&mode.to_le_bytes());
    s.extend_from_slice(&0u32.to_le_bytes());
    s.extend_from_slice(&0u32.to_le_bytes());
    s.extend_from_slice(&len.to_le_bytes());
    for field in [
        if path.is_empty() { "/" } else { name },
        "chasm",
        "chasm",
        "chasm",
    ] {
        put_str(&mut s, field);
    }
    let mut out = (s.len() as u16).to_le_bytes().to_vec();
    out.extend_from_slice(&s);
    out
}

fn children(tree: &BTreeMap<String, Node>, dir: &str) -> Vec<String> {
    tree.keys()
        .filter(|k| {
            !k.is_empty()
                && match k.rsplit_once('/') {
                    Some((parent, _)) => parent == dir,
                    None => dir.is_empty(),
                }
        })
        .cloned()
        .collect()
}

struct Msg<'a> {
    b: &'a [u8],
    at: usize,
}

impl Msg<'_> {
    fn n(&mut self, k: usize) -> u64 {
        let mut v = 0u64;
        for i in 0..k {
            v |= (self.b[self.at + i] as u64) << (8 * i);
        }
        self.at += k;
        v
    }
    fn s(&mut self) -> String {
        let len = self.n(2) as usize;
        let s = String::from_utf8_lossy(&self.b[self.at..self.at + len]).into_owned();
        self.at += len;
        s
    }
}

fn serve(mut stream: TcpStream, tree: Tree) {
    let mut fids: HashMap<u32, String> = HashMap::new();
    loop {
        let mut size = [0u8; 4];
        if stream.read_exact(&mut size).is_err() {
            return;
        }
        let mut msg = vec![0u8; u32::from_le_bytes(size) as usize - 4];
        if stream.read_exact(&mut msg).is_err() {
            return;
        }
        let (ty, tag) = (msg[0], u16::from_le_bytes([msg[1], msg[2]]));
        let mut m = Msg {
            b: &msg[3..],
            at: 0,
        };
        let mut t = tree.lock().unwrap();
        let reply: Result<Vec<u8>, &str> = match ty {
            100 => {
                let msize = (m.n(4) as u32).min(8192);
                let mut r = msize.to_le_bytes().to_vec();
                put_str(&mut r, "9P2000");
                Ok(r)
            }
            104 => {
                let fid = m.n(4) as u32;
                fids.insert(fid, String::new());
                Ok(qid(&t, ""))
            }
            110 => {
                let (fid, newfid, n) = (m.n(4) as u32, m.n(4) as u32, m.n(2));
                let mut path = fids.get(&fid).cloned().unwrap_or_default();
                let mut qids = Vec::new();
                let mut walked = 0;
                for _ in 0..n {
                    let next = join(&path, &m.s());
                    if !t.contains_key(&next) {
                        break;
                    }
                    path = next;
                    qids.extend_from_slice(&qid(&t, &path));
                    walked += 1;
                }
                if walked == 0 && n > 0 {
                    Err("file not found")
                } else {
                    if walked == n {
                        fids.insert(newfid, path);
                    }
                    let mut r = (walked as u16).to_le_bytes().to_vec();
                    r.extend_from_slice(&qids);
                    Ok(r)
                }
            }
            112 => {
                let (fid, mode) = (m.n(4) as u32, m.n(1) as u8);
                let path = fids.get(&fid).cloned().unwrap_or_default();
                if mode & 0x10 != 0 {
                    if let Some(Node::File(b)) = t.get_mut(&path) {
                        b.clear();
                    }
                }
                let mut r = qid(&t, &path);
                r.extend_from_slice(&0u32.to_le_bytes());
                Ok(r)
            }
            114 => {
                let fid = m.n(4) as u32;
                let name = m.s();
                let path = join(&fids.get(&fid).cloned().unwrap_or_default(), &name);
                t.insert(path.clone(), Node::File(Vec::new()));
                fids.insert(fid, path.clone());
                let mut r = qid(&t, &path);
                r.extend_from_slice(&0u32.to_le_bytes());
                Ok(r)
            }
            116 => {
                let (fid, offset, count) = (m.n(4) as u32, m.n(8) as usize, m.n(4) as usize);
                let path = fids.get(&fid).cloned().unwrap_or_default();
                let data = match t.get(&path) {
                    Some(Node::File(b)) => {
                        b[offset.min(b.len())..(offset + count).min(b.len())].to_vec()
                    }
                    _ => {
                        let all: Vec<u8> = children(&t, &path)
                            .iter()
                            .flat_map(|c| stat(&t, c))
                            .collect();
                        let mut end = offset.min(all.len());
                        let mut at = end;
                        while at + 2 <= all.len() {
                            let rec = 2 + u16::from_le_bytes([all[at], all[at + 1]]) as usize;
                            if at + rec - offset > count {
                                break;
                            }
                            at += rec;
                            end = at;
                        }
                        all[offset.min(all.len())..end].to_vec()
                    }
                };
                let mut r = (data.len() as u32).to_le_bytes().to_vec();
                r.extend_from_slice(&data);
                Ok(r)
            }
            118 => {
                let (fid, offset, count) = (m.n(4) as u32, m.n(8) as usize, m.n(4) as usize);
                let data = m.b[m.at..m.at + count].to_vec();
                let path = fids.get(&fid).cloned().unwrap_or_default();
                match t.get_mut(&path) {
                    Some(Node::File(b)) => {
                        if b.len() < offset + count {
                            b.resize(offset + count, 0);
                        }
                        b[offset..offset + count].copy_from_slice(&data);
                        Ok((count as u32).to_le_bytes().to_vec())
                    }
                    _ => Err("permission denied"),
                }
            }
            120 => {
                fids.remove(&(m.n(4) as u32));
                Ok(Vec::new())
            }
            124 => {
                let path = fids.get(&(m.n(4) as u32)).cloned().unwrap_or_default();
                let s = stat(&t, &path);
                let mut r = (s.len() as u16).to_le_bytes().to_vec();
                r.extend_from_slice(&s);
                Ok(r)
            }
            _ => Err("not supported"),
        };
        drop(t);
        let (rty, body) = match reply {
            Ok(b) => (ty + 1, b),
            Err(e) => {
                let mut b = Vec::new();
                put_str(&mut b, e);
                (107, b)
            }
        };
        let mut out = ((7 + body.len()) as u32).to_le_bytes().to_vec();
        out.push(rty);
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&body);
        if stream.write_all(&out).is_err() {
            return;
        }
    }
}
