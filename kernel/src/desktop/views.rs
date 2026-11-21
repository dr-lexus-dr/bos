use super::{
    canvas::{mix, rect, width, Picture, Rect},
    icons::{icon, symbol, Symbol},
    model::*,
    preferences::Lang,
    ui::*,
};
use alloc::{format, string::String, vec::Vec};
pub fn content(
    ui: &mut Ui<'_>,
    win: &Window,
    b: Rect,
    coast: &Picture,
    cliffs: &Picture,
    loads: &[u8],
    windows: usize,
) {
    let p = ui.p;
    let l = p.lang;
    let (x, y, w, h) = (b.x, b.y, b.w, b.h);
    match &win.model {
        Model::Image(path, image) => {
            let scale = (w * 1024 / image.w as i32).min((h - 35) * 1024 / image.h as i32);
            let iw = image.w as i32 * scale / 1024;
            let ih = image.h as i32 * scale / 1024;
            ui.c.picture(rect(x + (w - iw) / 2, y + (h - 35 - ih) / 2, iw, ih), image);
            ui.dim(x + 12, y + h - 25, path, w - 24);
        }
        Model::Files(f) => {
            let side = sidebar(w);
            ui.c.fill(rect(x, y, side, h), p.bar());
            ui.c.fill(rect(x + side - 1, y, 1, h), p.line());
            for (i, (en, ru, path)) in [
                ("Home", "Домой", HOME),
                ("Documents", "Документы", "/users/default/documents"),
                ("Downloads", "Загрузки", "/users/default/downloads"),
                ("bOS Live", "bOS Live", "/"),
            ]
            .iter()
            .enumerate()
            {
                let r = rect(
                    x + 6,
                    y + 14 + i as i32 * 37 + if i == 3 { 17 } else { 0 },
                    side - 12,
                    31,
                );
                if f.path == *path {
                    ui.c.round(r, 5, mix(p.bar(), p.accent(), 63));
                }
                symbol(
                    ui.c,
                    rect(r.x + 7, r.y + 6, 18, 18),
                    [
                        Symbol::Home,
                        Symbol::Document,
                        Symbol::Download,
                        Symbol::Drive,
                    ][i],
                    p.ink(),
                );
                ui.c.label(r.x + 35, r.y + 6, l.t(en, ru), p.ink(), 0, r.w - 29);
                ui.hit(r, Cmd::Location(i as u8));
            }
            let grid = rect(x + side + 14, y + 60, w - side - 26, h - 68);
            let title = if f.path == HOME {
                l.t("Home folder", "Домашняя папка")
            } else if f.path == "/" {
                "/"
            } else {
                filename(f.path.rsplit('/').next().unwrap_or(""), l)
            };
            symbol(ui.c, rect(grid.x, y + 12, 21, 21), Symbol::Home, p.ink());
            ui.c.label(grid.x + 31, y + 10, title, p.ink(), 2, grid.w - 126);
            for (j, shape) in [Symbol::Grid, Symbol::List].into_iter().enumerate() {
                let r = rect(x + w - 116 + j as i32 * 34, y + 8, 30, 29);
                if f.list == (j == 1) {
                    ui.c.round(r, 5, mix(p.bar(), p.accent(), 60));
                }
                symbol(ui.c, r.inset(6), shape, p.ink());
                if f.list != (j == 1) {
                    ui.hit(r, Cmd::ToggleList);
                }
            }
            ui.button(rect(x + w - 40, y + 9, 29, 27), "…", Cmd::Menu, false);
            let cols = if f.list {
                1
            } else {
                (grid.w / 105).clamp(1, 3)
            };
            let cw = grid.w / cols;
            let rowh = if f.list {
                34
            } else {
                (grid.h / 2).clamp(74, 170)
            };
            for (i, item) in f.items.iter().enumerate().skip(f.offset) {
                let n = (i - f.offset) as i32;
                let r = rect(
                    grid.x + n % cols * cw,
                    grid.y + n / cols * rowh,
                    cw - 3,
                    rowh - 3,
                );
                if r.y >= y + h - 6 {
                    break;
                }
                if f.selected == Some(i) {
                    ui.c.round(r, 7, mix(p.paper(), p.accent(), 53));
                }
                let size = if f.list {
                    25
                } else {
                    (r.h - 46).clamp(32, 92).min(r.w - 12)
                };
                let ix = if f.list {
                    r.x + 3
                } else {
                    r.x + (r.w - size) / 2
                };
                let iy = if f.list { r.y + 3 } else { r.y + 14 };
                let ir = rect(ix, iy, size, size);
                if item.file_type == crate::fs::FileType::Directory {
                    super::icons::folder(ui.c, ir, &item.name);
                } else if item.name == "Coast.jpg" || item.name == "Cliffs.jpg" {
                    ui.c.picture(
                        rect(ix - 3, iy + 4, size + 6, size - 6),
                        if item.name == "Coast.jpg" {
                            coast
                        } else {
                            cliffs
                        },
                    );
                } else {
                    ui.c.round(ir.inset(3), 6, p.line());
                    ui.c.round(ir.inset(4), 5, if p.dark { 0x535361 } else { 0xf2f0f4 });
                    for j in 0..3 {
                        ui.c.fill(
                            rect(ix + size / 4, iy + size / 3 + j * size / 7, size / 2, 2),
                            p.muted(),
                        );
                    }
                }
                let label = filename(&item.name, l);
                if f.list {
                    ui.c.label(ix + 34, r.y + 6, label, p.ink(), 0, r.w - 40);
                } else {
                    ui.c.centered(rect(r.x, iy + size + 3, r.w, 22), label, p.ink(), 0);
                }
                ui.hit(r, Cmd::File(i));
            }
            if f.items.is_empty() {
                ui.dim(
                    grid.x,
                    grid.y + 18,
                    l.t("Empty folder", "Пустая папка"),
                    grid.w,
                );
            }
        }
        Model::Browser(br) => {
            let bar = rect(x, y, w, 40);
            ui.c.fill(bar, p.bar());
            ui.button(rect(x + 5, y + 4, 27, 29), "←", Cmd::Back, false);
            ui.button(rect(x + 33, y + 4, 26, 29), "→", Cmd::Forward, false);
            ui.button(rect(x + 60, y + 4, 27, 29), "↻", Cmd::Reload, false);
            ui.input(
                rect(x + 92, y + 4, w - 130, 29),
                &br.address.value,
                br.editing,
                Cmd::Address,
            );
            ui.button(rect(x + w - 32, y + 4, 27, 29), "→", Cmd::Go, false);
            if br.home() {
                ui.text(x + 21, y + 56, l.t("Bookmarks", "Закладки"), 3);
                for (i, (name, url)) in BOOKMARKS.iter().enumerate() {
                    let r = rect(x + 16, y + 106 + i as i32 * 53, w - 32, 51);
                    if br.link == Some(i) {
                        ui.c.round(r, 6, mix(p.paper(), p.accent(), 42));
                    }
                    symbol(
                        ui.c,
                        rect(r.x + 5, r.y + 11, 22, 22),
                        [Symbol::Globe, Symbol::Document, Symbol::Link][i],
                        p.ink(),
                    );
                    ui.text(
                        r.x + 43,
                        r.y + 4,
                        match i {
                            0 => l.t(name, "Википедия"),
                            1 => l.t(name, "Изучить Rust"),
                            _ => l.t(name, "Пример страницы"),
                        },
                        1,
                    );
                    ui.dim(
                        r.x + 43,
                        r.y + 24,
                        url.trim_start_matches("https://").trim_end_matches('/'),
                        r.w - 55,
                    );
                    ui.c.fill(rect(r.x + 42, r.y + 48, r.w - 48, 1), p.line());
                    ui.hit(r, Cmd::Link(i));
                }
            } else {
                let area = rect(x, y + 40, w, h - 63);
                ui.c.fill(area, 0xffffff);
                if let Some(f) = &br.frame {
                    ui.c.pixels(area, &f.pixels, f.width, f.height);
                    if f.document_height > f.height {
                        let thumb = (area.h * f.height as i32 / f.document_height as i32).max(12);
                        let pos = (i64::from(area.h - thumb) * f.scroll as i64
                            / (f.document_height - f.height) as i64)
                            as i32;
                        ui.c.round(rect(x + w - 5, area.y + pos, 4, thumb), 2, p.muted());
                    }
                }
                ui.hit(area, Cmd::WebContent);
            }
            if br.error || !br.home() || br.loading {
                let status = if br.error {
                    super::local_error(&br.status, l).into()
                } else if br.loading {
                    l.t("Loading page…", "Загрузка страницы…").into()
                } else {
                    format!(
                        "{}  {}   {}",
                        if br.history[br.index].starts_with("https:") {
                            "HTTPS"
                        } else {
                            "HTTP"
                        },
                        br.code,
                        br.status
                    )
                };
                ui.c.fill(rect(x, y + h - 23, w, 23), p.bar());
                ui.dim(x + 12, y + h - 19, &status, w - 24);
            }
        }
        Model::Terminal(t) => {
            ui.c.fill(b, 0x1d1f29);
            let cols = ((w - 26) / 8).max(1) as usize;
            let rows = ((h - 49) / 19).max(1) as usize;
            let count = t.log.rows(cols).count();
            let end = count.saturating_sub(t.scroll);
            let start = end.saturating_sub(rows);
            for (row, cells) in t.log.rows(cols).skip(start).take(end - start).enumerate() {
                let py = y + 8 + row as i32 * 19;
                let selected = t
                    .selection
                    .is_some_and(|(a, b)| (a.min(b)..=a.max(b)).contains(&(start + row)));
                if selected {
                    ui.c.fill(rect(x + 8, py - 1, w - 16, 19), 0x4b4966);
                }
                for (i, cell) in cells.iter().enumerate() {
                    let px = x + 13 + i as i32 * 8;
                    if let Some(bg) = cell.bg.filter(|_| !selected) {
                        ui.c.fill(rect(px, py, 8, 16), bg);
                    }
                    ui.c.mono(px, py, cell.ch, cell.fg);
                }
            }
            ui.c.fill(rect(x + 11, y + h - 39, w - 22, 1), 0x353746);
            let dir = t
                .cwd
                .strip_prefix(HOME)
                .map_or_else(|| t.cwd.clone(), |p| format!("~{p}"));
            let prompt = if let Some(pid) = t.job {
                format!("[{pid}] › ")
            } else {
                format!("{dir} % ")
            };
            let plen = prompt.chars().count().min(cols / 2);
            let py = y + h - 28;
            for (i, ch) in prompt.chars().take(plen).enumerate() {
                ui.c.mono(x + 13 + i as i32 * 8, py, ch, 0xa8dfc3);
            }
            let available = cols.saturating_sub(plen).max(2);
            let cursor = t.line.value[..t.line.pos].chars().count();
            let skip = cursor.saturating_sub(available - 1);
            let (a, z) = t.line.selection();
            for (i, (idx, ch)) in t
                .line
                .value
                .char_indices()
                .skip(skip)
                .take(available)
                .enumerate()
            {
                let px = x + 13 + (plen + i) as i32 * 8;
                if (a..z).contains(&idx) {
                    ui.c.fill(rect(px, py, 8, 17), 0x4b4966);
                }
                ui.c.mono(px, py, ch, 0xf4f0f8);
            }
            ui.c.fill(
                rect(x + 13 + (plen + cursor - skip) as i32 * 8, py, 2, 16),
                0xaadcc4,
            );
            ui.hit(b, Cmd::Input);
        }
        Model::Notes(n) => {
            ui.c.fill(rect(x, y, w, 38), p.bar());
            ui.dim(x + 12, y + 10, &n.path, w - 166);
            ui.button(
                rect(x + w - 145, y + 5, 68, 27),
                l.t("Save as", "Копия"),
                Cmd::SaveAs,
                false,
            );
            ui.button(
                rect(x + w - 72, y + 5, 66, 27),
                l.t("Save", "Сохранить"),
                Cmd::Save,
                true,
            );
            ui.c.fill(rect(x + 1, y + 39, 38, h - 40), p.bar());
            let r = rect(x + 50, y + 48, w - 62, h - 57);
            let mut at = 0;
            let mut row = 0;
            let (a, z) = n.text.selection();
            for line in n.text.value.split('\n') {
                if row >= n.scroll && (row - n.scroll) as i32 * 20 < r.h {
                    let ty = r.y + (row - n.scroll) as i32 * 20;
                    ui.c.label(x + 8, ty, &format!("{}", row + 1), p.muted(), 0, 27);
                    let lo = a.max(at);
                    let hi = z.min(at + line.len());
                    if lo < hi {
                        let left = width(&line[..lo - at], 0);
                        ui.c.fill(
                            rect(r.x + left, ty, width(&line[lo - at..hi - at], 0), 19),
                            mix(p.paper(), p.accent(), 95),
                        );
                    }
                    ui.c.label(r.x, ty, line, p.ink(), 0, r.w);
                    if (at..=at + line.len()).contains(&n.text.pos) {
                        let left = width(&line[..n.text.pos - at], 0);
                        if left < r.w {
                            ui.c.fill(rect(r.x + left, ty + 1, 1, 17), p.accent());
                        }
                    }
                }
                row += 1;
                at += line.len() + 1;
            }
            ui.hit(r, Cmd::Editor);
        }
        Model::Calculator(e) => {
            ui.input(rect(x + 16, y + 14, w - 32, 45), &e.value, true, Cmd::Input);
            let keys = [
                '7', '8', '9', '/', '4', '5', '6', '*', '1', '2', '3', '-', '0', 'C', '=', '+',
            ];
            let cw = (w - 29) / 4;
            let ch = ((h - 86) / 4).clamp(28, 69);
            for (i, key) in keys.iter().enumerate() {
                let r = rect(
                    x + 16 + (i as i32 % 4) * cw,
                    y + 73 + (i as i32 / 4) * ch,
                    cw - 6,
                    ch - 6,
                );
                ui.button(r, &format!("{key}"), Cmd::Calc(*key), *key == '=');
            }
        }
        Model::Settings => settings(ui, b, win.scroll),
        Model::Monitor => {
            ui.text(
                x + 18,
                y + 16,
                l.t("System activity", "Активность системы"),
                2,
            );
            let load = loads.last().copied().unwrap_or(0);
            ui.text(x + 18, y + 55, &format!("CPU    {load}%"), 1);
            let g = rect(x + 18, y + 89, w - 36, (h / 3).clamp(45, 150));
            ui.c.round(g, 7, if p.dark { 0x192e29 } else { 0xe3efe9 });
            for i in 1..4 {
                ui.c.fill(rect(g.x + 4, g.y + g.h * i / 4, g.w - 8, 1), 0x9abeaa);
            }
            for (i, vals) in loads.windows(2).enumerate() {
                ui.c.line(
                    g.x + 5 + i as i32 * (g.w - 10) / 59,
                    g.y + g.h - 4 - vals[0] as i32 * (g.h - 9) / 100,
                    g.x + 5 + (i + 1) as i32 * (g.w - 10) / 59,
                    g.y + g.h - 4 - vals[1] as i32 * (g.h - 9) / 100,
                    0x26885b,
                    2,
                );
            }
            let yy = g.y + g.h + 18;
            ui.text(
                x + 18,
                yy,
                &format!(
                    "{}   {} / {} MiB",
                    l.t("Allocated RAM", "Занято ОЗУ"),
                    crate::mm::phys::used_memory() / 1048576,
                    crate::mm::phys::total_memory() / 1048576
                ),
                1,
            );
            ui.dim(
                x + 18,
                yy + 35,
                &format!(
                    "{}   {}     ·     {}   {} s",
                    l.t("Open windows", "Открытые окна"),
                    windows,
                    l.t("Uptime", "Время работы"),
                    crate::boottime::elapsed_ms() / 1000
                ),
                w - 36,
            );
        }
        Model::Widgets {
            field,
            level,
            tab,
            focus,
            switch,
            segment,
            ..
        } => {
            let now = crate::boottime::elapsed_ms();
            let t = p.motion_ms();
            ui.text(
                x + 18,
                y + 13,
                l.t("Interface widgets", "Элементы интерфейса"),
                2,
            );
            ui.input(
                rect(x + 18, y + 55, w - 36, 32),
                &field.value,
                *focus == 0,
                Cmd::Input,
            );
            ui.text(x + 18, y + 110, l.t("Switch", "Переключатель"), 0);
            ui.toggle_at(
                rect(x + w - 68, y + 103, 46, 25),
                switch.value(now, t),
                Cmd::WidgetToggle,
            );
            let slider = rect(x + 20, y + 151, w - 44, 33);
            ui.c.round(rect(slider.x, slider.y + 13, slider.w, 5), 2, p.line());
            let knob = *level as i32 * (slider.w - 16) / 4;
            ui.c.round(rect(slider.x, slider.y + 13, knob + 8, 5), 2, p.accent());
            ui.c.round(rect(slider.x + knob, slider.y + 7, 17, 17), 8, p.accent());
            for i in 0..5 {
                ui.hit(
                    rect(
                        slider.x + i * slider.w / 5,
                        slider.y,
                        slider.w / 5 + 1,
                        slider.h,
                    ),
                    Cmd::Slider(i as u8),
                );
            }
            let seg = rect(x + 18, y + 202, w - 36, 31);
            let cw = seg.w / 3;
            ui.c.round(seg, 8, p.bar());
            ui.c.round(
                rect(seg.x + segment.value(now, t) * cw / 256, seg.y, cw, seg.h),
                8,
                p.accent(),
            );
            for (i, label) in [
                l.t("First", "Первая"),
                l.t("Second", "Вторая"),
                l.t("Third", "Третья"),
            ]
            .iter()
            .enumerate()
            {
                let r = rect(seg.x + i as i32 * cw, seg.y, cw, seg.h);
                ui.c.centered(
                    r,
                    label,
                    if *tab as usize == i {
                        0xffffff
                    } else {
                        p.ink()
                    },
                    0,
                );
                ui.hit(r, Cmd::WidgetsTab(i as u8));
            }
            let focus_rect = match *focus {
                1 => Some(rect(x + w - 68, y + 103, 46, 25)),
                2 => Some(slider),
                3 => Some(seg),
                4 => Some(rect(x + 18, y + 258, 126, 32)),
                5 => Some(rect(x + 155, y + 258, 150, 32)),
                _ => None,
            };
            if let Some(r) = focus_rect {
                ui.c.fill(rect(r.x + 3, r.y + r.h + 2, r.w - 6, 2), p.accent());
            }
            ui.button(
                rect(x + 155, y + 258, 150, 32),
                l.t("Save note", "Сохранить заметку"),
                Cmd::WidgetSave,
                false,
            );
            ui.button(
                rect(x + 18, y + 258, 126, 32),
                l.t("Test sound", "Проверить звук"),
                Cmd::Beep,
                true,
            );
        }
        Model::Snake(s) => {
            let cell = ((w - 36) / 24).min((h - 77) / 16).clamp(3, 23);
            let g = rect(x + (w - cell * 24) / 2, y + 60, cell * 24, cell * 16);
            ui.text(
                x + 16,
                y + 15,
                &format!("{}   {}", l.t("Score", "Счёт"), s.body.len() - 3),
                1,
            );
            ui.button(
                rect(x + w - 150, y + 8, 136, 31),
                if s.over {
                    l.t("Play again", "Ещё раз")
                } else if s.running {
                    l.t("Pause", "Пауза")
                } else {
                    l.t("Play", "Играть")
                },
                Cmd::Play,
                true,
            );
            ui.c.round(g, 5, 0x193c2c);
            for (i, (px, py)) in s.body.iter().enumerate() {
                ui.c.round(
                    rect(g.x + px * cell, g.y + py * cell, cell - 1, cell - 1),
                    2,
                    if i == 0 { 0xb8eb7f } else { 0x80c558 },
                );
            }
            ui.c.round(
                rect(
                    g.x + s.food.0 * cell,
                    g.y + s.food.1 * cell,
                    cell - 1,
                    cell - 1,
                ),
                3,
                0xf27869,
            );
            ui.hit(g, Cmd::Play);
        }
        Model::About => {
            icon(ui.c, rect(x + 24, y + 27, 67, 67), Some(App::About));
            ui.text(x + 112, y + 27, "bOS 1.0", 3);
            ui.dim(
                x + 113,
                y + 65,
                l.t("Native live desktop", "Графическая live-система"),
                w - 130,
            );
            for (i, s) in [
                l.t("Kernel", "Ядро"),
                l.t("Architecture", "Архитектура"),
                l.t("Display", "Дисплей"),
                l.t("Build", "Сборка"),
            ]
            .iter()
            .enumerate()
            {
                let yy = y + 125 + i as i32 * 36;
                ui.dim(x + 24, yy, s, w / 3);
                ui.text(
                    x + w / 3,
                    yy,
                    &[
                        String::from("bOS 1.0.0"),
                        "x86-64".into(),
                        format!("{} × {}", ui.c.w, ui.c.h),
                        env!("BOS_SOURCE_REVISION").into(),
                    ][i],
                    0,
                );
            }
        }
    }
}
pub fn sidebar(w: i32) -> i32 {
    (w / 4).clamp(90, 172)
}
fn settings(ui: &mut Ui<'_>, r: Rect, scroll: i32) {
    let p = ui.p;
    let l = p.lang;
    let x = r.x + 18;
    let w = r.w - 36;
    let y = r.y + 12 - scroll;
    ui.text(x, y, l.t("Preferences", "Параметры"), 2);
    let rows = [
        (Setting::Language, "Language", "Язык"),
        (Setting::Theme, "Appearance", "Оформление"),
        (Setting::Accent, "Accent colour", "Акцентный цвет"),
        (Setting::Wallpaper, "Wallpaper", "Обои"),
        (Setting::Animation, "Animations", "Анимации"),
        (Setting::Speed, "Animation speed", "Скорость анимации"),
        (Setting::Shadow, "Window shadows", "Тени окон"),
        (Setting::Glass, "Translucency", "Прозрачность"),
        (Setting::Seconds, "Clock seconds", "Секунды в часах"),
        (Setting::Mouse, "Pointer speed", "Скорость мыши"),
    ];
    for (i, (set, en, ru)) in rows.iter().enumerate() {
        let yy = y + 41 + i as i32 * 37;
        ui.text(x, yy + 5, l.t(en, ru), 0);
        let left = x + (w * 40 / 100);
        let right = r.x + r.w - 18;
        let space = right - left;
        let mut options = |items: &[&str], selected: u8, ui: &mut Ui<'_>| {
            let n = items.len() as i32;
            for (j, label) in items.iter().enumerate() {
                ui.button(
                    rect(left + space * j as i32 / n, yy, space / n - 4, 28),
                    label,
                    Cmd::Set(*set, j as u8),
                    selected == j as u8,
                );
            }
        };
        match set {
            Setting::Language => options(
                &["English", "Русский"],
                if l == Lang::En { 0 } else { 1 },
                ui,
            ),
            Setting::Theme => options(
                &[l.t("Light", "Светлая"), l.t("Dark", "Тёмная")],
                p.dark as u8,
                ui,
            ),
            Setting::Accent => {
                for (j, color) in [0x6054d2, 0x2970ca, 0x13805e, 0xa05a16, 0xb74170, 0x5b6377]
                    .iter()
                    .enumerate()
                {
                    let a = rect(left + j as i32 * (space / 6), yy + 1, 25, 25);
                    ui.c.round(a, 12, *color);
                    if p.accent as usize == j {
                        ui.c.centered(a, "✓", 0xffffff, 1);
                    }
                    ui.hit(a, Cmd::Set(*set, j as u8));
                }
            }
            Setting::Wallpaper => options(&["1", "2", "3", "4", "5"], p.wall, ui),
            Setting::Speed => options(
                &[
                    l.t("Slow", "Медленно"),
                    l.t("Normal", "Средне"),
                    l.t("Fast", "Быстро"),
                ],
                p.speed,
                ui,
            ),
            Setting::Mouse => options(&["1", "2", "3", "4"], p.mouse, ui),
            _ => {
                let on = match set {
                    Setting::Animation => p.animations,
                    Setting::Shadow => p.shadows,
                    Setting::Glass => p.glass,
                    Setting::Seconds => p.seconds,
                    _ => false,
                };
                ui.toggle(
                    rect(right - 47, yy + 2, 43, 24),
                    on,
                    Cmd::Set(*set, !on as u8),
                );
            }
        }
    }
    if r.h < 430 {
        ui.button(
            rect(r.x + r.w - 28, r.y + r.h - 27, 24, 24),
            "↓",
            Cmd::Scroll(60),
            false,
        );
        ui.button(
            rect(r.x + r.w - 55, r.y + r.h - 27, 24, 24),
            "↑",
            Cmd::Scroll(-60),
            false,
        );
    }
}
