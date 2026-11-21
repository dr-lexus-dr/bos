extern crate alloc;
#[path = "../kernel/src/web/document.rs"]
mod document;
#[path = "../kernel/src/web/http.rs"]
mod http;
use http::{Receiver, Url};

#[test]
fn resolves_links_and_keeps_request_lines_well_formed() {
    let url = Url::parse("https://example.org:8443/news/today?q=abc#top").unwrap();
    assert_eq!(
        url.join("../old?p=1#top").unwrap().address(),
        "https://example.org:8443/old?p=1"
    );
    assert_eq!(url.join("?next").unwrap().path, "/news/today?next");
    assert_eq!(
        url.join("//example.net/test").unwrap().address(),
        "https://example.net/test"
    );
    assert_eq!(url.join("/я").unwrap().path, "/%D1%8F");
    for bad in [
        "http://bad\r\nheader/",
        "https://name@host/",
        "file:///path",
        "https://host:0",
    ] {
        assert!(Url::parse(bad).is_err());
    }
    assert!(url.join("javascript:alert(1)").is_err());
}

#[test]
fn frames_chunked_http_across_arbitrary_record_boundaries() {
    let mut rx = Receiver::new();
    for b in b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n1;ext=y\r\nc\r\n0\r\nX-test: yes\r\n\r\n" {
        rx.push(&[*b]).unwrap();
    }
    assert_eq!(rx.finish(false).unwrap().body, b"abc");
}

#[test]
fn rejects_conflicting_and_truncated_responses() {
    for head in [
        "Content-Length: 2\r\nContent-Length: 2",
        "Content-Length: 0\r\nTransfer-Encoding: chunked",
    ] {
        assert!(Receiver::new()
            .push(format!("HTTP/1.1 200 OK\r\n{head}\r\n\r\n").as_bytes())
            .is_err());
    }
    let mut rx = Receiver::new();
    rx.push(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nabc")
        .unwrap();
    assert!(rx.finish(true).is_err());
    let mut rx = Receiver::new();
    rx.push(b"HTTP/1.0 200 OK\r\n\r\nabc").unwrap();
    assert!(rx.finish(false).is_err()); // TLS close without close_notify or framing
}

#[test]
fn displays_a_bounded_prefix_of_long_documents() {
    let body = vec![b'x'; 65536];
    for (header, final_bytes) in [
        ("Content-Length: 65537\r\n\r\n", ""),
        ("Transfer-Encoding: chunked\r\n\r\n10001\r\n", ""),
        ("Transfer-Encoding: chunked\r\n\r\n10000\r\n", "\r\n1\r\n"),
        ("\r\n", "x"),
    ] {
        let mut rx = Receiver::new();
        rx.push(format!("HTTP/1.1 200 OK\r\n{header}").as_bytes())
            .unwrap();
        for chunk in body.chunks(1000) {
            rx.push(chunk).unwrap();
        }
        rx.push(final_bytes.as_bytes()).unwrap();
        assert!(rx.done());
        let page = rx.finish(false).unwrap();
        assert!(page.clipped);
        assert_eq!(page.body, body);
    }
    let mut rx = Receiver::new();
    rx.push(b"HTTP/1.1 200 OK\r\nContent-Length: 65536\r\n\r\n")
        .unwrap();
    rx.push(&body).unwrap();
    assert!(!rx.finish(false).unwrap().clipped);
}

#[test]
fn renders_unicode_text_and_only_visible_links() {
    let html="<head><title>Новости &amp; web</title></head><style>.hide{}</style><h1>Привет</h1><script>if(a<b) test()</script><a class='wide >' HREF='../ещё?a=1&amp;b=2'>Ссылка</a>";
    let doc = document::render(html.as_bytes(), true, 64);
    assert_eq!(doc.title, "Новости & web");
    assert_eq!(doc.links, ["../ещё?a=1&b=2"]);
    assert_eq!(doc.rows.len(), 2);
    assert!(doc.rows[0].heading);
    assert_eq!(doc.rows[1].text, "Ссылка");
    assert_eq!(doc.rows[1].link, Some(0));
    let doc = document::render("один\nдва".as_bytes(), false, 60);
    assert_eq!(doc.rows.len(), 2);
}

#[test]
fn limits_layout_for_long_pages() {
    let doc = document::render("<a href='/x'>x</a>".repeat(5000).as_bytes(), true, 50);
    assert!(doc.clipped);
    assert!(doc.links.len() <= 64 && doc.rows.len() <= 384);
}
