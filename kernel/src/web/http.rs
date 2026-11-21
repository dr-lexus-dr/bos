//! Bounded HTTP/1.x framing shared by the console and the graphical reader.
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

pub type Result<T> = core::result::Result<T, &'static str>;
pub const URL_LIMIT: usize = 1024;
const HEAD_LIMIT: usize = 8192;
const BODY_LIMIT: usize = 65536;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Url {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.len() > URL_LIMIT
            || input
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
        {
            return Err("Invalid or overlong URL");
        }
        let input = input.split('#').next().unwrap_or("");
        let (tls, rest) = match input.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => (true, rest),
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => (false, rest),
            Some(_) => return Err("Only HTTP and HTTPS are supported"),
            None => (true, input),
        };
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let authority = &rest[..end];
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (
                host,
                port.parse::<u16>()
                    .ok()
                    .filter(|p| *p > 0)
                    .ok_or("Invalid port")?,
            ),
            None => (authority, if tls { 443 } else { 80 }),
        };
        if host.len() > 253
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            })
        {
            return Err("Invalid host name");
        }
        let tail = &rest[end..];
        let mut path = String::new();
        if !tail.starts_with('/') {
            path.push('/');
        }
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        for byte in tail.bytes() {
            if byte.is_ascii() {
                path.push(byte as char);
            } else {
                path.push('%');
                path.push(HEX[usize::from(byte >> 4)] as char);
                path.push(HEX[usize::from(byte & 15)] as char);
            }
        }
        if host.len() + path.len() + 16 > URL_LIMIT {
            return Err("URL is too long");
        }
        Ok(Self {
            tls,
            host: host.to_ascii_lowercase(),
            port,
            path,
        })
    }

    pub fn authority(&self) -> String {
        if self.port == if self.tls { 443 } else { 80 } {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub fn address(&self) -> String {
        format!(
            "{}://{}{}",
            if self.tls { "https" } else { "http" },
            self.authority(),
            self.path
        )
    }

    pub fn join(&self, reference: &str) -> Result<Self> {
        let reference = reference.trim();
        if reference.len() > URL_LIMIT {
            return Err("Link is too long");
        }
        let scheme = if self.tls { "https" } else { "http" };
        if reference.starts_with("//") {
            return Self::parse(&format!("{scheme}:{reference}"));
        }
        if reference
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .contains(':')
        {
            return if reference.contains("://") {
                Self::parse(reference)
            } else {
                Err("Unsupported link scheme")
            };
        }
        let reference = reference.split('#').next().unwrap_or("");
        if reference.is_empty() {
            return Ok(self.clone());
        }
        let base = self.path.split('?').next().unwrap_or("/");
        let path = if reference.starts_with('?') {
            format!("{base}{reference}")
        } else if reference.starts_with('/') {
            reference.to_string()
        } else {
            format!(
                "{}/{}",
                base.rsplit_once('/').map_or("", |v| v.0),
                reference
            )
        };
        let (part, query) = path
            .split_once('?')
            .map_or((path.as_str(), None), |(p, q)| (p, Some(q)));
        let mut segments = Vec::new();
        for segment in part[1..].split('/') {
            match segment {
                ".." => {
                    segments.pop();
                }
                "." => {}
                s => segments.push(s),
            }
        }
        if part.ends_with("/.") || part.ends_with("/..") {
            segments.push("");
        }
        let mut normalized = format!("/{}", segments.join("/"));
        if let Some(query) = query {
            normalized.push('?');
            normalized.push_str(query);
        }
        Self::parse(&format!("{scheme}://{}{normalized}", self.authority()))
    }
}

pub struct Response {
    pub status: u16,
    pub headers: String,
    pub location: Option<String>,
    pub html: bool,
    pub body: Vec<u8>,
    pub clipped: bool,
}

enum Body {
    Headers,
    Close,
    Fixed(usize),
    ChunkSize,
    Chunk(usize),
    ChunkCr,
    ChunkLf,
    Trailers,
    End,
}

pub struct Receiver {
    state: Body,
    line: Vec<u8>,
    trailers: usize,
    response: Response,
    limit: usize,
    resource: bool,
}

impl Receiver {
    pub fn new() -> Self {
        Self::with_limit(BODY_LIMIT, false)
    }
    pub fn resource() -> Self {
        Self::with_limit(8 * 1024 * 1024, true)
    }
    fn with_limit(limit: usize, resource: bool) -> Self {
        Self {
            limit,
            resource,
            state: Body::Headers,
            line: Vec::new(),
            trailers: 0,
            response: Response {
                status: 0,
                headers: String::new(),
                location: None,
                html: true,
                body: Vec::new(),
                clipped: false,
            },
        }
    }

    fn headers(&mut self) -> Result<()> {
        let head = core::str::from_utf8(&self.line).map_err(|_| "Invalid HTTP headers")?;
        let mut lines = head.split("\r\n");
        let mut status = lines.next().unwrap_or("").split_whitespace();
        if !matches!(status.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
            return Err("Invalid HTTP response");
        }
        self.response.status = status
            .next()
            .and_then(|s| s.parse().ok())
            .filter(|n| (100..=599).contains(n))
            .ok_or("Invalid HTTP status")?;
        self.response.headers.clear();
        let mut length = None;
        let mut chunked = false;
        let mut media = None;
        for line in lines.take_while(|s| !s.is_empty()) {
            let (name, value) = line.split_once(':').ok_or("Invalid HTTP header")?;
            let value = value.trim();
            if self.resource {
                self.response.headers.push_str(line);
                self.response.headers.push('\n');
            }
            if name.eq_ignore_ascii_case("content-length") {
                let n = value
                    .parse::<usize>()
                    .map_err(|_| "Invalid Content-Length")?;
                if length.replace(n).is_some() {
                    return Err("Duplicate Content-Length");
                }
            } else if name.eq_ignore_ascii_case("transfer-encoding") {
                if chunked || !value.eq_ignore_ascii_case("chunked") {
                    return Err("Unsupported transfer encoding");
                }
                chunked = true;
            } else if name.eq_ignore_ascii_case("location") {
                if value.len() > URL_LIMIT {
                    return Err("Redirect URL too long");
                }
                self.response.location = Some(value.to_string());
            } else if name.eq_ignore_ascii_case("content-type") {
                media = Some(
                    value
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase(),
                );
            } else if name.eq_ignore_ascii_case("content-encoding")
                && !self.resource
                && !value.eq_ignore_ascii_case("identity")
            {
                return Err("Compressed HTTP content is unsupported");
            }
        }
        if self.response.status < 200 {
            if self.response.status == 101 {
                return Err("HTTP upgrade unsupported");
            }
            self.response.location = None;
            self.state = Body::Headers;
        } else if matches!(self.response.status, 204 | 304) || self.redirect() {
            self.state = Body::End;
        } else {
            self.response.html = match media.as_deref() {
                None | Some("text/html" | "application/xhtml+xml") => true,
                Some(s) if s.starts_with("text/") => false,
                _ if self.resource => false,
                _ => return Err("This browser opens HTML and text documents"),
            };
            self.state = match (chunked, length) {
                (true, Some(_)) => return Err("Conflicting HTTP framing"),
                (true, None) => Body::ChunkSize,
                (false, Some(0)) => Body::End,
                (false, Some(n)) => {
                    self.response
                        .body
                        .try_reserve_exact(n.min(self.limit))
                        .map_err(|_| "Insufficient memory for the page")?;
                    Body::Fixed(n)
                }
                _ => Body::Close,
            };
        }
        self.line.clear();
        Ok(())
    }

    fn redirect(&self) -> bool {
        matches!(self.response.status, 301 | 302 | 303 | 307 | 308)
            && self.response.location.is_some()
    }

    fn append_body(&mut self, bytes: &[u8], remaining: usize) -> usize {
        let room = self.limit - self.response.body.len();
        let take = bytes.len().min(remaining).min(room);
        self.response.body.extend_from_slice(&bytes[..take]);
        self.response.clipped = take == room && remaining > room;
        take
    }

    pub fn push(&mut self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() && !self.done() {
            match self.state {
                Body::Headers | Body::ChunkSize | Body::Trailers => {
                    self.line.push(bytes[0]);
                    bytes = &bytes[1..];
                    if self.line.len() > HEAD_LIMIT {
                        return Err("HTTP header or chunk line too long");
                    }
                    match self.state {
                        Body::Headers if self.line.ends_with(b"\r\n\r\n") => self.headers()?,
                        Body::ChunkSize if self.line.ends_with(b"\r\n") => {
                            let size = self
                                .line
                                .split(|&b| b == b';' || b == b'\r')
                                .next()
                                .unwrap_or(&[]);
                            let n = core::str::from_utf8(size)
                                .ok()
                                .and_then(|s| usize::from_str_radix(s, 16).ok())
                                .ok_or("Invalid chunk length")?;
                            self.response.clipped =
                                n != 0 && self.response.body.len() == self.limit;
                            self.state = if n == 0 {
                                Body::Trailers
                            } else {
                                Body::Chunk(n)
                            };
                            self.line.clear();
                        }
                        Body::Trailers if self.line.ends_with(b"\r\n") => {
                            self.trailers += self.line.len();
                            if self.trailers > HEAD_LIMIT {
                                return Err("Trailers too long");
                            }
                            if self.line.len() == 2 {
                                self.state = Body::End;
                            }
                            self.line.clear();
                        }
                        _ => {}
                    }
                }
                Body::Fixed(n) | Body::Chunk(n) => {
                    let take = self.append_body(bytes, n);
                    bytes = &bytes[take..];
                    self.state = match (&self.state, n - take) {
                        (Body::Fixed(_), 0) => Body::End,
                        (Body::Fixed(_), left) => Body::Fixed(left),
                        (_, 0) => Body::ChunkCr,
                        (_, left) => Body::Chunk(left),
                    };
                }
                Body::ChunkCr | Body::ChunkLf => {
                    let cr = matches!(self.state, Body::ChunkCr);
                    if bytes[0] != if cr { b'\r' } else { b'\n' } {
                        return Err("Invalid chunk boundary");
                    }
                    bytes = &bytes[1..];
                    self.state = if cr { Body::ChunkLf } else { Body::ChunkSize };
                }
                Body::Close => {
                    self.append_body(bytes, bytes.len());
                    break;
                }
                Body::End => break,
            }
        }
        Ok(())
    }

    pub fn done(&self) -> bool {
        self.response.clipped || matches!(self.state, Body::End)
    }

    pub fn finish(self, clean_eof: bool) -> Result<Response> {
        if self.done() || (clean_eof && matches!(self.state, Body::Close)) {
            Ok(self.response)
        } else {
            Err("Connection closed before the page was complete")
        }
    }
}
