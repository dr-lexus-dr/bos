//! Window movements reuse a composited backdrop and a single raster of the
//! foreground window. Only the old and new bounds touch video memory.
use super::{
    canvas::{rect, Canvas, Rect},
    model::{Transition, DOCK},
    ui::Cmd,
    Desktop,
};
use alloc::vec::Vec;
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Drag(Cmd),
    Effect(Transition),
}
pub(super) struct Surface {
    id: u32,
    kind: Kind,
    source: Rect,
    last: Rect,
    back: Vec<u32>,
    pixels: Vec<u32>,
}
pub(super) fn progress(now: u64, started: u64, duration: u64) -> i32 {
    if duration == 0 {
        return 256;
    }
    let t = (now.saturating_sub(started).min(duration) * 256 / duration) as i32;
    t * t * (768 - 2 * t) / 65536
}

#[derive(Clone, Copy, Default)]
pub struct Slide {
    start: u64,
    from: i32,
    to: i32,
}
impl Slide {
    pub fn value(self, now: u64, duration: u64) -> i32 {
        self.from + (self.to - self.from) * progress(now, self.start, duration) / 256
    }
    pub fn set(&mut self, target: i32, now: u64, duration: u64) {
        if self.to != target {
            self.from = self.value(now, duration);
            self.to = target;
            self.start = now;
        }
    }
    pub fn active(self, now: u64, duration: u64) -> bool {
        self.from != self.to && now.saturating_sub(self.start) <= duration + 16
    }
}
impl Desktop {
    pub(super) fn paint(&mut self, c: &mut Canvas) {
        let front = self.windows.iter().rposition(|w| !w.minimized);
        let motion = front.and_then(|i| {
            let w = &self.windows[i];
            if self.overview || self.popup.is_some() || self.dialog.is_some() {
                return None;
            }
            let k = self
                .drag
                .filter(|d| d.0 == w.id)
                .map(|d| Kind::Drag(d.1))
                .or_else(|| w.transition.filter(|_| self.p.animations).map(Kind::Effect))?;
            Some((i, w.id, k, w.frame, w.kind))
        });
        let Some((i, id, k, r, app)) = motion else {
            self.surface = None;
            self.compose(c, None);
            c.present();
            return;
        };
        let fresh = self
            .surface
            .as_ref()
            .is_none_or(|s| s.kind != k || s.id != id);
        if fresh {
            self.surface = None;
            self.compose(c, Some(id));
            let back = c.backdrop();
            self.compose(c, None);
            let Some(pixels) = c.snapshot(r) else {
                c.present();
                return;
            };
            self.surface = Some(Surface {
                id,
                kind: k,
                source: r,
                last: r,
                back,
                pixels,
            });
            if let Kind::Effect(_) = k {
                self.windows[i].opened = crate::boottime::elapsed_ms();
            }
        }
        let s = self.surface.as_mut().unwrap();
        let now = crate::boottime::elapsed_ms();
        let mut alpha = 256;
        let target = match k {
            Kind::Drag(_) => r,
            Kind::Effect(effect) => {
                let duration = [340, 245, 160][self.p.speed as usize];
                let smooth = progress(now, self.windows[i].opened, duration);
                let q = if matches!(effect, Transition::Minimize | Transition::Close) {
                    256 - smooth
                } else {
                    smooth
                };
                let dock = matches!(effect, Transition::Minimize | Transition::Restore);
                let little = if dock {
                    let slot = DOCK
                        .iter()
                        .position(|a| *a == app)
                        .map_or(0, |n| n as i32 + 1);
                    let step = if c.w >= 1000 { 63 } else { 52 };
                    rect(c.w / 2 + (slot - 3) * step - 12, c.h - 44, 24, 18)
                } else {
                    let dx = r.w / 14;
                    let dy = r.h / 14;
                    rect(r.x + dx, r.y + dy, r.w - 2 * dx, r.h - 2 * dy)
                };
                let mix = |x: i32, y: i32| x + (y - x) * q / 256;
                alpha = if dock {
                    (154 + 102 * q / 256) as u32
                } else {
                    (64 + 192 * q / 256) as u32
                };
                rect(
                    mix(little.x, r.x),
                    mix(little.y, r.y),
                    mix(little.w, r.w),
                    mix(little.h, r.h),
                )
            }
        };
        let damage = |r: Rect| r.inset(-11);
        if fresh {
            c.region_from(&s.back, damage(s.source));
        }
        c.region_from(&s.back, damage(s.last));
        if self.p.shadows {
            c.shadow(target);
        }
        c.surface(&s.pixels, s.source, target, alpha);
        let step = if c.w >= 1000 { 63 } else { 52 };
        let dw = 7 * step + 9;
        let dock = rect((c.w - dw) / 2, c.h - (step - 16) - 28, dw, step);
        let covered = dock.intersection(target);
        if covered.w > 0 && covered.h > 0 {
            c.region_from(&s.back, covered);
        }
        if fresh {
            c.present()
        } else {
            c.present_area(damage(s.last));
            c.present_area(damage(target));
        }
        s.last = target;
    }
}
#[derive(Default)]
pub(super) struct Timings {
    motion: u32,
    inputs: u32,
    render: [u16; 32],
    latency: [u16; 32],
}
impl Timings {
    pub fn record(&mut self, time: u64, latency: Option<u64>, moving: bool) {
        if let Some(ms) = latency {
            self.latency[self.inputs as usize % 32] = ms.min(u16::MAX as u64) as u16;
            self.inputs += 1;
        }
        if !moving {
            return;
        }
        self.render[self.motion as usize % 32] = time.min(u16::MAX as u64) as u16;
        self.motion += 1;
        if self.motion % 8 == 0 {
            let mut render = self.render;
            let mut input = self.latency;
            let r = &mut render[..self.motion.min(32) as usize];
            r.sort_unstable();
            let i = &mut input[..self.inputs.min(32) as usize];
            i.sort_unstable();
            let p95 = |v: &[u16]| {
                v.get((v.len() * 95).div_ceil(100).saturating_sub(1))
                    .copied()
                    .unwrap_or(0)
            };
            log::debug!(
                "Compositor: motion={} render_p50_ms={} render_p95_ms={} inputs={} input_p95_ms={}",
                self.motion,
                r[r.len() / 2],
                p95(r),
                self.inputs,
                p95(i)
            );
        }
    }
}
