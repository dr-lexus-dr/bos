//! The browser is shipped on the boot medium. Decode the immutable module on
//! first navigation; application windows keep separate DOMs and cookie jars.
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use bos_web_runtime::{Library, Platform, Request, Session};
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Once;
static BOOT_BLOB: AtomicUsize = AtomicUsize::new(0);
static LIB: Once<Library> = Once::new();
/// The live loaders reserve this blob outside the physical frame allocator.
pub unsafe fn install(address: usize) {
    BOOT_BLOB.store(address, Ordering::Release)
}
fn library() -> Result<&'static Library, String> {
    LIB.try_call_once(|| {
        let addr = BOOT_BLOB.load(Ordering::Acquire);
        let file;
        let blob = if addr == 0 {
            file = crate::fs::VFS
                .lock()
                .read_file("/system/browser.bweb")
                .map_err(|_| String::from("Browser executable is missing from the boot medium"))?;
            &file[..]
        } else {
            let header = unsafe { core::slice::from_raw_parts(addr as *const u8, 12) };
            let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
            if !header.starts_with(b"BWEB") || len > 128 << 20 {
                return Err("Invalid browser executable".into());
            }
            unsafe { core::slice::from_raw_parts(addr as *const u8, len + 12) }
        };
        if blob.len() < 12 || &blob[..4] != b"BWEB" {
            return Err("Invalid browser executable".into());
        }
        let size = u32::from_le_bytes(blob[8..12].try_into().unwrap()) as usize;
        if size == 0 || size > 160 << 20 {
            return Err("Invalid browser executable size".into());
        }
        if crate::mm::phys::free_memory() < ((size * 3) + 96 * 1024 * 1024) as u64 {
            return Err("More RAM is needed to start the web engine".into());
        }
        log::info!("Web engine: decoding {} bytes", size);
        let module = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&blob[12..], size)
            .map_err(|_| String::from("Corrupt browser executable"))?;
        if module.len() != size {
            return Err("Incomplete browser executable".into());
        }
        let lib = Library::new(&module).map_err(|e| e.to_string())?;
        log::info!("Web engine: executable ready");
        Ok(lib)
    })
}
pub struct System;
impl Platform for System {
    fn monotonic_ms() -> u64 {
        crate::boottime::elapsed_ms()
    }
    fn epoch_ms() -> u64 {
        crate::drivers::net::unix_seconds().unwrap_or(0) * 1000
    }
    fn random(dst: &mut [u8]) -> bool {
        getrandom::getrandom(dst).is_ok()
    }
    fn log(msg: &str) {
        log::debug!("Web engine: {msg}")
    }
}
pub type View = Session<System>;
pub fn start() -> Result<View, String> {
    let lib = library()?;
    let heap = crate::mm::heap::stats();
    let free = (crate::mm::phys::free_memory() as usize)
        .saturating_add(heap.total_size.saturating_sub(heap.used_size));
    if free < 128 << 20 {
        return Err("Insufficient memory for a browser window".into());
    }
    Session::new(lib, (free.saturating_sub(64 << 20) / 2).min(320 << 20)).map_err(|e| e.to_string())
}
pub fn transfer(r: Request) -> Result<(String, Vec<u8>), String> {
    let mut parts = r.meta.splitn(3, '\n');
    let method = parts.next().unwrap_or("");
    let url = super::http::Url::parse(parts.next().unwrap_or("")).map_err(String::from)?;
    let resp = crate::drivers::net::request(
        &url,
        method,
        parts.next().unwrap_or(""),
        &r.body,
        true,
        &mut super::network_task::checkpoint,
    )
    .map_err(String::from)?;
    if resp.clipped {
        return Err("Resource exceeds the browser transfer limit".into());
    }
    Ok((format!("{}\n{}", resp.status, resp.headers), resp.body))
}
