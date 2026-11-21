pub mod document;
pub mod engine;
pub mod http;
pub mod network_task;

use alloc::{format, string::String};
use core::fmt::Write;
use document::Document;
use http::{Result, Url};

pub struct Page {
    pub url: Url,
    pub status: u16,
    pub doc: Document,
}

pub fn load(mut url: Url, columns: usize, cancel: &mut dyn FnMut() -> bool) -> Result<Page> {
    for _ in 0..=4 {
        let response = crate::drivers::net::fetch(&url, cancel)?;
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            if let Some(next) = response.location {
                let next = url.join(&next)?;
                if url.tls && !next.tls {
                    return Err("HTTPS redirect to insecure HTTP blocked");
                }
                url = next;
                continue;
            }
        }
        let mut doc = document::render(&response.body, response.html, columns);
        doc.clipped |= response.clipped;
        return Ok(Page {
            url,
            status: response.status,
            doc,
        });
    }
    Err("Too many redirects")
}

pub fn command(args: &[&str], out: &mut String) -> i32 {
    let [address] = args else {
        out.push_str("usage: web <URL>\r\n");
        return 2;
    };
    match Url::parse(address).and_then(|url| load(url, 78, &mut || false)) {
        Ok(page) => {
            let _ = writeln!(out, "HTTP {} {}\r", page.status, page.url.address());
            for row in page.doc.rows {
                out.push_str(&row.text);
                out.push_str("\r\n");
            }
            if page.doc.clipped {
                out.push_str("[display limit reached]\r\n");
            }
            i32::from(page.status >= 400)
        }
        Err(error) => {
            out.push_str(&format!("web: {error}\r\n"));
            1
        }
    }
}
