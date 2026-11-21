use super::{
    canvas::{mix, rect, Canvas, Rect},
    model::App,
    preferences::Preferences,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Language,
    Theme,
    Accent,
    Wallpaper,
    Animation,
    Speed,
    Shadow,
    Glass,
    Seconds,
    Mouse,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Focus,
    Move,
    Resize,
    Close,
    Minimize,
    Maximize,
    Overview,
    OverviewTab(bool),
    Launch(App),
    Activate(u32),
    Calendar,
    Month(i32),
    Today,
    Wallpaper,
    Theme,
    WidgetSave,
    Network,
    Sound,
    Power,
    Console,
    Reboot,
    Shutdown,
    Language,
    Back,
    Forward,
    Home,
    Address,
    WebContent,
    Go,
    Reload,
    Link(usize),
    Scroll(i32),
    File(usize),
    Location(u8),
    ToggleList,
    Menu,
    NewFolder,
    NewFile,
    Rename,
    Remove,
    Copy,
    Cut,
    Paste,
    SelectAll,
    Open,
    Save,
    SaveAs,
    Editor,
    Input,
    Calc(char),
    Set(Setting, u8),
    WidgetsTab(u8),
    WidgetToggle,
    Slider(u8),
    Beep,
    Play,
    DialogOk,
    Cancel,
    NewWindow,
}
#[derive(Clone, Copy)]
pub struct Hit {
    pub id: u32,
    pub area: Rect,
    pub cmd: Cmd,
}
pub struct Ui<'a> {
    pub c: &'a mut Canvas,
    pub hits: &'a mut alloc::vec::Vec<Hit>,
    pub id: u32,
    pub p: Preferences,
    pub bounds: Rect,
}
impl Ui<'_> {
    pub fn hit(&mut self, r: Rect, cmd: Cmd) {
        let x = r.x.max(self.bounds.x);
        let y = r.y.max(self.bounds.y);
        let w = (r.x + r.w).min(self.bounds.x + self.bounds.w) - x;
        let h = (r.y + r.h).min(self.bounds.y + self.bounds.h) - y;
        if w > 0 && h > 0 {
            self.hits.push(Hit {
                id: self.id,
                area: rect(x, y, w, h),
                cmd,
            });
        }
    }
    pub fn text(&mut self, x: i32, y: i32, s: &str, font: usize) {
        self.c.label(
            x,
            y,
            s,
            self.p.ink(),
            font,
            self.bounds.x + self.bounds.w - x,
        );
    }
    pub fn dim(&mut self, x: i32, y: i32, s: &str, w: i32) {
        self.c.label(x, y, s, self.p.muted(), 0, w);
    }
    pub fn button(&mut self, r: Rect, s: &str, cmd: Cmd, primary: bool) {
        let c = if primary {
            self.p.accent()
        } else {
            self.p.bar()
        };
        self.c.round(r, 6, c);
        self.c
            .centered(r, s, if primary { 0xffffff } else { self.p.ink() }, 0);
        self.hit(r, cmd);
    }
    pub fn input(&mut self, r: Rect, text: &str, focus: bool, cmd: Cmd) {
        self.c.round(
            r,
            6,
            if focus {
                self.p.accent()
            } else {
                self.p.line()
            },
        );
        self.c.round(r.inset(1), 5, self.p.paper());
        let s = super::canvas::tail(text, 0, r.w - 17);
        self.c
            .label(r.x + 8, r.y + (r.h - 16) / 2, s, self.p.ink(), 0, r.w - 13);
        if focus {
            let x = r.x + 8 + super::canvas::width(s, 0);
            self.c.fill(rect(x, r.y + 7, 1, r.h - 14), self.p.accent());
        }
        self.hit(r, cmd);
    }
    pub fn toggle(&mut self, r: Rect, on: bool, cmd: Cmd) {
        self.toggle_at(r, if on { 256 } else { 0 }, cmd);
    }
    pub fn toggle_at(&mut self, r: Rect, t: i32, cmd: Cmd) {
        self.c.round(
            r,
            r.h / 2,
            mix(
                mix(self.p.bar(), self.p.muted(), 100),
                self.p.accent(),
                t as u32,
            ),
        );
        let d = r.h - 6;
        self.c.round(
            rect(r.x + 3 + (r.w - d - 6) * t / 256, r.y + 3, d, d),
            d / 2,
            0xffffff,
        );
        self.hit(r, cmd);
    }
}
