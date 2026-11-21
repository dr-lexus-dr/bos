//! Short-lived DNS and TCP sockets driven by the same DHCP/ARP poller as bSH.
use super::*;
use crate::web::http::{Receiver, Response, Result, Url};
use smoltcp::{
    socket::{dns, tcp},
    time::Duration,
    wire::DnsQueryType,
};

pub fn fetch(url: &Url, cancel: &mut dyn FnMut() -> bool) -> Result<Response> {
    request(
        url,
        "GET",
        "Accept: text/html,text/plain\n",
        &[],
        false,
        cancel,
    )
}

pub fn request(
    url: &Url,
    method: &str,
    headers: &str,
    body: &[u8],
    resource: bool,
    cancel: &mut dyn FnMut() -> bool,
) -> Result<Response> {
    if !matches!(
        method,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    ) || headers.len() > 8192
        || body.len() > 1 << 20
    {
        return Err("Invalid HTTP request");
    }
    let encoding = if resource {
        "gzip, br, deflate"
    } else {
        "identity"
    };
    let mut head = alloc::format!("{method} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: bOS/{}\r\nAccept-Encoding: {encoding}\r\nConnection: close\r\n",url.path,url.authority(),env!("CARGO_PKG_VERSION"));
    for line in headers.lines() {
        let (key, value) = line.split_once(':').ok_or("Invalid request header")?;
        if ![
            "Accept",
            "Content-Type",
            "Cookie",
            "Origin",
            "Referer",
            "Authorization",
            "If-None-Match",
        ]
        .iter()
        .any(|k| key.eq_ignore_ascii_case(k))
            || value.chars().any(char::is_control)
        {
            return Err("Invalid request header");
        }
        head.push_str(line);
        head.push_str("\r\n");
    }
    if !body.is_empty() || method == "POST" {
        head.push_str(&alloc::format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    let mut request = head.into_bytes();
    request.extend_from_slice(body);
    // A browser request may yield to the compositor. Own the interface until
    // the sockets are removed and restore it even on timeout/cancellation.
    struct Lease(Option<Network>);
    impl Drop for Lease {
        fn drop(&mut self) {
            *NETWORK.lock() = self.0.take();
        }
    }
    let net = NETWORK
        .lock()
        .take()
        .ok_or("Network adapter is busy or unavailable")?;
    LINK_WHILE_FETCHING.store(
        net.link && net.address.is_some(),
        core::sync::atomic::Ordering::Relaxed,
    );
    let mut lease = Lease(Some(net));
    let net = lease.0.as_mut().unwrap();
    let deadline = net.clock.now() + Duration::from_secs(20);
    while net.address.is_none() {
        step(net, deadline, cancel)?;
    }
    let address = resolve(net, &url.host, deadline, cancel)?;
    let rx = if resource { 128 * 1024 } else { 16 * 1024 };
    let mut socket = tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0; rx]),
        tcp::SocketBuffer::new(vec![0; 32768]),
    );
    socket.set_ack_delay(None);
    socket.set_nagle_enabled(false);
    let port = 49152 + (rdtsc() as u16 & 0x3fff);
    socket
        .connect(net.iface.context(), (address, url.port), port)
        .map_err(|_| "TCP connect failed")?;
    let handle = net.sockets.add(socket);
    let mut stream = Stream {
        net,
        handle,
        deadline,
        cancel,
    };
    loop {
        stream.step()?;
        match stream.socket().state() {
            tcp::State::Established | tcp::State::CloseWait => break,
            tcp::State::SynSent => {}
            _ => return Err("TCP connection refused"),
        }
    }
    let mut parser = if resource {
        Receiver::resource()
    } else {
        Receiver::new()
    };
    if url.tls {
        return super::tls::request(&mut stream, &url.host, &request, parser);
    }
    stream.write(&request)?;
    let mut bytes = [0; 2048];
    while !parser.done() {
        let n = stream.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        parser.push(&bytes[..n])?;
    }
    parser.finish(true)
}

fn step(net: &mut Network, end: Instant, cancel: &mut dyn FnMut() -> bool) -> Result<()> {
    if cancel() {
        return Err("Navigation cancelled");
    }
    let now = net.clock.now();
    if now >= end {
        return Err("Network request timed out");
    }
    net.poll(now);
    if !net.link && !net.device.nic.link_up() {
        return Err("Ethernet link is down");
    }
    core::hint::spin_loop();
    Ok(())
}

fn resolve(
    net: &mut Network,
    host: &str,
    end: Instant,
    cancel: &mut dyn FnMut() -> bool,
) -> Result<Ipv4Address> {
    if let Ok(ip) = host.parse() {
        return Ok(ip);
    }
    if net.dns.is_empty() {
        return Err("No DNS server configured");
    }
    let servers: Vec<_> = net.dns.iter().copied().map(IpAddress::Ipv4).collect();
    let mut socket = dns::Socket::new(&servers, Vec::new());
    let query = socket
        .start_query(net.iface.context(), host, DnsQueryType::A)
        .map_err(|_| "Invalid DNS name")?;
    let handle = net.sockets.add(socket);
    let result = loop {
        if let Err(error) = step(net, end, cancel) {
            break Err(error);
        }
        match net
            .sockets
            .get_mut::<dns::Socket>(handle)
            .get_query_result(query)
        {
            Ok(ips) => {
                break ips
                    .iter()
                    .map(|ip| {
                        let IpAddress::Ipv4(ip) = ip;
                        *ip
                    })
                    .next()
                    .ok_or("No IPv4 address for host")
            }
            Err(dns::GetQueryResultError::Pending) => {}
            Err(_) => break Err("DNS lookup failed"),
        }
    };
    net.sockets.remove(handle);
    result
}

pub(super) struct Stream<'a> {
    net: &'a mut Network,
    handle: SocketHandle,
    deadline: Instant,
    cancel: &'a mut dyn FnMut() -> bool,
}

impl Stream<'_> {
    fn socket(&mut self) -> &mut tcp::Socket<'static> {
        self.net.sockets.get_mut(self.handle)
    }
    fn step(&mut self) -> Result<()> {
        step(self.net, self.deadline, self.cancel)
    }

    pub(super) fn read(&mut self, dest: &mut [u8]) -> Result<usize> {
        loop {
            self.step()?;
            let socket = self.socket();
            if socket.can_recv() {
                return socket.recv_slice(dest).map_err(|_| "TCP receive failed");
            }
            if !socket.may_recv() {
                return Ok(0);
            }
        }
    }

    pub(super) fn write(&mut self, mut src: &[u8]) -> Result<()> {
        while !src.is_empty() {
            self.step()?;
            let n = self
                .socket()
                .send_slice(src)
                .map_err(|_| "TCP send failed")?;
            src = &src[n..];
        }
        Ok(())
    }
}

impl Drop for Stream<'_> {
    fn drop(&mut self) {
        self.socket().abort();
        let now = self.net.clock.now();
        self.net
            .iface
            .poll_egress(now, &mut self.net.device, &mut self.net.sockets);
        self.net.sockets.remove(self.handle);
    }
}
