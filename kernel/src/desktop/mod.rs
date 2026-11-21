//! Native window manager. Each application owns its state and the compositor
//! builds the input hit regions from the same layout that it displays.
mod canvas;
mod chrome;
mod edit;
mod icons;
mod model;
mod motion;
mod preferences;
mod terminal;
mod ui;
mod views;
use crate::{
    arch::x86_64::{apic, serial},
    bsh::line::{self, Key},
    drivers::{console, keyboard, mouse, net},
    fs::{FileType, VFS},
};
use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use canvas::{rect, width, Canvas, Picture, Rect};
use edit::Edit;
use model::*;
use preferences::{Lang, Preferences};
use spin::Mutex;
use ui::*;

type MenuItem = (&'static str, &'static str, Cmd);
enum Popup {
    Calendar,
    Network,
    Menu {
        id: u32,
        x: i32,
        y: i32,
        items: Vec<MenuItem>,
    },
}
#[derive(Clone, Copy, PartialEq)]
enum Task {
    Folder,
    File,
    Rename,
    Delete,
    SaveAs,
    Discard,
    Restart,
    Off,
}
struct Dialog {
    task: Task,
    id: u32,
    field: Edit,
}
impl Dialog {
    fn editable(&self) -> bool {
        matches!(
            self.task,
            Task::Folder | Task::File | Task::Rename | Task::SaveAs
        )
    }
    fn title(&self, l: Lang) -> &'static str {
        let (en, ru) = match self.task {
            Task::Folder => ("New folder", "Новая папка"),
            Task::File => ("New file", "Новый файл"),
            Task::Rename => ("Rename", "Переименовать"),
            Task::Delete => ("Delete the selected item?", "Удалить выбранный объект?"),
            Task::SaveAs => ("Save note as", "Сохранить заметку как"),
            Task::Discard => (
                "Discard unsaved changes?",
                "Отменить несохранённые изменения?",
            ),
            Task::Restart => ("Restart bOS?", "Перезагрузить bOS?"),
            Task::Off => ("Shut down bOS?", "Выключить bOS?"),
        };
        l.t(en, ru)
    }
}
struct Desktop {
    russian_input: bool,
    p: Preferences,
    windows: Vec<Window>,
    next: u32,
    w: i32,
    h: i32,
    overview: bool,
    overview_stamp: u64,
    app_grid: bool,
    search: Edit,
    selected: Option<usize>,
    popup: Option<Popup>,
    dialog: Option<Dialog>,
    toast: Option<(String, u64)>,
    hits: Vec<Hit>,
    drag: Option<(u32, Cmd, i32, i32, Rect)>,
    surface: Option<motion::Surface>,
    selection_drag: Option<(u32, usize)>,
    last_click: (u32, Cmd, u64),
    calendar_offset: i32,
    file_clip: Option<(String, bool)>,
    coast: Picture,
    cliffs: Picture,
    loads: Vec<u8>,
    time: u64,
    online: bool,
    quit: bool,
    wall_dirty: bool,
}
static SESSION: Mutex<Option<Desktop>> = Mutex::new(None);
impl Desktop {
    fn new(w: i32, h: i32) -> Option<Self> {
        let mut d = Self {
            russian_input: false,
            p: Preferences::load(),
            w,
            h,
            windows: Vec::new(),
            next: 1,
            overview: false,
            overview_stamp: 0,
            app_grid: true,
            search: Edit::default(),
            selected: None,
            popup: None,
            dialog: None,
            toast: None,
            hits: Vec::new(),
            drag: None,
            surface: None,
            selection_drag: None,
            last_click: (0, Cmd::Focus, 0),
            calendar_offset: 0,
            file_clip: None,
            coast: Picture::jpeg(COAST)?,
            cliffs: Picture::jpeg(CLIFFS)?,
            loads: vec![0; 60],
            time: net::unix_seconds().unwrap_or(0),
            online: false,
            quit: false,
            wall_dirty: true,
        };
        d.launch(App::Files, false);
        d.launch(App::Browser, false);
        d.windows[0].transition = None;
        Some(d)
    }
    fn launch(&mut self, app: App, new: bool) -> u32 {
        self.overview = false;
        self.popup = None;
        if !new {
            if let Some(i) = self.windows.iter().rposition(|w| w.kind == app) {
                let id = self.windows[i].id;
                self.raise(id);
                return id;
            }
        }
        if self.windows.len() >= 32 {
            self.notice("Window limit reached", "Достигнут предел окон");
            return self.windows.last().map_or(0, |w| w.id);
        }
        let id = self.next;
        self.next += 1;
        let mut win = Window::new(id, app, self.w, self.h);
        let count = self.windows.iter().filter(|w| w.kind == app).count() as i32;
        win.frame.x = (win.frame.x + (count % 5) * 17).clamp(0, self.w - win.frame.w);
        win.frame.y =
            (win.frame.y + (count % 5) * 17).clamp(32, (self.h - win.frame.h - 15).max(32));
        self.windows.push(win);
        log::info!("Desktop app: {}", app.name(Lang::En));
        id
    }
    fn raise(&mut self, id: u32) {
        if let Some(i) = self.index(id) {
            let mut w = self.windows.remove(i);
            if w.minimized || w.transition == Some(Transition::Minimize) {
                w.opened = u64::MAX;
                w.transition = Some(Transition::Restore);
            }
            w.minimized = false;
            self.windows.push(w);
        }
    }
    fn animate(&mut self, index: usize, kind: Transition) {
        let win = &mut self.windows[index];
        win.opened = u64::MAX;
        win.transition = Some(kind);
    }
    fn step_animations(&mut self, now: u64) -> bool {
        let millis = if self.p.animations {
            [340, 245, 160][self.p.speed as usize]
        } else {
            0
        };
        let short = self.p.motion_ms();
        let mut dirty = short > 0 && (
            self.overview && now.saturating_sub(self.overview_stamp) <= short+16
            || self.toast.as_ref().is_some_and(|(_,end)| { let left=end.saturating_sub(now);left<short+16 || left>=2200-short-16 })
            || self.windows.iter().any(|w|!w.minimized && matches!(&w.model,Model::Widgets{switch,segment,..} if switch.active(now,short) || segment.active(now,short)))
        );
        if self.toast.as_ref().is_some_and(|(_, end)| *end <= now) {
            self.toast = None;
            dirty = true;
        }
        let front = self
            .windows
            .iter()
            .rev()
            .find(|w| !w.minimized)
            .map(|w| w.id);
        self.windows.retain_mut(|w| {
            let Some(effect) = w.transition else {
                return true;
            };
            dirty = true;
            if w.opened == u64::MAX && (self.overview || Some(w.id) != front) {
                w.opened = now;
            }
            if now.saturating_sub(w.opened) < millis {
                return true;
            }
            w.transition = None;
            match effect {
                Transition::Minimize => w.minimized = true,
                Transition::Close => {
                    if let Model::Terminal(t) = &w.model {
                        if let Some(pid) = t.job {
                            crate::user::jobs::kill(pid);
                        }
                    }
                    return false;
                }
                _ => {}
            }
            true
        });
        dirty
    }
    fn index(&self, id: u32) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }
    fn active(&self) -> u32 {
        self.windows
            .iter()
            .rev()
            .find(|w| !w.minimized)
            .map_or(0, |w| w.id)
    }
    fn filtered(&self) -> Vec<App> {
        let q = self.search.value.to_lowercase();
        ALL.iter()
            .copied()
            .filter(|a| {
                a.name(self.p.lang).to_lowercase().contains(&q)
                    || a.name(Lang::En).to_lowercase().contains(&q)
            })
            .collect()
    }
    fn notice(&mut self, en: &str, ru: &str) {
        self.toast = Some((
            self.p.lang.t(en, ru).into(),
            crate::boottime::elapsed_ms() + 2200,
        ));
    }
    fn overview(&mut self) {
        self.popup = None;
        self.overview = !self.overview;
        self.overview_stamp = crate::boottime::elapsed_ms();
        self.search = Edit::default();
        self.selected = None;
        self.app_grid = true;
        log::info!("Desktop overview: {}", self.overview);
    }
    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        self.hits
            .iter()
            .rev()
            .find(|h| h.area.contains(x, y))
            .copied()
    }
    fn menu(&mut self, id: u32, x: i32, y: i32, items: Vec<MenuItem>) {
        self.popup = Some(Popup::Menu { id, x, y, items });
    }
    fn context(&mut self, id: u32, x: i32, y: i32) {
        let kind = self.index(id).map(|i| self.windows[i].kind);
        let items = match kind {
            Some(App::Files) => vec![
                ("Open", "Открыть", Cmd::Open),
                ("New folder", "Новая папка", Cmd::NewFolder),
                ("New file", "Новый файл", Cmd::NewFile),
                ("Rename", "Переименовать", Cmd::Rename),
                ("Copy", "Копировать", Cmd::Copy),
                ("Cut", "Вырезать", Cmd::Cut),
                ("Paste", "Вставить", Cmd::Paste),
                ("Delete", "Удалить", Cmd::Remove),
                ("New window", "Новое окно", Cmd::NewWindow),
            ],
            Some(App::Notes) => vec![
                ("Save", "Сохранить", Cmd::Save),
                ("Save as", "Сохранить как", Cmd::SaveAs),
                ("Cut", "Вырезать", Cmd::Cut),
                ("Copy", "Копировать", Cmd::Copy),
                ("Paste", "Вставить", Cmd::Paste),
                ("Select all", "Выделить всё", Cmd::SelectAll),
                ("New window", "Новое окно", Cmd::NewWindow),
            ],
            Some(App::Terminal | App::Browser | App::Widgets) => vec![
                ("Copy", "Копировать", Cmd::Copy),
                ("Paste", "Вставить", Cmd::Paste),
                ("Select all", "Выделить всё", Cmd::SelectAll),
                ("New window", "Новое окно", Cmd::NewWindow),
                ("Close", "Закрыть", Cmd::Close),
            ],
            Some(_) => vec![
                ("Minimize", "Свернуть", Cmd::Minimize),
                ("Close", "Закрыть", Cmd::Close),
            ],
            None => vec![
                ("Files", "Файлы", Cmd::Launch(App::Files)),
                ("Notes", "Заметки", Cmd::Launch(App::Notes)),
                ("Change wallpaper", "Сменить обои", Cmd::Wallpaper),
                ("Switch theme", "Сменить тему", Cmd::Theme),
                ("Applications", "Приложения", Cmd::Overview),
                ("Settings", "Настройки", Cmd::Launch(App::Settings)),
                ("Terminal", "Терминал", Cmd::Launch(App::Terminal)),
                ("Console", "Консоль", Cmd::Console),
            ],
        };
        self.menu(id, x, y, items);
    }
    fn ask(&mut self, id: u32, task: Task, value: &str) {
        let mut field = Edit::new(value);
        field.select_all();
        self.dialog = Some(Dialog { id, task, field });
    }
    fn confirm(&mut self) {
        let Some(d) = self.dialog.take() else {
            return;
        };
        if d.task == Task::Off || d.task == Task::Restart {
            crate::bsh::power(d.task == Task::Restart);
            return;
        }
        let Some(i) = self.index(d.id) else {
            return;
        };
        let name = d.field.value.trim();
        if d.task == Task::Discard {
            self.animate(i, Transition::Close);
            return;
        }
        if d.task == Task::SaveAs {
            if let Model::Notes(n) = &mut self.windows[i].model {
                if name.is_empty() {
                    return;
                }
                let Ok(path) = crate::fs::normalize_path(HOME, name) else {
                    return;
                };
                if VFS
                    .lock()
                    .write_file(&path, n.text.value.as_bytes())
                    .is_ok()
                {
                    n.path = path;
                    n.dirty = false;
                    self.notice("Saved", "Сохранено");
                } else {
                    self.notice("Could not save file", "Не удалось сохранить файл");
                }
            }
            return;
        }
        let Model::Files(f) = &mut self.windows[i].model else {
            return;
        };
        if matches!(d.task, Task::File | Task::Folder | Task::Rename)
            && (name.is_empty() || name.contains('/') || name == "." || name == "..")
        {
            self.notice("Invalid name", "Недопустимое имя");
            return;
        }
        let p = join(&f.path, name);
        let result = {
            let mut v = VFS.lock();
            match d.task {
                Task::File => v.create(&p, FileType::RegularFile),
                Task::Folder => v.mkdir(&p),
                Task::Rename => f
                    .selected_path()
                    .ok_or(crate::fs::FileSystemError::NotFound)
                    .and_then(|a| v.rename(&a, &p)),
                Task::Delete => f
                    .selected_path()
                    .ok_or(crate::fs::FileSystemError::NotFound)
                    .and_then(|a| v.remove(&a)),
                _ => return,
            }
        };
        f.refresh();
        if result.is_err() {
            self.notice("File operation failed", "Операция с файлом не удалась");
        } else {
            self.notice("Done", "Готово");
        }
    }
    fn open_file(&mut self, i: usize) {
        let Model::Files(f) = &mut self.windows[i].model else {
            return;
        };
        let Some(path) = f.selected_path() else {
            return;
        };
        if VFS.lock().stat(&path).is_ok_and(|s| s.is_dir()) {
            f.enter(&path, true);
            return;
        }
        let bytes = {
            let mut v = VFS.lock();
            if !v.stat(&path).is_ok_and(|s| s.size <= 2 * 1024 * 1024) {
                return;
            }
            v.read_file(&path)
        };
        if let Ok(bytes) = bytes {
            if path.to_ascii_lowercase().ends_with(".jpg") {
                if let Some(image) = Picture::jpeg(&bytes) {
                    let id = self.launch(App::Files, true);
                    if let Some(j) = self.index(id) {
                        self.windows[j].model = Model::Image(path, image);
                    }
                    return;
                }
            }
            if bytes.starts_with(b"\x7fELF") {
                self.notice(
                    "Use run in Terminal to start this program",
                    "Запустите программу командой run в терминале",
                );
                return;
            }
            let id = self.launch(App::Notes, true);
            if let Some(j) = self.index(id) {
                if let Model::Notes(n) = &mut self.windows[j].model {
                    n.path = path;
                    n.text = Edit::new(&String::from_utf8_lossy(&bytes));
                    n.text.place(0, false);
                }
            }
        }
    }
    fn setting(&mut self, s: Setting, v: u8) {
        match s {
            Setting::Language => self.p.lang = if v == 0 { Lang::En } else { Lang::Ru },
            Setting::Theme => self.p.dark = v != 0,
            Setting::Accent => self.p.accent = v.min(5),
            Setting::Wallpaper => {
                self.p.wall = v.min(4);
                self.wall_dirty = true;
            }
            Setting::Animation => self.p.animations = v != 0,
            Setting::Speed => self.p.speed = v.min(2),
            Setting::Shadow => self.p.shadows = v != 0,
            Setting::Glass => self.p.glass = v != 0,
            Setting::Seconds => self.p.seconds = v != 0,
            Setting::Mouse => self.p.mouse = v.min(3),
        }
        if !self.p.save() {
            self.notice(
                "Could not save preferences",
                "Не удалось сохранить настройки",
            );
        }
        log::info!(
            "Desktop preferences: language={} theme={}",
            self.p.lang.code(),
            if self.p.dark { "dark" } else { "light" }
        );
    }
    fn dispatch(&mut self, id: u32, cmd: Cmd, x: i32, y: i32) {
        let l = self.p.lang;
        match cmd {
            Cmd::Cancel => {
                self.dialog = None;
                self.popup = None;
                return;
            }
            Cmd::DialogOk => {
                self.confirm();
                return;
            }
            Cmd::Overview => {
                self.overview();
                return;
            }
            Cmd::OverviewTab(apps) => {
                self.app_grid = apps;
                self.selected = None;
                return;
            }
            Cmd::Launch(a) => {
                self.launch(a, false);
                return;
            }
            Cmd::Activate(n) => {
                self.overview = false;
                self.raise(n);
                return;
            }
            Cmd::Console => {
                self.quit = true;
                return;
            }
            Cmd::Today => {
                self.calendar_offset = 0;
                self.popup = Some(Popup::Calendar);
                return;
            }
            Cmd::Month(d) => {
                self.calendar_offset = (self.calendar_offset + d).clamp(-1200, 1200);
                self.popup = Some(Popup::Calendar);
                return;
            }
            Cmd::Wallpaper => {
                self.setting(Setting::Wallpaper, (self.p.wall + 1) % 5);
                return;
            }
            Cmd::Theme => {
                self.setting(Setting::Theme, (!self.p.dark) as u8);
                return;
            }
            Cmd::Calendar => {
                self.calendar_offset = 0;
                self.popup = if matches!(self.popup, Some(Popup::Calendar)) {
                    None
                } else {
                    Some(Popup::Calendar)
                };
                return;
            }
            Cmd::Network => {
                self.popup = if matches!(self.popup, Some(Popup::Network)) {
                    None
                } else {
                    Some(Popup::Network)
                };
                return;
            }
            Cmd::Sound => {
                let en = crate::arch::x86_64::toggle_sound();
                if en {
                    self.notice("Sound on", "Звук включён");
                } else {
                    self.notice("Sound muted", "Звук выключен");
                }
                return;
            }
            Cmd::Language => {
                self.setting(Setting::Language, (l == Lang::En) as u8);
                return;
            }
            Cmd::Power => {
                self.menu(
                    0,
                    x,
                    32,
                    vec![
                        ("Settings", "Настройки", Cmd::Launch(App::Settings)),
                        ("Console", "Консоль", Cmd::Console),
                        ("Restart", "Перезагрузить", Cmd::Reboot),
                        ("Shut down", "Выключить", Cmd::Shutdown),
                    ],
                );
                return;
            }
            Cmd::Shutdown | Cmd::Reboot => {
                self.ask(
                    id,
                    if cmd == Cmd::Shutdown {
                        Task::Off
                    } else {
                        Task::Restart
                    },
                    "bOS",
                );
                return;
            }
            Cmd::Set(s, v) => {
                self.setting(s, v);
                return;
            }
            _ => {}
        }
        let Some(i) = self.index(id) else {
            return;
        };
        let frame = self.windows[i].frame;
        let app = self.windows[i].kind;
        match cmd {
            Cmd::Close => {
                if matches!(&self.windows[i].model,Model::Notes(n)if n.dirty) {
                    self.ask(id, Task::Discard, "");
                } else {
                    self.animate(i, Transition::Close);
                }
                return;
            }
            Cmd::Minimize => {
                self.animate(i, Transition::Minimize);
                return;
            }
            Cmd::Maximize => {
                let win = &mut self.windows[i];
                if let Some(old) = win.restored.take() {
                    win.frame = old;
                } else {
                    win.restored = Some(frame);
                    win.frame = rect(3, 34, self.w - 6, self.h - 109);
                }
                return;
            }
            Cmd::Move | Cmd::Resize => {
                self.drag = Some((id, cmd, x, y, frame));
                return;
            }
            Cmd::NewWindow => {
                self.launch(app, true);
                return;
            }
            Cmd::Menu => {
                self.context(id, x, y);
                return;
            }
            _ => {}
        }
        let isfile = matches!(self.windows[i].model, Model::Files(_));
        if matches!(cmd, Cmd::Copy | Cmd::Cut | Cmd::Paste | Cmd::SelectAll) && !isfile {
            self.app_key(
                id,
                Key::Control(match cmd {
                    Cmd::Copy => b'c',
                    Cmd::Cut => b'x',
                    Cmd::Paste => b'v',
                    _ => b'a',
                }),
                false,
            );
            return;
        }
        match &mut self.windows[i].model {
            Model::Browser(b) => {
                let size = (frame.w - 2, frame.h - 103);
                let mut travel = None;
                let url = match cmd {
                    Cmd::Address => {
                        b.focus();
                        None
                    }
                    Cmd::Go => Some(b.address.value.clone()),
                    Cmd::Reload => Some(if b.error {
                        b.address.value.clone()
                    } else {
                        b.history[b.index].clone()
                    }),
                    Cmd::Home => Some("about:home".into()),
                    Cmd::Back if b.index > 0 => {
                        travel = Some(b.index - 1);
                        Some(b.history[b.index - 1].clone())
                    }
                    Cmd::Forward if b.index + 1 < b.history.len() => {
                        travel = Some(b.index + 1);
                        Some(b.history[b.index + 1].clone())
                    }
                    Cmd::Link(n) => b.follow(n),
                    Cmd::Scroll(n) => {
                        b.event(4, n * 34, 0, "");
                        None
                    }
                    Cmd::WebContent => {
                        b.editing = false;
                        b.event(2, x - frame.x - 1, y - frame.y - 77, "");
                        None
                    }
                    _ => None,
                };
                if let Some(u) = url {
                    b.go(&u, travel, size);
                }
            }
            Model::Files(f) => match cmd {
                Cmd::Location(n) => f.enter(
                    [
                        HOME,
                        "/users/default/documents",
                        "/users/default/downloads",
                        "/",
                    ][n as usize],
                    true,
                ),
                Cmd::Home => f.enter(HOME, true),
                Cmd::ToggleList => {
                    f.list = !f.list;
                    f.offset = 0;
                }
                Cmd::Back => {
                    if let Some(p) = f.previous.pop() {
                        f.next.push(f.path.clone());
                        f.enter(&p, false);
                    } else {
                        let parent = crate::fs::split_parent(&f.path).0;
                        f.enter(&parent, true);
                    }
                }
                Cmd::Forward => {
                    if let Some(p) = f.next.pop() {
                        f.previous.push(f.path.clone());
                        f.enter(&p, false);
                    }
                }
                Cmd::File(n) => {
                    f.selected = Some(n);
                }
                Cmd::Open => self.open_file(i),
                Cmd::NewFolder | Cmd::NewFile => self.ask(
                    id,
                    if cmd == Cmd::NewFile {
                        Task::File
                    } else {
                        Task::Folder
                    },
                    "",
                ),
                Cmd::Rename | Cmd::Remove => {
                    if let Some(n) = f
                        .selected
                        .and_then(|n| f.items.get(n))
                        .map(|e| e.name.clone())
                    {
                        self.ask(
                            id,
                            if cmd == Cmd::Rename {
                                Task::Rename
                            } else {
                                Task::Delete
                            },
                            &n,
                        );
                    }
                }
                Cmd::Copy | Cmd::Cut => {
                    if let Some(p) = f.selected_path() {
                        edit::copy(&p);
                        self.file_clip = Some((p, cmd == Cmd::Cut));
                        self.notice("Copied to clipboard", "Скопировано в буфер");
                    }
                }
                Cmd::Paste => {
                    if let Some((src, cut)) = self.file_clip.clone() {
                        let basename = src.rsplit('/').next().unwrap_or("");
                        let mut dst = join(&f.path, basename);
                        if cut && src == dst {
                            return;
                        }
                        if !cut && VFS.lock().stat(&dst).is_ok() {
                            let mut index = 1;
                            loop {
                                let name = if index == 1 {
                                    format!("Copy of {basename}")
                                } else {
                                    format!("Copy {index} of {basename}")
                                };
                                dst = join(&f.path, &name);
                                if VFS.lock().stat(&dst).is_err() {
                                    break;
                                }
                                index += 1;
                            }
                        }
                        let result = if VFS.lock().stat(&dst).is_ok() {
                            Err(crate::fs::FileSystemError::AlreadyExists)
                        } else if cut {
                            VFS.lock().rename(&src, &dst)
                        } else {
                            copy_tree(&src, &dst, 0)
                        };
                        f.refresh();
                        if result.is_ok() {
                            if cut {
                                self.file_clip = None;
                            }
                            self.notice("Pasted", "Вставлено");
                        } else {
                            self.notice("Could not paste item", "Не удалось вставить");
                        }
                    }
                }
                Cmd::Scroll(n) => {
                    f.offset = (f.offset as i32 + n)
                        .clamp(0, f.items.len().saturating_sub(1) as i32)
                        as usize;
                }
                _ => {}
            },
            Model::Notes(n) => match cmd {
                Cmd::Save => {
                    if VFS
                        .lock()
                        .write_file(&n.path, n.text.value.as_bytes())
                        .is_ok()
                    {
                        n.dirty = false;
                        self.notice("Saved", "Сохранено");
                    } else {
                        self.notice("Could not save file", "Не удалось сохранить файл");
                    }
                }
                Cmd::SaveAs => {
                    let path = n.path.clone();
                    self.ask(id, Task::SaveAs, &path);
                }
                Cmd::Editor => {
                    let pos = note_point(n, frame, x, y);
                    n.text.place(pos, false);
                    self.selection_drag = Some((id, pos));
                }
                Cmd::Scroll(d) => {
                    n.scroll =
                        (n.scroll as i32 + d).clamp(0, n.text.value.lines().count() as i32) as usize
                }
                _ => {}
            },
            Model::Calculator(e) => {
                if let Cmd::Calc(ch) = cmd {
                    if ch == 'C' {
                        *e = Edit::default();
                    } else if ch == '=' {
                        match crate::bsh::expression(&e.value) {
                            Ok(v) => {
                                *e = Edit::new(&format!("{v}"));
                                e.select_all();
                                log::info!("Calculator result: {v}");
                            }
                            Err(_) => self.notice("Invalid expression", "Ошибка выражения"),
                        }
                    } else {
                        let mut b = [0; 4];
                        e.insert(ch.encode_utf8(&mut b));
                    }
                }
            }
            Model::Snake(s) => {
                if cmd == Cmd::Play {
                    if s.over {
                        *s = Snake::new();
                    }
                    s.running = !s.running;
                    s.stamp = crate::boottime::elapsed_ms();
                }
            }
            Model::Widgets {
                field,
                checked,
                level,
                tab,
                switch,
                segment,
                focus,
            } => match cmd {
                Cmd::Input => *focus = 0,
                Cmd::WidgetToggle => {
                    *focus = 1;
                    *checked = !*checked;
                    switch.set(
                        *checked as i32 * 256,
                        crate::boottime::elapsed_ms(),
                        self.p.motion_ms(),
                    );
                }
                Cmd::WidgetsTab(v) => {
                    *focus = 3;
                    *tab = v;
                    segment.set(
                        v as i32 * 256,
                        crate::boottime::elapsed_ms(),
                        self.p.motion_ms(),
                    );
                }
                Cmd::Slider(v) => {
                    *focus = 2;
                    *level = v;
                }
                Cmd::WidgetSave => {
                    if VFS
                        .lock()
                        .write_file("/note.txt", field.value.as_bytes())
                        .is_ok()
                    {
                        self.notice("Saved to /note.txt", "Сохранено в /note.txt");
                    } else {
                        self.notice("Could not save file", "Не удалось сохранить файл");
                    }
                }
                Cmd::Beep => crate::arch::x86_64::speaker_beep(740, 70),
                _ => {}
            },
            Model::Settings => {
                if let Cmd::Scroll(d) = cmd {
                    self.windows[i].scroll =
                        (self.windows[i].scroll + d).clamp(0, (465 - frame.h + 37).max(0));
                }
            }
            Model::Terminal(t) => match cmd {
                Cmd::Scroll(n) => {
                    t.scroll = (t.scroll as i32 + n).clamp(
                        0,
                        t.log.rows(((frame.w - 28) / 8).max(1) as usize).count() as i32,
                    ) as usize
                }
                Cmd::Input => {
                    if y < frame.y + frame.h - 42 {
                        let count = t.log.rows(((frame.w - 28) / 8).max(1) as usize).count();
                        let rows = ((frame.h - 90) / 19).max(1) as usize;
                        let selected = count.saturating_sub(t.scroll + rows)
                            + (y - frame.y - 45).max(0) as usize / 19;
                        t.selection = Some((selected, selected));
                        self.selection_drag = Some((id, selected));
                    } else {
                        t.selection = None;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    fn key(&mut self, k: Key, shift: bool) {
        if let Some(d) = &mut self.dialog {
            match k {
                Key::Enter => self.confirm(),
                Key::Escape | Key::Control(b'c') => self.dialog = None,
                _ if d.editable() => {
                    d.field.key(k, shift, false);
                }
                _ => {}
            }
            return;
        }
        match k {
            Key::Console | Key::Control(b'q') => {
                self.quit = true;
                return;
            }
            Key::Super | Key::Control(b'o') => {
                self.overview();
                return;
            }
            Key::CloseWindow => {
                let id = self.active();
                if id != 0 {
                    self.dispatch(id, Cmd::Close, 0, 0);
                }
                return;
            }
            Key::AltTab => {
                self.overview = false;
                if self.windows.iter().filter(|w| !w.minimized).count() > 1 {
                    if let Some(i) = self.index(self.active()) {
                        let old = self.windows.remove(i);
                        self.windows.insert(0, old);
                    }
                }
                return;
            }
            Key::Escape => {
                self.overview = false;
                self.popup = None;
                if let Some(i) = self.index(self.active()) {
                    if let Model::Browser(b) = &mut self.windows[i].model {
                        b.stop();
                        b.editing = false;
                    }
                }
                return;
            }
            Key::Control(b'l') => {
                let id = self.launch(App::Browser, false);
                self.dispatch(id, Cmd::Address, 0, 0);
                return;
            }
            _ => {}
        }
        if self.overview {
            match k {
                Key::Enter => {
                    let apps = self.filtered();
                    if self.app_grid {
                        if let Some(a) = apps.get(self.selected.unwrap_or(0)) {
                            self.launch(*a, false);
                        }
                    } else if let Some(id) = self
                        .windows
                        .iter()
                        .find(|w| {
                            w.kind
                                .name(self.p.lang)
                                .to_lowercase()
                                .contains(&self.search.value.to_lowercase())
                        })
                        .map(|w| w.id)
                    {
                        self.overview = false;
                        self.raise(id);
                    }
                }
                Key::Tab | Key::Right | Key::Down | Key::Left | Key::Up => {
                    let n = self.filtered().len();
                    if n > 0 {
                        let delta = match k {
                            Key::Left => -1,
                            Key::Up => -5,
                            Key::Down => 5,
                            _ => 1,
                        };
                        self.selected = Some(
                            self.selected
                                .map_or(0, |i| (i as i32 + delta).rem_euclid(n as i32) as usize),
                        );
                    }
                }
                _ => {
                    self.search.key(k, shift, false);
                    self.selected = None;
                }
            }
            return;
        }
        self.popup = None;
        let id = self.active();
        if id != 0 {
            self.app_key(id, k, shift);
        }
    }
    fn app_key(&mut self, id: u32, k: Key, shift: bool) {
        let Some(i) = self.index(id) else {
            return;
        };
        let mut cmd = None;
        let frame = self.windows[i].frame;
        match &mut self.windows[i].model {
            Model::Browser(b) => {
                if b.editing && k != Key::Enter && k != Key::Tab {
                    b.address.key(k, shift, false);
                } else if k == Key::Tab && b.home() {
                    b.editing = false;
                    b.link = Some(b.link.map_or(0, |p| (p + 1) % BOOKMARKS.len()));
                } else {
                    cmd = match k {
                        Key::Enter if b.editing => Some(Cmd::Go),
                        Key::Enter if b.home() => b.link.map(Cmd::Link),
                        Key::Control(b'b') => Some(Cmd::Back),
                        Key::Control(b'f') => Some(Cmd::Forward),
                        Key::Control(b'h') => Some(Cmd::Home),
                        Key::Control(b'r') => Some(Cmd::Reload),
                        Key::Up => Some(Cmd::Scroll(-1)),
                        Key::Down => Some(Cmd::Scroll(1)),
                        Key::PageUp => Some(Cmd::Scroll(-10)),
                        Key::PageDown => Some(Cmd::Scroll(10)),
                        Key::Tab => {
                            b.editing = false;
                            b.event(7, 0, 0, "");
                            None
                        }
                        Key::Control(b'v') => {
                            b.event(3, 0, 0, &edit::paste());
                            None
                        }
                        Key::Text(ch) => {
                            b.event(3, 0, 0, ch.encode_utf8(&mut [0; 4]));
                            None
                        }
                        Key::Enter => {
                            b.event(6, 0, 0, "");
                            None
                        }
                        Key::Backspace => {
                            b.event(5, 0, 0, "");
                            None
                        }
                        _ => None,
                    }
                }
            }
            Model::Files(f) => {
                cmd = match k {
                    Key::Control(b'n') => Some(Cmd::NewFolder),
                    Key::Control(b'c') => Some(Cmd::Copy),
                    Key::Control(b'x') => Some(Cmd::Cut),
                    Key::Control(b'v') => Some(Cmd::Paste),
                    Key::Control(b'r') => Some(Cmd::Rename),
                    Key::Delete => Some(Cmd::Remove),
                    Key::Enter => Some(Cmd::Open),
                    Key::Backspace => Some(Cmd::Back),
                    Key::Up | Key::Left | Key::Down | Key::Right | Key::Tab
                        if !f.items.is_empty() =>
                    {
                        let n = f.items.len() as i32;
                        let d = if matches!(k, Key::Up | Key::Left) {
                            -1
                        } else {
                            1
                        };
                        let pos = f
                            .selected
                            .map_or(0, |p| (p as i32 + d).rem_euclid(n) as usize);
                        f.selected = Some(pos);
                        let count = if f.list {
                            ((frame.h - 110) / 34).max(1) as usize
                        } else {
                            6
                        };
                        if pos < f.offset || pos >= f.offset + count {
                            f.offset = pos.saturating_sub(count - 1);
                        }
                        None
                    }
                    Key::PageDown => Some(Cmd::Scroll(3)),
                    Key::PageUp => Some(Cmd::Scroll(-3)),
                    _ => None,
                };
            }
            Model::Notes(n) => {
                if k == Key::Control(b's') {
                    cmd = Some(Cmd::Save);
                } else if k == Key::PageUp || k == Key::PageDown {
                    cmd = Some(Cmd::Scroll(if k == Key::PageUp { -6 } else { 6 }));
                } else {
                    let before = n.text.value.clone();
                    if n.text.key(k, shift, true) {
                        n.dirty |= n.text.value != before;
                        let row = n.text.value[..n.text.pos]
                            .bytes()
                            .filter(|b| *b == b'\n')
                            .count();
                        let rows = ((frame.h - 100) / 20).max(1) as usize;
                        if row < n.scroll {
                            n.scroll = row;
                        } else if row >= n.scroll + rows {
                            n.scroll = row + 1 - rows;
                        }
                    }
                }
            }
            Model::Calculator(e) => {
                if matches!(k, Key::Enter | Key::Text('=')) {
                    cmd = Some(Cmd::Calc('='));
                } else {
                    e.key(k, shift, false);
                }
            }
            Model::Terminal(t) => {
                if let Some(pid) = t.job {
                    match k {
                        Key::Control(b'c') => {
                            crate::user::jobs::kill(pid);
                        }
                        Key::Enter => {
                            if crate::user::jobs::input(pid, &t.line.value) {
                                t.log.push_str(&t.line.value);
                                t.log.push_str("\n");
                                t.line = Edit::default();
                            } else {
                                self.notice("Input buffer is full", "Буфер ввода заполнен");
                            }
                        }
                        _ => {
                            t.line.key(k, shift, false);
                        }
                    }
                    return;
                }
                if k == Key::Enter {
                    let command = t.line.value.clone();
                    t.line = Edit::default();
                    if command.is_empty() {
                        return;
                    }
                    t.log
                        .push_str(&format!("\x1b[92m{} % \x1b[0m{command}\n", t.cwd));
                    t.history.push(command.clone());
                    if t.history.len() > 96 {
                        t.history.remove(0);
                    }
                    t.cursor = t.history.len();
                    t.scroll = 0;
                    t.selection = None;
                    let first = command.split_whitespace().next().unwrap_or("");
                    match first {
                        "clear" => t.log.clear(),
                        "exit" => cmd = Some(Cmd::Close),
                        "cli" => cmd = Some(Cmd::Console),
                        "settings" => cmd = Some(Cmd::Launch(App::Settings)),
                        "gui" => {}
                        "history" => {
                            for (n, s) in t.history.iter().enumerate() {
                                t.log.push_str(&format!("{}  {s}\n", n + 1));
                            }
                        }
                        "edit" | "notepad" | "babo" => {
                            let path = command
                                .split_whitespace()
                                .nth(1)
                                .and_then(|s| crate::fs::normalize_path(&t.cwd, s).ok());
                            let win = self.launch(App::Notes, true);
                            if let Some(path) = path {
                                if let Some(j) = self.index(win) {
                                    if let Model::Notes(n) = &mut self.windows[j].model {
                                        n.path = path.clone();
                                        if let Ok(b) = VFS.lock().read_file(&path) {
                                            n.text = Edit::new(&String::from_utf8_lossy(&b));
                                            n.text.place(0, false);
                                        }
                                    }
                                }
                            }
                            return;
                        }
                        "native" => cmd = Some(Cmd::Console),
                        "calculator" if command.trim() == "calculator" => {
                            cmd = Some(Cmd::Launch(App::Calculator))
                        }
                        _ => {
                            let (result, job) = crate::bsh::window_command(&command, &mut t.cwd);
                            t.job = job;
                            t.log.push_str(&result);
                            log::info!("Terminal command: {} ({} bytes)", first, result.len());
                            if matches!(first, "language" | "lang" | "theme") {
                                self.p = Preferences::load();
                            }
                        }
                    }
                } else if k == Key::Control(b'c') && t.selection.is_some() {
                    let (a, b) = t.selection.unwrap();
                    let s = t.log.copy(((frame.w - 28) / 8).max(1) as usize, a, b);
                    edit::copy(&s);
                } else if k == Key::Tab {
                    let old = VFS.lock().get_cwd();
                    let _ = VFS.lock().set_cwd(&t.cwd);
                    let last = t.line.value.clone();
                    let choices =
                        console::capture(|| line::complete("", &mut t.line.value, &mut t.line.pos));
                    let _ = VFS.lock().set_cwd(&old);
                    t.line.anchor = t.line.pos;
                    if last == t.line.value && choices.contains("\r\n") {
                        if let Some(line) = choices.split("\r\n").nth(1) {
                            t.log.push_str(line);
                            t.log.push_str("\n");
                        }
                    }
                } else if k == Key::Control(b'r') {
                    let needle = &t.line.value;
                    if let Some(n) = t.history[..t.cursor.min(t.history.len())]
                        .iter()
                        .rposition(|s| s.contains(needle))
                    {
                        t.cursor = n;
                        t.line = Edit::new(&t.history[n]);
                    }
                } else if k == Key::Control(b'l') {
                    t.log.clear();
                } else if matches!(k, Key::Up | Key::Down) {
                    if k == Key::Up {
                        t.cursor = t.cursor.saturating_sub(1);
                    } else {
                        t.cursor = (t.cursor + 1).min(t.history.len());
                    }
                    t.line = Edit::new(t.history.get(t.cursor).map_or("", String::as_str));
                } else if matches!(k, Key::PageUp | Key::PageDown) {
                    cmd = Some(Cmd::Scroll(if k == Key::PageUp { 6 } else { -6 }));
                } else {
                    t.line.key(k, shift, false);
                }
            }
            Model::Widgets {
                field,
                focus,
                level,
                tab,
                ..
            } => {
                cmd = match k {
                    Key::Tab => {
                        *focus = (*focus + if shift { 5 } else { 1 }) % 6;
                        None
                    }
                    Key::Enter | Key::Text(' ') if *focus == 1 => Some(Cmd::WidgetToggle),
                    Key::Left if *focus == 2 => Some(Cmd::Slider(level.saturating_sub(1))),
                    Key::Right if *focus == 2 => Some(Cmd::Slider((*level + 1).min(4))),
                    Key::Right if *focus == 3 => Some(Cmd::WidgetsTab((*tab + 1) % 3)),
                    Key::Left if *focus == 3 => Some(Cmd::WidgetsTab((*tab + 2) % 3)),
                    Key::Enter if *focus == 4 => Some(Cmd::Beep),
                    Key::Enter if *focus == 5 => Some(Cmd::WidgetSave),
                    _ => {
                        if *focus == 0 {
                            field.key(k, shift, false);
                        }
                        None
                    }
                };
            }
            Model::Snake(s) => match k {
                Key::Left | Key::Text('a') => s.turn((-1, 0)),
                Key::Right | Key::Text('d') => s.turn((1, 0)),
                Key::Up | Key::Text('w') => s.turn((0, -1)),
                Key::Down | Key::Text('s') => s.turn((0, 1)),
                Key::Enter | Key::Text(' ') => cmd = Some(Cmd::Play),
                _ => {}
            },
            Model::Settings => match k {
                Key::PageDown | Key::Down => cmd = Some(Cmd::Scroll(40)),
                Key::PageUp | Key::Up => cmd = Some(Cmd::Scroll(-40)),
                Key::Text('1') => cmd = Some(Cmd::Set(Setting::Language, 0)),
                Key::Text('2') => cmd = Some(Cmd::Set(Setting::Language, 1)),
                _ => {}
            },
            _ => {}
        }
        if let Some(action) = cmd {
            self.dispatch(id, action, 0, 0);
        }
    }
    fn press(&mut self, x: i32, y: i32, right: bool) {
        let hit = self.hit(x, y);
        log::trace!("Desktop pointer {x},{y} {:?}", hit.map(|h| h.cmd));
        let now = crate::boottime::elapsed_ms();
        if self.dialog.is_some() {
            if let Some(h) = hit {
                self.dispatch(h.id, h.cmd, x, y);
            }
            return;
        }
        if right {
            let id = hit.map_or(0, |h| h.id);
            if id != 0 {
                self.raise(id);
            }
            if let Some(Hit {
                cmd: Cmd::File(n), ..
            }) = hit
            {
                self.dispatch(id, Cmd::File(n), x, y);
            }
            if let Some(h) = hit {
                if matches!(
                    h.cmd,
                    Cmd::Move | Cmd::Close | Cmd::Minimize | Cmd::Maximize | Cmd::Launch(_)
                ) {
                    let id = if let Cmd::Launch(kind) = h.cmd {
                        self.windows
                            .iter()
                            .rev()
                            .find(|w| w.kind == kind)
                            .map_or(0, |w| w.id)
                    } else {
                        id
                    };
                    if id != 0 {
                        self.menu(
                            id,
                            x,
                            y,
                            vec![
                                ("Minimize", "Свернуть", Cmd::Minimize),
                                ("Maximize", "Развернуть", Cmd::Maximize),
                                ("Close", "Закрыть", Cmd::Close),
                            ],
                        );
                        return;
                    }
                }
            }
            self.context(id, x, y);
            return;
        }
        if self.popup.is_some() {
            self.popup = None;
            if let Some(h) = hit {
                if !matches!(
                    h.cmd,
                    Cmd::Calendar | Cmd::Network | Cmd::Power | Cmd::Focus | Cmd::Move
                ) {
                    self.dispatch(h.id, h.cmd, x, y);
                }
            }
            return;
        }
        if let Some(h) = hit {
            if h.id != 0 {
                self.raise(h.id);
            }
            let mut action = h.cmd;
            if self.last_click.0 == h.id
                && self.last_click.1 == action
                && now.saturating_sub(self.last_click.2) < 430
            {
                action = match action {
                    Cmd::File(_) => Cmd::Open,
                    Cmd::Move => Cmd::Maximize,
                    _ => action,
                };
            }
            self.last_click = (h.id, h.cmd, now);
            self.dispatch(h.id, action, x, y);
        }
    }
    fn motion(&mut self, x: i32, y: i32) -> bool {
        if let Some((id, cmd, sx, sy, start)) = self.drag {
            if let Some(i) = self.index(id) {
                let win = &mut self.windows[i];
                if cmd == Cmd::Move {
                    win.frame.x = (start.x + x - sx).clamp(0, (self.w - win.frame.w).max(0));
                    win.frame.y = (start.y + y - sy).clamp(31, (self.h - win.frame.h).max(31));
                } else {
                    win.frame.w = (start.w + x - sx).clamp(270, (self.w - start.x).max(270));
                    win.frame.h = (start.h + y - sy).clamp(195, (self.h - 32 - start.y).max(195));
                }
                win.restored = None;
                return true;
            }
        }
        if let Some((id, anchor)) = self.selection_drag {
            if let Some(i) = self.index(id) {
                let f = self.windows[i].frame;
                match &mut self.windows[i].model {
                    Model::Notes(n) => {
                        n.text.anchor = anchor;
                        let p = note_point(n, f, x, y);
                        n.text.place(p, true);
                        return true;
                    }
                    Model::Terminal(t) => {
                        let row = t
                            .log
                            .rows(((f.w - 28) / 8).max(1) as usize)
                            .count()
                            .saturating_sub(t.scroll + ((f.h - 90) / 19).max(1) as usize)
                            + (y - f.y - 45).max(0) as usize / 19;
                        t.selection = Some((anchor, row));
                        return true;
                    }
                    _ => {}
                }
            }
        }
        false
    }
}
fn note_point(n: &Notes, f: Rect, x: i32, y: i32) -> usize {
    let target = n.scroll + (y - f.y - 85).max(0) as usize / 20;
    let mut pos = 0;
    for (i, s) in n.text.value.split('\n').enumerate() {
        if i == target {
            let mut used = 0;
            for ch in s.chars() {
                let mut buf = [0; 4];
                let adv = width(ch.encode_utf8(&mut buf), 0);
                if used + adv / 2 > x - f.x - 51 {
                    break;
                }
                pos += ch.len_utf8();
                used += adv;
            }
            return pos;
        }
        pos += s.len() + 1;
    }
    n.text.value.len()
}
fn copy_tree(src: &str, dst: &str, depth: usize) -> Result<(), crate::fs::FileSystemError> {
    use crate::fs::FileSystemError as E;
    if depth > 24 || dst.starts_with(&format!("{src}/")) {
        return Err(E::InvalidArgument);
    }
    if VFS.lock().stat(src)?.is_dir() {
        let items = VFS.lock().list_dir(src)?;
        VFS.lock().mkdir(dst)?;
        for item in items {
            copy_tree(&join(src, &item.name), &join(dst, &item.name), depth + 1)?;
        }
    } else {
        let mut v = VFS.lock();
        if v.stat(src)?.size > 8 * 1024 * 1024 {
            return Err(E::NoSpace);
        }
        let data = v.read_file(src)?;
        v.write_file(dst, &data)?;
    }
    Ok(())
}
pub fn local_error<'a>(s: &'a str, l: Lang) -> &'a str {
    if l == Lang::En {
        return s;
    }
    match s.rsplit(": ").next().unwrap_or(s) {
        "Network request timed out" => "Истекло время ожидания сети",
        "TLS certificate verification failed" => "Проверка сертификата TLS не пройдена",
        "No supported Ethernet adapter" => "Нет поддерживаемой сетевой карты",
        "DNS lookup failed" => "Ошибка разрешения имени DNS",
        "Invalid host name" => "Недопустимое имя узла",
        "TCP connection refused" => "В соединении отказано",
        "Navigation cancelled" => "Загрузка отменена",
        "Ethernet link is down" => "Сетевой кабель отключён",
        "No DNS server configured" => "DNS-сервер не задан",
        "More RAM is needed to start the web engine"
        | "Insufficient memory for a browser window" => "Для браузера требуется больше памяти",
        "Resource exceeds the browser transfer limit" => "Превышен размер загружаемого ресурса",
        _ => "Не удалось открыть страницу",
    }
}
pub fn clipboard() -> String {
    edit::paste()
}
pub fn clip_command(args: &[&str], out: &mut String) -> i32 {
    match args {
        [] => {
            out.push_str(&clipboard());
            out.push_str("\r\n");
        }
        ["-f", p] => {
            if let Ok(b) = VFS.lock().read_file(p) {
                edit::copy(&String::from_utf8_lossy(&b));
            } else {
                return 1;
            }
        }
        ["-w", p] => {
            if VFS.lock().write_file(p, clipboard().as_bytes()).is_err() {
                return 1;
            }
        }
        _ => edit::copy(&args.join(" ")),
    }
    0
}
pub fn date_string() -> String {
    clock(
        net::unix_seconds().unwrap_or(0),
        Preferences::load().lang,
        true,
    )
}
pub fn preference_command(cmd: &str, args: &[&str], out: &mut String) -> i32 {
    let mut p = Preferences::load();
    let theme = cmd == "theme";
    match (theme, args) {
        (_, []) => {}
        (true, ["dark"]) => p.dark = true,
        (true, ["light"]) => p.dark = false,
        (false, ["en"]) => p.lang = Lang::En,
        (false, ["ru"]) => p.lang = Lang::Ru,
        _ => {
            out.push_str(if theme {
                "theme dark|light\r\n"
            } else {
                "language en|ru\r\n"
            });
            return 2;
        }
    }
    if !args.is_empty() && !p.save() {
        return 1;
    }
    out.push_str(if theme {
        if p.dark {
            "dark"
        } else {
            "light"
        }
    } else {
        p.lang.code()
    });
    out.push_str("\r\n");
    0
}
pub fn settings() {
    let _ = session(Some(App::Settings));
}
pub fn run() -> bool {
    session(None)
}
fn session(start: Option<App>) -> bool {
    let Some(fb) = console::framebuffer() else {
        return false;
    };
    if fb.width < 480 || fb.height < 350 {
        return false;
    }
    let (w, h) = (fb.width as i32, fb.height as i32);
    let saved = SESSION.lock().take();
    let mut desk = if let Some(d) = saved.filter(|d| d.w == w && d.h == h) {
        d
    } else {
        seed();
        let Some(d) = Desktop::new(w, h) else {
            return false;
        };
        d
    };
    desk.p = Preferences::load();
    desk.quit = false;
    desk.wall_dirty = true;
    if let Some(a) = start {
        desk.launch(a, false);
    }
    let mut c = Canvas::new(fb);
    let mut pointer = (w / 2, h / 2);
    let mut buttons = 0;
    let packets = mouse::enable();
    let mouse = packets.is_some();
    let mut packets = packets.unwrap_or_default();
    let tick = apic::timer_frequency() / 100;
    if tick != 0 {
        apic::set_timer_periodic(tick);
    }
    let mut dirty = true;
    let mut last = crate::boottime::elapsed_ms();
    let mut busy = 0;
    let mut input_at = None;
    let mut samples = motion::Timings::default();
    let mut last_frame = 0;
    let mut presented = None;
    loop {
        let now = crate::boottime::elapsed_ms();
        if desk.quit {
            break;
        }
        if desk.wall_dirty {
            c.set_wall(&desk.coast, desk.p.wall);
            desk.wall_dirty = false;
            dirty = true;
        }
        crate::user::jobs::poll();
        let mut web_work = false;
        for win in &mut desk.windows {
            if let Model::Browser(b) = &mut win.model {
                let size = (win.frame.w - 2, win.frame.h - 103);
                if desk.drag.is_none() && b.frame.is_some() && size != b.size {
                    b.size = size;
                    b.event(8, size.0, size.1, "");
                }
                dirty |= b.tick();
                web_work |= b.running();
            }
            if let Model::Terminal(t) = &mut win.model {
                if let Some(pid) = t.job {
                    if let Some((out, status)) = crate::user::jobs::take(pid) {
                        if !out.is_empty() {
                            t.log.push_str(&out);
                            dirty = true;
                        }
                        if let Some(code) = status {
                            t.job = None;
                            if code != 0 {
                                t.log.push_str(&format!("exit {code}\n"));
                            }
                            dirty = true;
                        }
                    }
                }
            }
        }
        net::poll();
        if now.saturating_sub(last) >= 1000 {
            let old = desk.time;
            desk.time = net::unix_seconds().unwrap_or(old + 1);
            let online = net::online();
            dirty |= online != desk.online
                || desk.p.seconds
                || old / 60 != desk.time / 60
                || desk
                    .windows
                    .iter()
                    .any(|w| w.kind == App::Monitor && !w.minimized);
            desk.online = online;
            desk.loads.rotate_left(1);
            desk.loads[59] = (busy * 100 / (now - last).max(1)).min(100) as u8;
            last = now;
            busy = 0;
            if desk.toast.as_ref().is_some_and(|(_, t)| *t <= now) {
                desk.toast = None;
                dirty = true;
            }
        }
        for win in desk.windows.iter_mut().filter(|w| !w.minimized) {
            if let Model::Snake(s) = &mut win.model {
                dirty |= s.tick(now);
            }
        }
        if now.saturating_sub(last_frame) >= 16 {
            dirty |= desk.step_animations(now);
        }
        if dirty {
            let began = crate::boottime::elapsed_ms();
            desk.paint(&mut c);
            let end = crate::boottime::elapsed_ms();
            samples.record(
                end - began,
                input_at.take().map(|t| end.saturating_sub(t)),
                desk.surface.is_some(),
            );
            c.cursor(pointer.0, pointer.1);
            let state = (desk.overview, desk.p.lang, desk.active());
            if presented != Some(state) {
                if presented.is_none() {
                    log::info!(
                        "Desktop ready: {}x{}, PS/2 mouse {}, language={}, bOS 1.0",
                        w,
                        h,
                        mouse,
                        desk.p.lang.code()
                    );
                }
                log::debug!(
                    "Desktop frame: {} {} {}",
                    if desk.overview { "overview" } else { "windows" },
                    desk.p.lang.code(),
                    desk.active()
                );
                presented = Some(state);
            }

            dirty = false;
            last_frame = now;
        }
        let old_pointer = pointer;
        let mut received = false;
        let event_at = crate::boottime::elapsed_ms();
        // Drain queues before redrawing. Serial URL entry is as fast as PS/2 typing.
        for _ in 0..128 {
            if let Some(ch) = serial::try_read_byte() {
                received = true;
                if let Some(k) = line::desktop_byte(ch, true) {
                    desk.key(k, false);
                    dirty = true;
                }
                if desk.quit {
                    break;
                }
                continue;
            }
            let Some((aux, b)) = keyboard::read_controller() else {
                break;
            };
            received = true;
            if aux {
                let Some((dx, dy, new_buttons, wheel)) = packets.push(b) else {
                    continue;
                };
                let scale = [1, 2, 3, 4][desk.p.mouse as usize];
                pointer.0 = (pointer.0 + dx * scale / 2).clamp(0, w - 1);
                pointer.1 = (pointer.1 + dy * scale / 2).clamp(0, h - 1);
                if wheel != 0 {
                    if let Some(hit) = desk.hit(pointer.0, pointer.1) {
                        if let Some(i) = desk.index(hit.id) {
                            let step = match desk.windows[i].kind {
                                App::Terminal => -3,
                                App::Files => 1,
                                App::Settings => 36,
                                _ => 3,
                            };
                            desk.dispatch(hit.id, Cmd::Scroll(wheel * step), pointer.0, pointer.1);
                            dirty = true;
                        }
                    }
                }
                if new_buttons & 3 != 0 && buttons & 3 == 0 {
                    desk.press(pointer.0, pointer.1, new_buttons & 2 != 0);
                    dirty = true;
                } else if new_buttons & 1 != 0 {
                    dirty |= desk.motion(pointer.0, pointer.1);
                } else {
                    dirty |= desk.drag.take().is_some();
                    desk.selection_drag = None;
                }
                buttons = new_buttons;
            } else if let Some(k) = line::desktop_byte(b, false) {
                if k == Key::Layout {
                    desk.russian_input = !desk.russian_input;
                    if desk.russian_input {
                        desk.notice("Russian keyboard", "Русская раскладка");
                    } else {
                        desk.notice("English keyboard", "Английская раскладка");
                    }
                } else {
                    let k = if desk.russian_input {
                        russian_key(k)
                    } else {
                        k
                    };
                    desk.key(k, line::shift_held());
                }
                dirty = true;
            }
            if desk.quit {
                break;
            }
        }
        if pointer != old_pointer {
            c.present_area(rect(old_pointer.0, old_pointer.1, 12, 18));
            c.cursor(pointer.0, pointer.1);
        }
        if received && dirty && input_at.is_none() {
            input_at = Some(event_at);
        }

        busy += crate::boottime::elapsed_ms().saturating_sub(now);
        if !received && !dirty && !web_work && tick != 0 {
            unsafe {
                core::arch::asm!("sti", "hlt", "cli", options(nomem, nostack));
            }
        }
    }
    if tick != 0 {
        apic::stop_timer();
    }
    if mouse {
        mouse::disable();
    }
    desk.popup = None;
    desk.dialog = None;
    desk.surface = None;
    desk.drag = None;
    desk.selection_drag = None;
    *SESSION.lock() = Some(desk);
    console::clear();
    true
}

pub(crate) fn russian_key(k: Key) -> Key {
    let Key::Text(ch) = k else {
        return k;
    };
    let lower = ch.to_ascii_lowercase();
    const KEYS: &str = "`qwertyuiop[]asdfghjkl;'zxcvbnm,.";
    const LETTERS: &str = "ёйцукенгшщзхъфывапролджэячсмитьбю";
    if let Some(i) = KEYS.chars().position(|c| c == lower) {
        let ru = LETTERS.chars().nth(i).unwrap();
        return Key::Text(if ch.is_ascii_uppercase() {
            ru.to_uppercase().next().unwrap_or(ru)
        } else {
            ru
        });
    }
    Key::Text(match ch {
        '{' => 'Х',
        '}' => 'Ъ',
        ':' => 'Ж',
        '"' => 'Э',
        '<' => 'Б',
        '>' => 'Ю',
        '~' => 'Ё',
        _ => ch,
    })
}
