use bloader_shared::{FramebufferInfo, PixelFormat};
use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as UefiPixelFormat};

pub fn setup_framebuffer(boot_services: &BootServices, compact: bool) -> Option<FramebufferInfo> {
    let gop_handle = boot_services
        .get_handle_for_protocol::<GraphicsOutput>()
        .ok()?;
    // Do not claim GOP exclusively: firmware console drivers may already own
    // it. Exclusive access can disconnect the console used by our logger.
    let mut gop = unsafe {
        boot_services.open_protocol::<GraphicsOutput>(
            uefi::table::boot::OpenProtocolParams {
                handle: gop_handle,
                agent: boot_services.image_handle(),
                controller: None,
            },
            uefi::table::boot::OpenProtocolAttributes::GetProtocol,
        )
    }
    .ok()?;

    let current = gop.current_mode_info();
    let direct = |format| matches!(format, UefiPixelFormat::Rgb | UefiPixelFormat::Bgr);
    if compact && (current.resolution() != (1280, 800) || !direct(current.pixel_format())) {
        let best = gop
            .modes()
            .filter(|mode| {
                let (w, h) = mode.info().resolution();
                w >= 640 && h >= 480 && direct(mode.info().pixel_format())
            })
            .min_by_key(|mode| {
                let (w, h) = mode.info().resolution();
                ((w, h) != (1280, 800), (w, h) != (1024, 768), (w, h) != (800, 600), w.saturating_mul(h))
            });
        if let Some(mode) = best {
            if mode.info().resolution() != current.resolution() || !direct(current.pixel_format()) {
                let _ = gop.set_mode(&mode);
            }
        }
    }
    let mode = gop.current_mode_info();
    let (w, h) = mode.resolution();
    let pixel_format = match mode.pixel_format() {
        UefiPixelFormat::Rgb => PixelFormat::RGB,
        UefiPixelFormat::Bgr => PixelFormat::BGR,
        UefiPixelFormat::Bitmask => PixelFormat::Bitmask,
        UefiPixelFormat::BltOnly => return None,
    };
    let base = gop.frame_buffer().as_mut_ptr() as u64;

    Some(FramebufferInfo {
        base_addr: base,
        width: w as u32,
        height: h as u32,
        stride: mode.stride() as u32,
        pixel_format,
        bits_per_pixel: 32,
    })
}
