//! bnet — bOS Network Tools
//! Internet connectivity, HTTP client, DNS resolution, and network diagnostics

use std::env;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};
use anyhow::{Context, Result};
use reqwest::blocking::Client;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    let cmd = &args[1];
    match cmd.as_str() {
        "ping" => {
            let default_host = "8.8.8.8".to_string();
            let host = args.get(2).unwrap_or(&default_host);
            let count = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4);
            ping_host(host, count)?;
        }
        "dns" | "resolve" => {
            if args.len() < 3 {
                eprintln!("Usage: bnet dns <hostname>");
                return Ok(());
            }
            resolve_dns(&args[2])?;
        }
        "http" | "get" => {
            if args.len() < 3 {
                eprintln!("Usage: bnet http <url>");
                return Ok(());
            }
            http_get(&args[2])?;
        }
        "download" | "dl" => {
            if args.len() < 3 {
                eprintln!("Usage: bnet download <url> [output-file]");
                return Ok(());
            }
            let output = args.get(3).cloned();
            download_file(&args[2], output)?;
        }
        "wget" => {
            if args.len() < 3 {
                eprintln!("Usage: bnet wget <url> [-O output-file]");
                return Ok(());
            }
            let output = match args.get(3).map(String::as_str) {
                None => None,
                Some("-O") => Some(args.get(4).context("-O requires an output filename")?.clone()),
                Some(path) => Some(path.to_owned()),
            };
            download_file(&args[2], output)?;
        }
        "git" => git_command(&args[2..])?,
        "check" | "connectivity" => check_connectivity()?,
        "ifconfig" | "interfaces" => show_interfaces()?,
        "port" | "scan" => {
            if args.len() < 4 {
                eprintln!("Usage: bnet port <host> <port>");
                return Ok(());
            }
            scan_port(&args[2], &args[3])?;
        }
        "traceroute" | "trace" => {
            if args.len() < 3 {
                eprintln!("Usage: bnet traceroute <host>");
                return Ok(());
            }
            traceroute(&args[2])?;
        }
        "--help" | "-h" | "help" => print_usage(),
        _ => {
            eprintln!("Unknown command: {}", cmd);
            print_usage();
        }
    }
    Ok(())
}

fn print_usage() {
    println!("bOS Network Tools (bnet) v0.1.0");
    println!("Usage: bnet <command> [args]");
    println!();
    println!("Commands:");
    println!("  check                    Check internet connectivity");
    println!("  ping <host> [count]      Ping a host (default: 8.8.8.8, count: 4)");
    println!("  dns <hostname>           Resolve DNS for hostname");
    println!("  http <url>               Make HTTP GET request");
    println!("  download <url> [file]    Download file from URL");
    println!("  wget <url> [-O file]     Download a file (wget-compatible syntax)");
    println!("  git [args...]            Run Git on the Linux host");
    println!("  interfaces               Show network interfaces");
    println!("  port <host> <port>       Check if TCP port is open");
    println!("  traceroute <host>        Trace route to host");
    println!("  help                     Show this help");
}

/// Git requires Git to be installed on the Linux host. Forward arguments and
/// exit status unchanged so normal commands such as clone, pull and status work.
fn git_command(args: &[String]) -> Result<()> {
    let status = std::process::Command::new("git")
        .args(args)
        .status()
        .context("could not start git; install Git on the Linux host")?;
    if !status.success() {
        anyhow::bail!("git exited with {status}");
    }
    Ok(())
}

fn check_connectivity() -> Result<()> {
    println!("bOS Network Connectivity Check");
    println!("================================");

    // Test 1: Ping Google DNS
    print!("1. Pinging 8.8.8.8... ");
    match ping_once("8.8.8.8", Duration::from_secs(3)) {
        Ok(duration) => println!("✓ {:.2} ms", duration.as_secs_f64() * 1000.0),
        Err(e) => println!("✗ Failed: {}", e),
    }

    // Test 2: Ping Cloudflare DNS
    print!("2. Pinging 1.1.1.1... ");
    match ping_once("1.1.1.1", Duration::from_secs(3)) {
        Ok(duration) => println!("✓ {:.2} ms", duration.as_secs_f64() * 1000.0),
        Err(e) => println!("✗ Failed: {}", e),
    }

    // Test 3: DNS resolution
    print!("3. Resolving google.com... ");
    match resolve_dns("google.com") {
        Ok(ips) => {
            println!("✓ {}", ips.iter().map(|ip| ip.to_string()).collect::<Vec<_>>().join(", "));
        }
        Err(e) => println!("✗ Failed: {}", e),
    }

    // Test 4: HTTP request
    print!("4. HTTP GET http://httpbin.org/get... ");
    match http_get_simple("http://httpbin.org/get") {
        Ok(status) => println!("✓ HTTP {}", status),
        Err(e) => println!("✗ Failed: {}", e),
    }

    // Test 5: HTTPS request
    print!("5. HTTPS GET https://httpbin.org/get... ");
    match http_get_simple("https://httpbin.org/get") {
        Ok(status) => println!("✓ HTTP {}", status),
        Err(e) => println!("✗ Failed: {}", e),
    }

    println!("\nConnectivity check complete.");
    Ok(())
}

fn ping_host(host: &str, count: usize) -> Result<()> {
    println!("PING {} ({}) {} bytes of data", host, host, 56);

    let mut success = 0;
    let mut total_time = Duration::ZERO;

    for i in 1..=count {
        match ping_once(host, Duration::from_secs(5)) {
            Ok(duration) => {
                println!("{} bytes from {}: icmp_seq={} time={:.2} ms",
                    56, host, i, duration.as_secs_f64() * 1000.0);
                success += 1;
                total_time += duration;
            }
            Err(e) => {
                println!("Request timeout for icmp_seq={}: {}", i, e);
            }
        }
        if i < count {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    let loss = ((count - success) as f64 / count as f64) * 100.0;
    println!("\n--- {} ping statistics ---", host);
    println!("{} packets transmitted, {} received, {:.1}% packet loss", count, success, loss);
    if success > 0 {
        println!("rtt min/avg/max = {:.2}/{:.2}/{:.2} ms",
            total_time.as_secs_f64() * 1000.0 / success as f64,
            total_time.as_secs_f64() * 1000.0 / success as f64,
            total_time.as_secs_f64() * 1000.0 / success as f64);
    }
    Ok(())
}

fn ping_once(host: &str, timeout: Duration) -> Result<Duration> {
    // Use system ping command since raw sockets require root
    let start = Instant::now();
    let output = std::process::Command::new("ping")
        .args(["-c", "1", "-W", &timeout.as_secs().to_string(), host])
        .output()?;

    let elapsed = start.elapsed();
    if output.status.success() {
        Ok(elapsed)
    } else {
        Err(anyhow::anyhow!("ping failed"))
    }
}

fn resolve_dns(hostname: &str) -> Result<Vec<IpAddr>> {
    let addrs: Vec<SocketAddr> = format!("{}:80", hostname).to_socket_addrs()?.collect();
    let ips: Vec<IpAddr> = addrs.into_iter().map(|a| a.ip()).collect();

    if ips.is_empty() {
        return Err(anyhow::anyhow!("No addresses found for {}", hostname));
    }

    println!("DNS resolution for {}:", hostname);
    for ip in &ips {
        println!("  {}", ip);
    }
    Ok(ips)
}

fn http_get(url: &str) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("bOS-Network-Tools/0.1")
        .build()?;

    println!("GET {}", url);
    let start = Instant::now();
    let response = client.get(url).send()?;
    let elapsed = start.elapsed();

    println!("Status: {}", response.status());
    println!("Time: {:.2} ms", elapsed.as_secs_f64() * 1000.0);
    println!("Headers:");
    for (name, value) in response.headers() {
        println!("  {}: {}", name, value.to_str().unwrap_or("<invalid>"));
    }

    let body = response.text()?;
    println!("\nBody ({} bytes):", body.len());
    if body.len() > 2000 {
        println!("{}", &body[..2000]);
        println!("\n... (truncated, {} bytes total)", body.len());
    } else {
        println!("{}", body);
    }
    Ok(())
}

fn http_get_simple(url: &str) -> Result<u16> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("bOS-Network-Tools/0.1")
        .build()?;

    let response = client.get(url).send()?;
    Ok(response.status().as_u16())
}

fn download_file(url: &str, output: Option<String>) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("bOS-Network-Tools/0.1")
        .build()?;

    println!("Downloading {}", url);
    let start = Instant::now();
    let mut response = client.get(url).send()?;

    let filename = output.unwrap_or_else(|| {
        url::Url::parse(url)
            .ok()
            .and_then(|u| u.path_segments().and_then(|s| s.last().map(|s| s.to_string())))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "download".to_string())
    });

    let total_size = response.content_length().unwrap_or(0);
    let mut downloaded = 0u64;
    let mut file = std::fs::File::create(&filename)?;

    let mut buffer = vec![0u8; 8192];
    loop {
        let n = response.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buffer[..n])?;
        downloaded += n as u64;

        if total_size > 0 {
            let percent = (downloaded as f64 / total_size as f64) * 100.0;
            let speed = downloaded as f64 / start.elapsed().as_secs_f64() / 1024.0;
            print!("\rProgress: {:.1}% ({:.1} KB/s)", percent, speed);
            std::io::stdout().flush()?;
        }
    }

    let elapsed = start.elapsed();
    let speed = downloaded as f64 / elapsed.as_secs_f64() / 1024.0;
    println!("\nDownloaded {} bytes in {:.2}s ({:.1} KB/s)", downloaded, elapsed.as_secs_f64(), speed);
    println!("Saved to: {}", filename);
    Ok(())
}

fn show_interfaces() -> Result<()> {
    println!("Network Interfaces");
    println!("==================");

    // Try to use system commands
    let output = std::process::Command::new("ip")
        .args(["addr", "show"])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            println!("{}", String::from_utf8_lossy(&out.stdout));
            return Ok(());
        }
    }

    // Fallback to ifconfig
    let output = std::process::Command::new("ifconfig").output();
    if let Ok(out) = output {
        if out.status.success() {
            println!("{}", String::from_utf8_lossy(&out.stdout));
            return Ok(());
        }
    }

    // Fallback: try to read from /proc/net/dev
    if let Ok(content) = std::fs::read_to_string("/proc/net/dev") {
        println!("{}", content);
        return Ok(());
    }

    println!("Could not retrieve network interfaces");
    Ok(())
}

fn scan_port(host: &str, port: &str) -> Result<()> {
    let port: u16 = port.parse().context("Invalid port number")?;

    let addrs: Vec<SocketAddr> = format!("{}:{}", host, port).to_socket_addrs()?.collect();
    if addrs.is_empty() {
        return Err(anyhow::anyhow!("Could not resolve host: {}", host));
    }

    println!("Scanning {}:{}...", host, port);
    let start = Instant::now();

    for addr in addrs {
        match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(_) => {
                let elapsed = start.elapsed();
                println!("Port {} is OPEN ({:.2} ms)", port, elapsed.as_secs_f64() * 1000.0);
                return Ok(());
            }
            Err(e) => {
                let elapsed = start.elapsed();
                println!("Port {} is CLOSED/FILTERED ({:.2} ms): {}", port, elapsed.as_secs_f64() * 1000.0, e);
            }
        }
    }
    Ok(())
}

fn traceroute(host: &str) -> Result<()> {
    println!("Traceroute to {} (not fully implemented, using system traceroute)", host);

    let output = std::process::Command::new("traceroute")
        .args(["-n", host])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            println!("{}", String::from_utf8_lossy(&out.stdout));
            return Ok(());
        }
    }

    // Try tracepath
    let output = std::process::Command::new("tracepath")
        .args(["-n", host])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            println!("{}", String::from_utf8_lossy(&out.stdout));
            return Ok(());
        }
    }

    println!("traceroute/tracepath not available");
    Ok(())
}
