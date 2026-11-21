use super::{
    canvas::{rect, Rect},
    edit::Edit,
    preferences::Lang,
};
use crate::{
    fs::{self, FileType, VFS},
    web::{self, engine::View, http::Url},
};
use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
pub const HOME: &str = "/users/default";
pub const COAST: &[u8] = include_bytes!("assets/coast.jpg");
pub const CLIFFS: &[u8] = include_bytes!("assets/cliffs.jpg");
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    Files,
    Browser,
    Terminal,
    Notes,
    Calculator,
    Monitor,
    Settings,
    Widgets,
    Snake,
    About,
}
pub const ALL: [App; 10] = [
    App::Files,
    App::Browser,
    App::Terminal,
    App::Notes,
    App::Calculator,
    App::Monitor,
    App::Settings,
    App::Widgets,
    App::Snake,
    App::About,
];
pub const DOCK: [App; 6] = [
    App::Files,
    App::Browser,
    App::Terminal,
    App::Notes,
    App::Monitor,
    App::Settings,
];
impl App {
    pub fn name(self, l: Lang) -> &'static str {
        let (en, ru) = match self {
            Self::Files => ("Files", "Файлы"),
            Self::Browser => ("Browser", "Браузер"),
            Self::Terminal => ("Terminal", "Терминал"),
            Self::Notes => ("Notes", "Заметки"),
            Self::Calculator => ("Calculator", "Калькулятор"),
            Self::Monitor => ("System Monitor", "Системный монитор"),
            Self::Settings => ("Settings", "Настройки"),
            Self::Widgets => ("Widgets", "Виджеты"),
            Self::Snake => ("Snake", "Змейка"),
            Self::About => ("About", "О системе"),
        };
        l.t(en, ru)
    }
}
pub struct Browser {
    pub address: Edit,
    pub editing: bool,
    pub history: Vec<String>,
    pub index: usize,
    pub link: Option<usize>,
    pub status: String,
    pub code: u16,
    pub error: bool,
    pub loading: bool,
    pub frame: Option<bos_web_runtime::Frame>,
    pub view: Option<View>,
    transfer: Option<web::network_task::Transfer>,
    request: Option<bos_web_runtime::Request>,
    pending: Option<(i32, i32, String)>,
    travel: Option<usize>,
    pub size: (i32, i32),
}
impl Browser {
    pub fn new() -> Self {
        Self {
            address: Edit::new("about:home"),
            editing: false,
            history: vec!["about:home".into()],
            index: 0,
            link: None,
            status: String::new(),
            code: 0,
            error: false,
            loading: false,
            frame: None,
            view: None,
            transfer: None,
            request: None,
            pending: None,
            travel: None,
            size: (0, 0),
        }
    }
    pub fn home(&self) -> bool {
        self.history[self.index] == "about:home" && !self.loading
    }
    pub fn focus(&mut self) {
        self.editing = true;
        self.address.select_all();
    }
    fn record(&mut self, url: &str) {
        if let Some(i) = self.travel.take() {
            self.index = i;
            self.history[i] = url.into();
        } else if self.history[self.index] != url {
            self.history.truncate(self.index + 1);
            self.history.push(url.into());
            if self.history.len() > 64 {
                self.history.remove(0);
            }
            self.index = self.history.len() - 1;
        }
    }
    pub fn go(&mut self, url: &str, travel: Option<usize>, size: (i32, i32)) {
        if self.loading || self.transfer.is_some() || self.request.is_some() {
            self.transfer = None;
            self.request = None;
            self.view = None;
            self.pending = None;
        }
        self.editing = false;
        self.link = None;
        self.error = false;
        self.travel = travel;
        if url == "about:home" {
            self.record(url);
            self.address = Edit::new(url);
            self.frame = None;
            self.transfer = None;
            self.request = None;
            self.view = None;
            self.pending = None;
            self.loading = false;
            self.status.clear();
            return;
        }
        let url = url.trim();
        let search;
        let address = if !url.contains("://")
            && (url.chars().any(char::is_whitespace)
                || (!url.contains(['.', ':', '/']) && url != "localhost"))
        {
            let mut q = String::new();
            use core::fmt::Write;
            for b in url.bytes() {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                    q.push(b as char)
                } else {
                    let _ = write!(q, "%{b:02X}");
                }
            }
            search = format!("https://html.duckduckgo.com/html/?q={q}");
            &search
        } else {
            url
        };
        match Url::parse(address) {
            Ok(u) => {
                let url = u.address();
                self.address = Edit::new(&url);
                self.loading = true;
                self.size = size;
                self.pending = Some((size.0, size.1, url));
            }
            Err(e) => self.fail(e.into()),
        }
    }
    pub fn stop(&mut self) {
        if self.loading {
            self.transfer = None;
            self.request = None;
            self.view = None;
            self.pending = None;
            self.frame = None;
            self.fail("Navigation cancelled".into());
        }
    }
    fn fail(&mut self, e: String) {
        log::info!("Browser: {e}");
        self.error = true;
        self.status = e;
        self.loading = false;
        self.travel = None;
    }
    pub fn event(&mut self, kind: u32, a: i32, b: i32, s: &str) {
        if let Some(view) = &mut self.view {
            view.event(kind, a, b, s);
        }
    }
    pub fn running(&self) -> bool {
        self.transfer.is_some()
            || self.request.is_some()
            || self.pending.is_some()
            || self.view.as_ref().is_some_and(|v| v.runnable())
    }
    pub fn tick(&mut self) -> bool {
        if let Some((w, h, url)) = self.pending.take() {
            if self.view.is_none() {
                match web::engine::start() {
                    Ok(v) => self.view = Some(v),
                    Err(e) => {
                        self.fail(e);
                        return true;
                    }
                }
            }
            self.event(1, w, h, &url);
        }
        let Some(view) = &mut self.view else {
            return false;
        };
        if self.request.is_none() {
            self.request = view.take_request();
        }
        if let Some(req) = self.request.take() {
            match web::network_task::Transfer::start(req) {
                Ok(task) => self.transfer = Some(task),
                Err(req) => self.request = Some(req),
            }
        }
        if let Some(task) = self.transfer.as_mut() {
            if let Some(response) = task.step() {
                view.respond(response);
                self.transfer = None;
            }
        }
        if !view.runnable() {
            return false;
        }
        if let Err(e) = view.step(5) {
            self.view = None;
            self.fail(e.to_string());
            return true;
        }
        if let Some(error) = view.take_error() {
            self.fail(error);
            return true;
        }
        if let Some(f) = view.take_frame() {
            if !f.ready {
                log::debug!("Browser first paint: {} {}", f.url, f.title);
                self.frame = Some(f);
                return true;
            }
            if self.loading || self.history[self.index] != f.url {
                self.record(&f.url);
                log::info!("Browser: {} {}  {}", f.status, f.url, f.title);
            }
            if !self.editing {
                self.address = Edit::new(&f.url);
            }
            self.status = f.title.clone();
            self.code = f.status;
            self.loading = false;
            self.error = false;
            self.frame = Some(f);
            return true;
        }
        false
    }
    pub fn follow(&self, i: usize) -> Option<String> {
        BOOKMARKS.get(i).map(|(_, url)| (*url).into())
    }
}
pub const BOOKMARKS: [(&str, &str); 3] = [
    ("Wikipedia", "https://www.wikipedia.org/"),
    ("Learn Rust", "https://rust-lang.org/learn/"),
    ("Example Domain", "https://example.org/"),
];
pub struct Files {
    pub path: String,
    pub items: Vec<fs::DirEntry>,
    pub selected: Option<usize>,
    pub offset: usize,
    pub list: bool,
    pub previous: Vec<String>,
    pub next: Vec<String>,
}
impl Files {
    pub fn new() -> Self {
        let mut f = Self {
            path: HOME.into(),
            items: Vec::new(),
            selected: None,
            offset: 0,
            list: false,
            previous: Vec::new(),
            next: Vec::new(),
        };
        f.refresh();
        f
    }
    pub fn refresh(&mut self) {
        self.items = VFS.lock().list_dir(&self.path).unwrap_or_default();
        self.items.retain(|e| !e.name.starts_with('.'));
        self.items.sort_by(|a, b| {
            let rank = |e: &fs::DirEntry| {
                if self.path == HOME {
                    match e.name.as_str() {
                        "documents" => 0,
                        "downloads" => 1,
                        "projects" => 2,
                        "Coast.jpg" => 3,
                        "Cliffs.jpg" => 4,
                        "Plan.txt" => 5,
                        _ => 6,
                    }
                } else if e.file_type == FileType::Directory {
                    0
                } else {
                    1
                }
            };
            rank(a).cmp(&rank(b)).then_with(|| a.name.cmp(&b.name))
        });
        self.selected = self.selected.filter(|i| *i < self.items.len());
        self.offset = self.offset.min(self.items.len().saturating_sub(1));
    }
    pub fn enter(&mut self, p: &str, remember: bool) {
        if VFS.lock().stat(p).is_ok_and(|s| s.is_dir()) {
            if remember && p != self.path {
                if self.previous.len() == 32 {
                    self.previous.remove(0);
                }
                self.previous.push(self.path.clone());
                self.next.clear();
            }
            self.path = p.into();
            self.selected = None;
            self.offset = 0;
            self.refresh();
        }
    }
    pub fn selected_path(&self) -> Option<String> {
        self.items
            .get(self.selected?)
            .map(|e| join(&self.path, &e.name))
    }
}
pub fn join(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}
pub fn filename<'a>(s: &'a str, l: Lang) -> &'a str {
    match s {
        "documents" => l.t("Documents", "Документы"),
        "downloads" => l.t("Downloads", "Загрузки"),
        "projects" => l.t("Projects", "Проекты"),
        "pictures" => l.t("Pictures", "Изображения"),
        "music" => l.t("Music", "Музыка"),
        "videos" => l.t("Videos", "Видео"),
        _ => s,
    }
}
pub fn seed() {
    let mut v = VFS.lock();
    let mark = join(HOME, ".desktop-created");
    if v.stat(&mark).is_ok() {
        return;
    }
    for d in ["documents", "downloads", "projects"] {
        let _ = v.mkdir_p(&join(HOME, d));
    }
    for(n,bytes)in [("Coast.jpg",COAST),("Cliffs.jpg",CLIFFS),("Plan.txt",b"Project plan\n\nExplore the desktop.\nOpen Settings to change the language or appearance.\n" as &[u8])]{let p=join(HOME,n);if v.stat(&p).is_err(){if v.write_file(&p,bytes).is_err(){return;}}}
    let _ = v.write_file(&mark, b"1");
}
pub struct Snake {
    pub body: Vec<(i32, i32)>,
    pub dir: (i32, i32),
    pub next: (i32, i32),
    pub food: (i32, i32),
    pub running: bool,
    pub over: bool,
    pub stamp: u64,
}
impl Snake {
    pub fn new() -> Self {
        Self {
            body: vec![(9, 7), (8, 7), (7, 7)],
            dir: (1, 0),
            next: (1, 0),
            food: (15, 7),
            running: false,
            over: false,
            stamp: 0,
        }
    }
    pub fn turn(&mut self, d: (i32, i32)) {
        if d != (-self.dir.0, -self.dir.1) {
            self.next = d
        }
    }
    pub fn tick(&mut self, now: u64) -> bool {
        if !self.running || now.saturating_sub(self.stamp) < 160 {
            return false;
        }
        self.stamp = now;
        self.dir = self.next;
        let n = (self.body[0].0 + self.dir.0, self.body[0].1 + self.dir.1);
        let grows = n == self.food;
        if !(0..24).contains(&n.0)
            || !(0..16).contains(&n.1)
            || self.body[..self.body.len() - usize::from(!grows)].contains(&n)
        {
            self.running = false;
            self.over = true;
            return true;
        }
        self.body.insert(0, n);
        if grows {
            let start = (now as usize) % 384;
            let Some(p) = (0..384)
                .map(|i| ((i + start) % 384) as i32)
                .map(|n| (n % 24, n / 24))
                .find(|p| !self.body.contains(p))
            else {
                self.running = false;
                self.over = true;
                return true;
            };
            self.food = p;
        } else {
            self.body.pop();
        }
        true
    }
}
pub struct Notes {
    pub text: Edit,
    pub path: String,
    pub dirty: bool,
    pub scroll: usize,
}
pub struct Terminal {
    pub job: Option<u64>,
    pub line: Edit,
    pub log: super::terminal::Screen,
    pub cwd: String,
    pub history: Vec<String>,
    pub cursor: usize,
    pub scroll: usize,
    pub selection: Option<(usize, usize)>,
}
pub enum Model {
    Image(String, super::canvas::Picture),
    Files(Files),
    Browser(Browser),
    Terminal(Terminal),
    Notes(Notes),
    Calculator(Edit),
    Monitor,
    Settings,
    Widgets {
        field: Edit,
        checked: bool,
        focus: u8,
        switch: super::motion::Slide,
        segment: super::motion::Slide,
        level: u8,
        tab: u8,
    },
    Snake(Snake),
    About,
}
#[derive(Clone, Copy, PartialEq)]
pub enum Transition {
    Open,
    Close,
    Minimize,
    Restore,
}
pub struct Window {
    pub id: u32,
    pub kind: App,
    pub frame: Rect,
    pub restored: Option<Rect>,
    pub minimized: bool,
    pub opened: u64,
    pub transition: Option<Transition>,
    pub scroll: i32,
    pub model: Model,
}
impl Window {
    pub fn new(id: u32, app: App, w: i32, h: i32) -> Self {
        let frame = match app {
            App::Files => rect(w / 20, h * 16 / 100, w * 53 / 100, h * 57 / 100),
            App::Browser => rect(w * 53 / 100, h * 31 / 100, w * 43 / 100, h * 49 / 100),
            _ => {
                let ww = (w * 58 / 100).clamp(460, 740).min(w - 18);
                let hh = (h * 67 / 100).clamp(340, 620).min(h - 90);
                rect((w - ww) / 2, (h - hh) / 2, ww, hh)
            }
        };
        let model = match app {
            App::Files => Model::Files(Files::new()),
            App::Browser => Model::Browser(Browser::new()),
            App::Terminal => Model::Terminal(Terminal {
                job: None,
                line: Edit::default(),
                log: super::terminal::Screen::new(),
                cwd: HOME.into(),
                history: Vec::new(),
                cursor: 0,
                scroll: 0,
                selection: None,
            }),
            App::Notes => Model::Notes(Notes {
                text: Edit::default(),
                path: join(HOME, "documents/Note.txt"),
                dirty: false,
                scroll: 0,
            }),
            App::Calculator => Model::Calculator(Edit::default()),
            App::Settings => Model::Settings,
            App::Monitor => Model::Monitor,
            App::Widgets => Model::Widgets {
                field: Edit::default(),
                checked: false,
                focus: 0,
                switch: Default::default(),
                segment: Default::default(),
                level: 2,
                tab: 0,
            },
            App::Snake => Model::Snake(Snake::new()),
            App::About => Model::About,
        };
        Self {
            id,
            kind: app,
            frame,
            restored: None,
            minimized: false,
            scroll: 0,
            opened: u64::MAX,
            transition: Some(Transition::Open),
            model,
        }
    }
}
pub fn date(mut days: u64) -> (i32, usize, usize) {
    let mut y = 1970;
    while days >= days_in_year(y) {
        days -= days_in_year(y);
        y += 1;
    }
    let mut m = 0;
    while days >= month_days(y, m) as u64 {
        days -= month_days(y, m) as u64;
        m += 1;
    }
    (y, m, days as usize + 1)
}
fn days_in_year(y: i32) -> u64 {
    if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
        366
    } else {
        365
    }
}
pub fn month_days(y: i32, m: usize) -> usize {
    [
        31,
        if days_in_year(y) == 366 { 29 } else { 28 },
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
    ][m]
}
pub fn month(m: usize, l: Lang) -> &'static str {
    if l == Lang::En {
        [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ][m]
    } else {
        [
            "Январь",
            "Февраль",
            "Март",
            "Апрель",
            "Май",
            "Июнь",
            "Июль",
            "Август",
            "Сентябрь",
            "Октябрь",
            "Ноябрь",
            "Декабрь",
        ][m]
    }
}
pub fn clock(ts: u64, l: Lang, seconds: bool) -> String {
    let (_, m, d) = date(ts / 86400);
    let day = (ts / 86400 + 3) as usize % 7;
    let week = if l == Lang::En {
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][day]
    } else {
        ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"][day]
    };
    let mo = if l == Lang::En {
        [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ][m]
    } else {
        [
            "янв", "фев", "мар", "апр", "май", "июн", "июл", "авг", "сен", "окт", "ноя", "дек",
        ][m]
    };
    let mut text = format!(
        "{week}, {d} {mo}   {:02}:{:02}",
        ts / 3600 % 24,
        ts / 60 % 60
    );
    if seconds {
        text.push_str(&format!(":{:02}", ts % 60));
    }
    text
}

/// Gregorian weekday of the first day, Monday = 0.
pub fn weekday_first(year: i32, month: usize) -> i32 {
    let y = year - i32::from(month < 2);
    let m = (month + 10) % 12;
    let d = 365 * y + y / 4 - y / 100 + y / 400 + (153 * m as i32 + 2) / 5;
    (d + 2).rem_euclid(7)
}
