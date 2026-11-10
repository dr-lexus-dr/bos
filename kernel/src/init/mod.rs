//! Kernel main: hand off to the built-in bSH console.
//! (Spawning /sbin/init from userspace comes back once proc/fs land.)
use log::info;

pub fn kernel_main(boot_info: &'static bloader_shared::BootInfo) -> ! {
    info!("bOS {} booted. cpu_count={} initrd={:?}",
        boot_info.bootloader_version_str(), boot_info.cpu_count, boot_info.initrd_addr);

    crate::bsh::run(boot_info)
}
