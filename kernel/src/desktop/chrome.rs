use super::{
    canvas::{mix, rect, width},
    icons::{icon, symbol, Symbol},
    model::*,
    ui::*,
    Desktop, Popup,
};
use alloc::{format, vec::Vec};
impl Desktop {
    pub(super) fn compose(&mut self, c: &mut super::canvas::Canvas, hidden: Option<u32>) {
        let mut hits = core::mem::take(&mut self.hits);
        hits.clear();
        let screen = rect(0, 0, c.w, c.h);
        c.begin(self.overview);
        let mut ui = Ui {
            c,
            hits: &mut hits,
            id: 0,
            p: self.p,
            bounds: screen,
        };
        if self.overview {
            self.draw_overview(&mut ui);
        } else {
            for (index, win) in self
                .windows
                .iter()
                .enumerate()
                .filter(|(_, w)| !w.minimized && Some(w.id) != hidden)
            {
                if self.windows[index + 1..].iter().any(|front| {
                    !front.minimized
                        && Some(front.id) != hidden
                        && front.transition.is_none()
                        && front.frame.x == win.frame.x
                        && front.frame.y == win.frame.y
                        && front.frame.w == win.frame.w
                        && front.frame.h == win.frame.h
                }) {
                    continue;
                }
                let r = win.frame;
                ui.id = win.id;
                ui.bounds = r;
                let p = self.p;
                let active = self
                    .windows
                    .iter()
                    .rev()
                    .find(|w| !w.minimized)
                    .is_some_and(|w| w.id == win.id);
                if p.shadows {
                    ui.c.shadow(r);
                }
                ui.c.round(r, 11, p.line());
                ui.c.round(r.inset(1), 10, p.paper());
                ui.hit(r, Cmd::Focus);
                ui.c.round(rect(r.x + 1, r.y + 1, r.w - 2, 36), 9, p.bar());
                ui.c.fill(rect(r.x + 1, r.y + 22, r.w - 2, 14), p.bar());
                ui.c.fill(rect(r.x + 1, r.y + 36, r.w - 2, 1), p.line());
                ui.hit(rect(r.x + 1, r.y + 1, r.w - 2, 35), Cmd::Move);
                if matches!(win.kind, App::Files) {
                    let left = rect(r.x + 7, r.y + 3, 26, 30);
                    ui.c.centered(left, "←", p.muted(), 1);
                    ui.hit(left, Cmd::Back);
                    let next = rect(r.x + 35, r.y + 3, 26, 30);
                    ui.c.centered(next, "→", p.muted(), 1);
                    ui.hit(next, Cmd::Forward);
                }
                let tx = if matches!(win.kind, App::Files) {
                    r.x + 76
                } else {
                    r.x + 16
                };
                ui.c.label(
                    tx,
                    r.y + 8,
                    win.kind.name(p.lang),
                    if active { p.ink() } else { p.muted() },
                    1,
                    r.w - 150,
                );
                for (i, (name, cmd)) in [
                    ("−", Cmd::Minimize),
                    ("□", Cmd::Maximize),
                    ("×", Cmd::Close),
                ]
                .iter()
                .enumerate()
                {
                    let a = rect(r.x + r.w - 99 + i as i32 * 31, r.y + 3, 30, 29);
                    ui.c.centered(a, name, p.ink(), 1);
                    ui.hit(a, *cmd);
                }
                let b = rect(r.x + 1, r.y + 37, r.w - 2, r.h - 40);
                let old = ui.c.clip(b);
                ui.bounds = b;
                super::views::content(
                    &mut ui,
                    win,
                    b,
                    &self.coast,
                    &self.cliffs,
                    &self.loads,
                    self.windows.len(),
                );
                ui.c.restore(old);
                ui.bounds = r;
                ui.hit(rect(r.x + r.w - 12, r.y + r.h - 12, 12, 12), Cmd::Resize);
            }
        }
        ui.id = 0;
        ui.bounds = screen;
        let w = screen.w;
        let h = screen.h;
        let p = self.p;
        let l = p.lang;
        ui.c.panel(
            rect(0, 0, w, 31),
            0,
            0x171923,
            if p.glass { 196 } else { 256 },
        );
        ui.c.label(16, 4, "bOS", 0xffffff, 2, 42);
        let ov = rect(68, 3, 86, 25);
        if self.overview {
            ui.c.panel(ov, 10, 0xffffff, 55);
        }
        ui.c.centered(ov, l.t("Overview", "Обзор"), 0xffffff, 1);
        ui.hit(rect(0, 0, 158, 31), Cmd::Overview);
        let dt = clock(self.time, l, p.seconds);
        let dr = rect(w / 2 - 127, 0, 254, 31);
        ui.c.centered(dr, &dt, 0xf6f5fc, 0);
        ui.hit(dr, Cmd::Calendar);
        let lr = rect(w - 177, 0, 35, 31);
        ui.c.centered(
            lr,
            if l == super::preferences::Lang::En {
                "EN"
            } else {
                "RU"
            },
            0xf6f5fc,
            0,
        );
        ui.hit(lr, Cmd::Language);
        let nr = rect(w - 140, 0, 52, 31);
        ui.c.round(rect(nr.x + 12, 8, 19, 13), 2, 0xe8e8fa);
        ui.c.fill(rect(nr.x + 14, 10, 15, 8), 0x343643);
        ui.c.fill(
            rect(nr.x + 18, 22, 8, 2),
            if self.online { 0x9cddae } else { 0xabaaba },
        );
        ui.hit(nr, Cmd::Network);
        let sound = rect(w - 88, 0, 36, 31);
        symbol(
            ui.c,
            rect(sound.x + 8, 7, 18, 18),
            Symbol::Speaker,
            if crate::arch::x86_64::sound_enabled() {
                0xffffff
            } else {
                0x88889a
            },
        );
        ui.hit(sound, Cmd::Sound);
        let pr = rect(w - 48, 0, 42, 31);
        symbol(ui.c, rect(pr.x + 10, 7, 18, 18), Symbol::Power, 0xffffff);
        ui.hit(pr, Cmd::Power);
        let size = if w >= 1000 { 47 } else { 36 };
        let gap = 16;
        let dw = 7 * (size + gap) + 9;
        let dock = rect((w - dw) / 2, h - size - 28, dw, size + 16);
        ui.c.panel(dock, 16, 0xa4a6b9, 110);
        ui.c.panel(dock.inset(1), 15, 0x262735, if p.glass { 222 } else { 256 });
        for i in 0..7 {
            let a = rect(dock.x + 11 + i * (size + gap), dock.y + 5, size, size);
            let kind = if i == 0 {
                None
            } else {
                Some(DOCK[i as usize - 1])
            };
            icon(ui.c, a, kind);
            ui.hit(
                rect(a.x - 4, dock.y, a.w + 8, dock.h),
                kind.map_or(Cmd::Overview, Cmd::Launch),
            );
            if kind.is_some_and(|k| self.windows.iter().any(|w| w.kind == k)) {
                ui.c.round(rect(a.x + size / 2 - 2, a.y + size + 4, 4, 4), 2, 0xe1e0fb);
            }
        }
        if let Some(pop) = &self.popup {
            self.draw_popup(&mut ui, pop);
        }
        if let Some(d) = &self.dialog {
            let r = rect(
                (w - 360.min(w - 32)) / 2,
                (h - 156) / 2,
                360.min(w - 32),
                156,
            );
            ui.c.panel(screen, 0, 0x12121d, 68);
            ui.hit(screen, Cmd::Focus);
            ui.c.round(r, 12, p.line());
            ui.c.round(r.inset(1), 11, p.paper());
            ui.text(r.x + 17, r.y + 12, d.title(l), 2);
            if d.editable() {
                ui.input(
                    rect(r.x + 16, r.y + 52, r.w - 32, 32),
                    &d.field.value,
                    true,
                    Cmd::Input,
                );
            } else {
                ui.dim(r.x + 16, r.y + 58, &d.field.value, r.w - 32);
            }
            ui.button(
                rect(r.x + r.w - 185, r.y + 109, 81, 32),
                l.t("Cancel", "Отмена"),
                Cmd::Cancel,
                false,
            );
            ui.button(
                rect(r.x + r.w - 95, r.y + 109, 79, 32),
                l.t("OK", "ОК"),
                Cmd::DialogOk,
                true,
            );
        }
        if let Some((msg, until)) = &self.toast {
            if crate::boottime::elapsed_ms() < *until {
                let tw = (width(msg, 0) + 28).min(w - 32);
                let left = until.saturating_sub(crate::boottime::elapsed_ms());
                let t = super::motion::progress(
                    2200u64.saturating_sub(left).min(left),
                    0,
                    p.motion_ms(),
                );
                let r = rect(w - tw - 14 + (256 - t) * (tw + 16) / 256, 42, tw, 36);
                ui.c.panel(r, 9, 0x262737, 240);
                ui.c.centered(r, msg, 0xffffff, 0);
            }
        }
        self.hits = hits;
    }
    fn draw_overview(&self, ui: &mut Ui<'_>) {
        let l = self.p.lang;
        let w = ui.c.w;
        let h = ui.c.h;
        let t = super::motion::progress(
            crate::boottime::elapsed_ms(),
            self.overview_stamp,
            self.p.motion_ms(),
        );
        let search = rect(
            w / 2 - (w / 3).min(240),
            h / 9 + (256 - t) * 32 / 256,
            (w * 2 / 3).min(480),
            44,
        );
        ui.c.panel(search, 19, 0xbcbacd, 175);
        ui.c.panel(search.inset(1), 18, 0x242431, 230);
        let text = if self.search.value.is_empty() {
            l.t("Search applications", "Поиск приложений")
        } else {
            &self.search.value
        };
        ui.c.label(
            search.x + 19,
            search.y + 11,
            text,
            0xf3f2ff,
            1,
            search.w - 32,
        );
        ui.hit(search, Cmd::Input);
        let tab = rect(w / 2 - 123, search.y + 63, 246, 32);
        ui.c.panel(tab, 13, 0x242431, 205);
        for (i, (name, apps)) in [
            (l.t("Windows", "Окна"), false),
            (l.t("Applications", "Приложения"), true),
        ]
        .iter()
        .enumerate()
        {
            let r = rect(tab.x + i as i32 * 123, tab.y, 123, 32);
            if self.app_grid == *apps {
                ui.c.round(r, 12, 0xb5adf3);
            }
            ui.c.centered(
                r,
                name,
                if self.app_grid == *apps {
                    0x252338
                } else {
                    0xffffff
                },
                1,
            );
            ui.hit(r, Cmd::OverviewTab(*apps));
        }
        let apps = self.filtered();
        let rowtop = tab.y + tab.h + ((h / 13).clamp(22, 68));
        let sw = (w * 78 / 100).min(1100);
        let cw = sw / 5;
        let rh = ((h - rowtop - 87) / 2).clamp(60, 170);
        let isize = (rh - 36).clamp(37, 96).min(cw - 25);
        if self.app_grid {
            for (i, app) in apps.iter().enumerate() {
                let cx = (w - sw) / 2 + (i as i32 % 5) * cw;
                let yy = rowtop + (i as i32 / 5) * rh;
                let r = rect(cx, yy, cw, rh - 2);
                if self.selected == Some(i) {
                    ui.c.panel(r.inset(2), 10, 0xf1effa, 28);
                }
                icon(
                    ui.c,
                    rect(cx + (cw - isize) / 2, yy + 2, isize, isize),
                    Some(*app),
                );
                ui.c.centered(rect(cx, yy + isize + 7, cw, 28), app.name(l), 0xffffff, 1);
                ui.hit(r, Cmd::Launch(*app));
            }
        } else {
            let list: Vec<_> = self
                .windows
                .iter()
                .filter(|v| {
                    v.kind
                        .name(l)
                        .to_lowercase()
                        .contains(&self.search.value.to_lowercase())
                })
                .collect();
            for (i, win) in list.iter().enumerate() {
                let cw = (sw / 3).max(1);
                let r = rect(
                    (w - sw) / 2 + i as i32 % 3 * cw,
                    rowtop + i as i32 / 3 * 103,
                    cw - 12,
                    89,
                );
                ui.c.panel(r, 10, 0x252634, 210);
                icon(ui.c, rect(r.x + 13, r.y + 17, 50, 50), Some(win.kind));
                ui.c.label(r.x + 75, r.y + 31, win.kind.name(l), 0xffffff, 1, r.w - 80);
                ui.hit(r, Cmd::Activate(win.id));
            }
        }
    }
    fn draw_popup(&self, ui: &mut Ui<'_>, pop: &Popup) {
        let p = self.p;
        let l = p.lang;
        let w = ui.c.w;
        let h = ui.c.h;
        match pop {
            Popup::Calendar => {
                let r = rect(w / 2 - 126, 35, 252, 260);
                ui.c.round(r, 10, p.paper());
                let (cy, cm, today) = date(self.time / 86400);
                let months =
                    (cy * 12 + cm as i32 + self.calendar_offset).clamp(1970 * 12, 9999 * 12 + 11);
                let (y, m) = (months / 12, (months % 12) as usize);
                let d = if y == cy && m == cm { today } else { 0 };
                ui.c.centered(
                    rect(r.x, r.y + 5, r.w, 33),
                    &format!("{} {y}", month(m, l)),
                    p.ink(),
                    2,
                );
                let first = weekday_first(y, m);
                for (i, day) in if l == super::preferences::Lang::En {
                    ["M", "T", "W", "T", "F", "S", "S"]
                } else {
                    ["П", "В", "С", "Ч", "П", "С", "В"]
                }
                .iter()
                .enumerate()
                {
                    ui.c.centered(
                        rect(r.x + 9 + i as i32 * 34, r.y + 43, 33, 22),
                        day,
                        p.muted(),
                        0,
                    );
                }
                for n in 1..=month_days(y, m) {
                    let pos = first + n as i32 - 1;
                    let a = rect(r.x + 9 + pos % 7 * 34, r.y + 69 + pos / 7 * 25, 32, 24);
                    if n == d {
                        ui.c.round(a, 9, p.accent());
                    }
                    ui.c.centered(
                        a,
                        &format!("{n}"),
                        if n == d { 0xffffff } else { p.ink() },
                        0,
                    );
                }
                ui.hit(r, Cmd::Focus);
                ui.button(rect(r.x + 6, r.y + 5, 28, 30), "←", Cmd::Month(-1), false);
                ui.button(
                    rect(r.x + r.w - 34, r.y + 5, 28, 30),
                    "→",
                    Cmd::Month(1),
                    false,
                );
                ui.button(
                    rect(r.x + 70, r.y + r.h - 35, r.w - 140, 29),
                    l.t("Today", "Сегодня"),
                    Cmd::Today,
                    false,
                );
            }
            Popup::Network => {
                let r = rect(w - 295, 34, 282, 197);
                ui.c.round(r, 10, p.paper());
                ui.text(
                    r.x + 15,
                    r.y + 12,
                    l.t("Ethernet network", "Сеть Ethernet"),
                    2,
                );
                let mut info = alloc::string::String::new();
                crate::drivers::net::ifconfig(&[], &mut info);
                for (i, line) in info.lines().take(7).enumerate() {
                    ui.dim(r.x + 15, r.y + 54 + i as i32 * 19, line, r.w - 24);
                }
                ui.hit(r, Cmd::Network);
            }
            Popup::Menu { id, x, y, items } => {
                let ww = 189;
                let hh = items.len() as i32 * 30 + 10;
                let r = rect(
                    (*x).clamp(2, w - ww - 2),
                    (*y).clamp(32, (h - hh - 4).max(32)),
                    ww,
                    hh,
                );
                ui.id = *id;
                ui.c.round(r, 8, p.line());
                ui.c.round(r.inset(1), 7, p.paper());
                for (i, (en, ru, cmd)) in items.iter().enumerate() {
                    let a = rect(r.x + 3, r.y + 5 + i as i32 * 30, r.w - 6, 29);
                    ui.c.label(a.x + 11, a.y + 6, l.t(en, ru), p.ink(), 0, a.w - 15);
                    ui.hit(a, *cmd);
                }
            }
        }
    }
}
