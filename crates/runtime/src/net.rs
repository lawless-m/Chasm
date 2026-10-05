//! `/net/http`: what a program writes to an open handle is the rest of an
//! HTTP request after the request line: `Name: value` lines, an empty line,
//! then the body. Nothing written is a GET; a non-empty body makes a POST.

use wack_core::layout as L;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Parse what was written to a handle. A header line without `: `, or a
/// header block with no terminating empty line, is `E_MALFORMED`.
pub fn parse_request(written: &[u8]) -> Result<Request, i32> {
    let mut headers = Vec::new();
    if written.is_empty() {
        return Ok(Request {
            method: Method::Get,
            headers,
            body: Vec::new(),
        });
    }
    let mut pos = 0;
    loop {
        let nl = written[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .ok_or(L::E_MALFORMED)?;
        let mut line = &written[pos..pos + nl];
        pos += nl + 1;
        if let Some(l) = line.strip_suffix(b"\r") {
            line = l;
        }
        if line.is_empty() {
            break;
        }
        let line = String::from_utf8_lossy(line);
        let (name, value) = line.split_once(": ").ok_or(L::E_MALFORMED)?;
        headers.push((name.to_string(), value.to_string()));
    }
    let body = written[pos..].to_vec();
    let method = if body.is_empty() {
        Method::Get
    } else {
        Method::Post
    };
    Ok(Request {
        method,
        headers,
        body,
    })
}

/// Send the request written to a handle and return the response body.
/// 404 is `E_NOT_FOUND`, 401 and 403 `E_PERMISSION`, any other non-2xx
/// status or a failed connection `E_IO`.
pub fn perform(url: &str, written: &[u8]) -> Result<Vec<u8>, i32> {
    let req = parse_request(written)?;
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build(),
    );
    let sent = match req.method {
        Method::Get => {
            let mut r = agent.get(url);
            for (name, value) in &req.headers {
                r = r.header(name, value);
            }
            r.call()
        }
        Method::Post => {
            let mut r = agent.post(url);
            for (name, value) in &req.headers {
                r = r.header(name, value);
            }
            r.send(&req.body[..])
        }
    };
    let mut resp = sent.map_err(|_| L::E_IO)?;
    match resp.status().as_u16() {
        200..=299 => resp
            .body_mut()
            .with_config()
            .limit(u64::MAX)
            .read_to_vec()
            .map_err(|_| L::E_IO),
        404 => Err(L::E_NOT_FOUND),
        401 | 403 => Err(L::E_PERMISSION),
        _ => Err(L::E_IO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn requests() {
        let r = parse_request(b"").unwrap();
        assert_eq!(
            (r.method, r.headers.len(), r.body.len()),
            (Method::Get, 0, 0)
        );
        let r = parse_request(b"\n").unwrap();
        assert_eq!((r.method, r.headers.len()), (Method::Get, 0));
        let r = parse_request(b"X-A: 1\n\n").unwrap();
        assert_eq!((r.method, r.headers), (Method::Get, h(&[("X-A", "1")])));
        let r = parse_request(b"X-A: 1\nX-B: two words\n\nbody").unwrap();
        assert_eq!(r.method, Method::Post);
        assert_eq!(r.headers, h(&[("X-A", "1"), ("X-B", "two words")]));
        assert_eq!(r.body, b"body");
        let r = parse_request(b"X-A: 1\r\n\r\nx").unwrap();
        assert_eq!(r.headers, h(&[("X-A", "1")]));
        let r = parse_request(b"\nbody").unwrap();
        assert_eq!((r.method, r.headers.len()), (Method::Post, 0));
    }

    #[test]
    fn malformed() {
        assert_eq!(parse_request(b"X-A: 1\nbody"), Err(-6));
        assert_eq!(parse_request(b"bad\n\n"), Err(-6));
        assert_eq!(parse_request(b"X-A: 1\n"), Err(-6));
    }
}
