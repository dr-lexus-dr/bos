//! Ethernet and IPv4 for the built-in shell. All hardware access is polled.
use alloc::{boxed::Box, string::String, vec, vec::Vec};
use core::{fmt::Write, str::FromStr};
use smoltcp::{
    iface::{Config, Interface, SocketHandle, SocketSet},
    phy::{self, DeviceCapabilities, Medium},
    socket::{dhcpv4, icmp},
    time::Instant,
    wire::{EthernetAddress, Icmpv4Packet, Icmpv4Repr, IpAddress, Ipv4Address, Ipv4Cidr},
};
use spin::Mutex;
use x86_64::instructions::port::Port;

use super::{e1000, pci, rtl8139};
use crate::arch::x86_64::rdtsc;

pub(super) const FRAME_CAPACITY: usize = 1514;

pub(super) struct Frame {
    storage: [u8; FRAME_CAPACITY],
    len: usize,
}

impl Frame {
    pub(super) fn new(len: usize) -> Self {
        Self {
            storage: [0; FRAME_CAPACITY],
            len,
        }
    }

    pub(super) fn bytes(&mut self) -> &mut [u8] {
        &mut self.storage[..self.len]
    }
}

pub(super) trait Nic: Send {
    fn name(&self) -> &'static str;
    fn mac(&self) -> [u8; 6];
    fn link_up(&self) -> bool;
    fn can_send(&self) -> bool;
    fn send(&mut self, frame: &[u8]);
    fn receive(&mut self) -> Option<Frame>;
}

struct Ethernet {
    nic: Box<dyn Nic>,
}

impl Ethernet {
    fn open() -> Result<Self, &'static str> {
        let devices = pci::enumerate();
        let mut error = "no supported Ethernet adapter";
        if let Some(device) = devices.iter().copied().find(|dev| e1000::probe_pci(*dev)) {
            match e1000::Device::open(device) {
                Ok(nic) => return Ok(Self { nic: Box::new(nic) }),
                Err(reason) => error = reason,
            }
        }
        if let Some(device) = devices.iter().copied().find(|dev| rtl8139::probe_pci(*dev)) {
            match rtl8139::Device::open(device) {
                Ok(nic) => return Ok(Self { nic: Box::new(nic) }),
                Err(reason) => error = reason,
            }
        }
        Err(error)
    }
}

struct Transmit<'a>(&'a mut Ethernet);

impl phy::RxToken for Frame {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.storage[..self.len])
    }
}

impl phy::TxToken for Transmit<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut frame = Frame::new(len);
        let result = f(frame.bytes());
        self.0.nic.send(frame.bytes());
        result
    }
}

impl phy::Device for Ethernet {
    type RxToken<'a> = Frame;
    type TxToken<'a> = Transmit<'a>;

    fn receive(&mut self, _: Instant) -> Option<(Frame, Transmit<'_>)> {
        if !self.nic.can_send() {
            return None;
        }
        Some((self.nic.receive()?, Transmit(self)))
    }

    fn transmit(&mut self, _: Instant) -> Option<Transmit<'_>> {
        self.nic.can_send().then_some(Transmit(self))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.medium = Medium::Ethernet;
        capabilities.max_transmission_unit = FRAME_CAPACITY;
        capabilities
    }
}

struct Clock {
    epoch: u64,
    cycles_per_ms: u64,
}

impl Clock {
    fn start() -> Result<Self, &'static str> {
        // PIT channel 2, one shot, about 10 ms. Speaker output stays disabled.
        let cycles = unsafe {
            let mut gate = Port::<u8>::new(0x61);
            let previous = gate.read() & 3;
            gate.write(0);
            Port::<u8>::new(0x43).write(0xb0);
            let [low, high] = 11_932_u16.to_le_bytes();
            Port::<u8>::new(0x42).write(low);
            Port::<u8>::new(0x42).write(high);
            let start = rdtsc();
            gate.write(1);
            let finished = (0..1_000_000).any(|_| gate.read() & 0x20 != 0);
            let cycles = rdtsc().wrapping_sub(start);
            gate.write(previous);
            if !finished {
                return Err("PIT calibration timed out");
            }
            cycles
        };
        Ok(Self {
            epoch: rdtsc(),
            cycles_per_ms: (cycles / 10).max(1),
        })
    }

    fn now(&self) -> Instant {
        Instant::from_millis((rdtsc().wrapping_sub(self.epoch) / self.cycles_per_ms) as i64)
    }
}

struct Network {
    device: Ethernet,
    iface: Interface,
    sockets: SocketSet<'static>,
    dhcp: Option<SocketHandle>,
    address: Option<Ipv4Cidr>,
    router: Option<Ipv4Address>,
    dns: Vec<Ipv4Address>,
    link: bool,
    clock: Clock,
    last_poll: Instant,
    ping_id: u16,
}

static NETWORK: Mutex<Option<Network>> = Mutex::new(None);

pub fn init() {
    let mut slot = NETWORK.lock();
    if slot.is_none() {
        match Network::open() {
            Ok(network) => *slot = Some(network),
            Err(error) => log::info!("net: {error}"),
        }
    }
}

impl Network {
    fn open() -> Result<Self, &'static str> {
        let clock = Clock::start()?;
        let mut device = Ethernet::open()?;
        let mut config = Config::new(EthernetAddress(device.nic.mac()).into());
        config.random_seed = rdtsc();
        let now = clock.now();
        let iface = Interface::new(config, &mut device, now);
        let mut sockets = SocketSet::new(Vec::new());
        let dhcp = sockets.add(dhcpv4::Socket::new());
        Ok(Self {
            device,
            iface,
            sockets,
            dhcp: Some(dhcp),
            clock,
            address: None,
            router: None,
            dns: Vec::new(),
            link: false,
            last_poll: now,
            ping_id: 0,
        })
    }

    fn poll(&mut self, now: Instant) {
        if now <= self.last_poll {
            return;
        }
        self.last_poll = now;
        let link = self.device.nic.link_up();
        if self.link && !link {
            if let Some(handle) = self.dhcp {
                self.sockets.get_mut::<dhcpv4::Socket>(handle).reset();
                self.configure(None, None, &[]);
            } else {
                self.iface.update_ip_addrs(|_| {});
            }
        }
        self.link = link;
        if !link {
            return;
        }
        // Limit work per iteration so a flooded NIC can't starve keyboard input.
        for _ in 0..16 {
            if self
                .iface
                .poll_ingress_single(now, &mut self.device, &mut self.sockets)
                == smoltcp::iface::PollIngressSingleResult::None
            {
                break;
            }
        }
        self.iface
            .poll_egress(now, &mut self.device, &mut self.sockets);
        let Some(handle) = self.dhcp else { return };
        let lease =
            self.sockets
                .get_mut::<dhcpv4::Socket>(handle)
                .poll()
                .map(|event| match event {
                    dhcpv4::Event::Configured(cfg)
                        if host_address(cfg.address, cfg.address.address()) =>
                    {
                        let router = cfg
                            .router
                            .filter(|&router| host_address(cfg.address, router));
                        (Some(cfg.address), router, cfg.dns_servers)
                    }
                    dhcpv4::Event::Configured(_) | dhcpv4::Event::Deconfigured => {
                        (None, None, Default::default())
                    }
                });
        if let Some((address, router, dns)) = lease {
            self.configure(address, router, dns.as_slice());
        }
    }

    fn configure(
        &mut self,
        address: Option<Ipv4Cidr>,
        router: Option<Ipv4Address>,
        dns: &[Ipv4Address],
    ) {
        self.iface.update_ip_addrs(|addrs| {
            addrs.clear();
            if let Some(address) = address {
                let _ = addrs.push(address.into());
            }
        });
        self.iface.routes_mut().remove_default_ipv4_route();
        if let Some(router) = router {
            let _ = self.iface.routes_mut().add_default_ipv4_route(router);
        }
        self.address = address;
        self.router = router;
        self.dns.clear();
        self.dns.extend_from_slice(dns);
    }

    fn ping(&mut self, destination: Ipv4Address) -> Result<i64, &'static str> {
        if !self.device.nic.link_up() {
            return Err("eth0 link is down");
        }
        if self.address.is_none() {
            return Err("eth0 has no IPv4 address");
        }
        let buffer = || icmp::PacketBuffer::new(vec![icmp::PacketMetadata::EMPTY; 2], vec![0; 512]);
        let mut socket = icmp::Socket::new(buffer(), buffer());
        self.ping_id = self.ping_id.wrapping_add(1);
        let ident = self.ping_id;
        socket
            .bind(icmp::Endpoint::Ident(ident))
            .map_err(|_| "ICMP bind failed")?;
        let handle = self.sockets.add(socket);
        let start = self.clock.now();
        let mut sent = false;
        let payload = start.total_millis().to_be_bytes();
        let request = Icmpv4Repr::EchoRequest {
            ident,
            seq_no: 0,
            data: &payload,
        };
        let result = loop {
            let now = self.clock.now();
            if (now - start).total_millis() >= 3000 {
                break Err("request timed out");
            }
            self.poll(now);
            let socket = self.sockets.get_mut::<icmp::Socket>(handle);
            if !sent {
                let Ok(bytes) = socket.send(request.buffer_len(), IpAddress::Ipv4(destination))
                else {
                    break Err("ICMP send failed");
                };
                request.emit(&mut Icmpv4Packet::new_unchecked(bytes), &Default::default());
                sent = true;
            }
            if let Ok((bytes, source)) = socket.recv() {
                let reply = Icmpv4Packet::new_checked(bytes)
                    .ok()
                    .and_then(|packet| Icmpv4Repr::parse(&packet, &Default::default()).ok());
                if source == IpAddress::Ipv4(destination)
                    && reply
                        == Some(Icmpv4Repr::EchoReply {
                            ident,
                            seq_no: 0,
                            data: &payload,
                        })
                {
                    break Ok((now - start).total_millis() as i64);
                }
            }
            core::hint::spin_loop();
        };
        self.sockets.remove(handle);
        result
    }
}

pub fn poll() {
    if let Some(network) = NETWORK.lock().as_mut() {
        network.poll(network.clock.now());
    }
}

fn host_address(subnet: Ipv4Cidr, address: Ipv4Address) -> bool {
    !address.is_unspecified()
        && !address.is_loopback()
        && !address.is_multicast()
        && !address.is_broadcast()
        && subnet.contains_addr(&address)
        && subnet.broadcast() != Some(address)
        && (subnet.prefix_len() >= 31 || subnet.network().address() != address)
}

pub fn ifconfig(args: &[&str], out: &mut String) -> i32 {
    let mut slot = NETWORK.lock();
    let Some(network) = slot.as_mut() else {
        let _ = writeln!(out, "No supported Ethernet adapter");
        return 1;
    };
    match args {
        [] => {}
        ["dhcp"] => {
            if let Some(handle) = network.dhcp {
                network.sockets.get_mut::<dhcpv4::Socket>(handle).reset();
            } else {
                network.dhcp = Some(network.sockets.add(dhcpv4::Socket::new()));
            }
            network.configure(None, None, &[]);
        }
        ["static", cidr] | ["static", cidr, _] => {
            let Ok(address) = Ipv4Cidr::from_str(cidr) else {
                let _ = writeln!(out, "ifconfig: invalid IPv4 CIDR");
                return 2;
            };
            if !host_address(address, address.address()) {
                let _ = writeln!(out, "ifconfig: expected a host address");
                return 2;
            }
            let gateway = if let Some(value) = args.get(2) {
                let Ok(gateway) = Ipv4Address::from_str(value) else {
                    let _ = writeln!(out, "ifconfig: invalid gateway");
                    return 2;
                };
                if gateway == address.address() || !host_address(address, gateway) {
                    let _ = writeln!(out, "ifconfig: gateway must be another host on this subnet");
                    return 2;
                }
                Some(gateway)
            } else {
                None
            };
            if let Some(handle) = network.dhcp.take() {
                network.sockets.remove(handle);
            }
            network.configure(Some(address), gateway, &[]);
        }
        _ => {
            let _ = writeln!(
                out,
                "usage: ifconfig [dhcp | static <IPv4/prefix> [gateway]]"
            );
            return 2;
        }
    }
    let link = if network.link { "UP" } else { "DOWN" };
    let nic = &network.device.nic;
    let _ = writeln!(
        out,
        "eth0: {}, link {link}, MAC {}",
        nic.name(),
        EthernetAddress(nic.mac())
    );
    let mode = if network.dhcp.is_some() {
        "DHCP"
    } else {
        "static"
    };
    match network.address {
        Some(address) => {
            let _ = writeln!(out, "  inet {address} ({mode})");
        }
        None => {
            let _ = writeln!(out, "  no IPv4 address");
        }
    }
    if let Some(router) = network.router {
        let _ = writeln!(out, "  gateway {router}");
    }
    for dns in &network.dns {
        let _ = writeln!(out, "  DNS {dns}");
    }
    0
}

pub fn ping(args: &[&str], out: &mut String) -> i32 {
    let [destination] = args else {
        let _ = writeln!(out, "usage: ping <IPv4 address>");
        return 2;
    };
    let Ok(destination) = Ipv4Address::from_str(destination) else {
        let _ = writeln!(out, "ping: invalid IPv4 address");
        return 2;
    };
    if destination.is_unspecified() || destination.is_multicast() || destination.is_broadcast() {
        let _ = writeln!(out, "ping: expected a unicast IPv4 address");
        return 2;
    }
    let result = NETWORK
        .lock()
        .as_mut()
        .ok_or("no supported Ethernet adapter")
        .and_then(|network| network.ping(destination));
    match result {
        Ok(ms) => {
            let _ = writeln!(out, "reply from {destination}: {ms} ms");
            0
        }
        Err(error) => {
            let _ = writeln!(out, "ping: {error}");
            1
        }
    }
}
