//! rustls' no_std client. The RTC supplies UTC for WebPKI validity checks.
use super::client::Stream;
use crate::web::http::{Receiver, Response, Result};
use alloc::{sync::Arc, vec, vec::Vec};
use core::time::Duration;
use rustls::{
    client::UnbufferedClientConnection,
    crypto::ring::default_provider,
    pki_types::{ServerName, UnixTime},
    time_provider::TimeProvider,
    unbuffered::ConnectionState,
    ClientConfig, RootCertStore,
};
use x86_64::instructions::port::Port;

#[derive(Debug)]
struct Rtc;

fn cmos(index: u8) -> u8 {
    unsafe {
        Port::<u8>::new(0x70).write(index);
        Port::<u8>::new(0x71).read()
    }
}

impl TimeProvider for Rtc {
    fn current_time(&self) -> Option<UnixTime> {
        let sample = || {
            [
                cmos(0),
                cmos(2),
                cmos(4),
                cmos(7),
                cmos(8),
                cmos(9),
                cmos(0x0b),
            ]
        };
        for _ in 0..10000 {
            if cmos(0x0a) & 0x80 != 0 {
                continue;
            }
            let time = sample();
            if sample() != time || cmos(0x0a) & 0x80 != 0 {
                continue;
            }
            let bin = |x: u8| {
                if time[6] & 4 != 0 {
                    x
                } else {
                    (x >> 4) * 10 + (x & 15)
                }
            };
            let sec = bin(time[0]);
            let minute = bin(time[1]);
            let mut hour = bin(time[2] & 0x7f);
            if time[6] & 2 == 0 {
                hour = hour % 12 + 12 * (time[2] >> 7);
            }
            let day = bin(time[3]);
            let month = bin(time[4]);
            let year = 2000 + u32::from(bin(time[5]));
            let leap = |y| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
            let months = [
                31,
                if leap(year) { 29 } else { 28 },
                31,
                30,
                31,
                30,
                31,
                31,
                30,
                31,
                30,
                31,
            ];
            if !(1..=12).contains(&month)
                || day == 0
                || day > months[month as usize - 1]
                || hour > 23
                || minute > 59
                || sec > 59
            {
                return None;
            }
            let mut days = (1970..year)
                .map(|y| if leap(y) { 366 } else { 365 })
                .sum::<u64>();
            days += months[..month as usize - 1]
                .iter()
                .map(|&n| u64::from(n))
                .sum::<u64>()
                + u64::from(day - 1);
            return Some(UnixTime::since_unix_epoch(Duration::from_secs(
                days * 86400 + u64::from(hour) * 3600 + u64::from(minute) * 60 + u64::from(sec),
            )));
        }
        None
    }
}

pub(super) fn unix_seconds() -> Option<u64> {
    Rtc.current_time().map(|t| t.as_secs())
}

pub(super) fn request(
    stream: &mut Stream<'_>,
    host: &str,
    request: &[u8],
    mut parser: Receiver,
) -> Result<Response> {
    static CONFIG: spin::Once<Arc<ClientConfig>> = spin::Once::new();
    let config = CONFIG.try_call_once(|| {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        Ok(Arc::new(
            ClientConfig::builder_with_details(Arc::new(default_provider()), Arc::new(Rtc))
                .with_safe_default_protocol_versions()
                .map_err(|_| "TLS configuration failed")?
                .with_root_certificates(roots)
                .with_no_client_auth(),
        ))
    })?;
    let name = ServerName::try_from(host)
        .map_err(|_| "Invalid TLS server name")?
        .to_owned();
    let mut tls = UnbufferedClientConnection::new(config.clone(), name)
        .map_err(|_| "TLS initialization failed (CPU entropy required)")?;
    let mut input = Vec::new();
    let mut output = vec![0; 18432];
    let mut chunk = [0; 8192];
    let mut pending = 0;
    let mut sent = false;
    loop {
        let status = tls.process_tls_records(&mut input);
        let mut discard = status.discard;
        let mut need_input = false;
        match status.state.map_err(|error| {
            log::info!("TLS: {error}");
            match error {
                rustls::Error::InvalidCertificate(_) => "TLS certificate verification failed",
                rustls::Error::FailedToGetCurrentTime => "TLS requires a valid UTC hardware clock",
                _ => "TLS handshake or record failed",
            }
        })? {
            ConnectionState::EncodeTlsData(mut record) => {
                pending += record
                    .encode(&mut output[pending..])
                    .map_err(|_| "TLS flight too large")?;
            }
            ConnectionState::TransmitTlsData(record) => {
                stream.write(&output[..pending])?;
                pending = 0;
                record.done();
            }
            ConnectionState::BlockedHandshake => need_input = true,
            ConnectionState::WriteTraffic(mut record) if !sent => {
                for part in request.chunks(16384) {
                    let n = record
                        .encrypt(part, &mut output)
                        .map_err(|_| "TLS write failed")?;
                    stream.write(&output[..n])?;
                }
                sent = true;
            }
            ConnectionState::WriteTraffic(_) => need_input = true,
            ConnectionState::ReadTraffic(mut data) => {
                while let Some(record) = data.next_record() {
                    let record = record.map_err(|_| "Invalid TLS record")?;
                    discard += record.discard;
                    parser.push(record.payload)?;
                }
                if parser.done() {
                    return parser.finish(true);
                }
            }
            ConnectionState::PeerClosed | ConnectionState::Closed => return parser.finish(true),
            _ => return Err("Unsupported TLS state"),
        }
        input.drain(..discard);
        if need_input {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return parser.finish(false);
            }
            if input.len() + n > 65536 {
                return Err("TLS handshake exceeds memory budget");
            }
            input.extend_from_slice(&chunk[..n]);
        }
    }
}
