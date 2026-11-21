use super::{
    canvas::{rect, Canvas, Rect},
    model::App,
};

const ATLAS: &[u8] = include_bytes!("assets/icons.bin");
pub fn icon(c: &mut Canvas, r: Rect, kind: Option<App>) {
    if let Some(app) = kind {
        sprite(c, r, app as usize);
        return;
    }
    // The launcher uses the same shape at every size, including the compact dock.
    let size = r.w.min(r.h);
    c.round(r, size / 5, 0x4a4b59);
    for y in 0..3 {
        for x in 0..3 {
            let d = (size / 10).max(2);
            c.round(
                rect(
                    r.x + size / 4 + x * size / 5,
                    r.y + size / 4 + y * size / 5,
                    d,
                    d,
                ),
                d / 2,
                0xf9f9fb,
            );
        }
    }
}
pub fn folder(c: &mut Canvas, r: Rect, name: &str) {
    let index = match name {
        "documents" => 10,
        "downloads" => 11,
        "projects" => 12,
        _ => 0,
    };
    sprite(c, r, index);
}
fn sprite(c: &mut Canvas, r: Rect, index: usize) {
    let n = 96;
    let record = 512 + n * n;
    let tile = &ATLAS[index * record..(index + 1) * record];
    c.indexed(r, &tile[..512], &tile[512..], n);
}
#[derive(Clone, Copy)]
pub enum Symbol {
    Home,
    Document,
    Download,
    Folder,
    Drive,
    Grid,
    List,
    Power,
    Globe,
    Link,
    Speaker,
}
pub fn symbol(c: &mut Canvas, r: Rect, shape: Symbol, color: u32) {
    let s = r.w.min(r.h);
    let weight = if s >= 26 { 2 } else { 1 };
    let mut stroke = |points: &[(i32, i32)]| {
        for p in points.windows(2) {
            c.line(
                r.x + p[0].0 * s / 20,
                r.y + p[0].1 * s / 20,
                r.x + p[1].0 * s / 20,
                r.y + p[1].1 * s / 20,
                color,
                weight,
            );
        }
    };
    match shape {
        Symbol::Home => stroke(&[
            (2, 9),
            (10, 2),
            (18, 9),
            (18, 18),
            (12, 18),
            (12, 13),
            (8, 13),
            (8, 18),
            (2, 18),
            (2, 9),
        ]),
        Symbol::Document => stroke(&[
            (4, 2),
            (12, 2),
            (17, 7),
            (17, 18),
            (4, 18),
            (4, 2),
            (12, 2),
            (12, 7),
            (17, 7),
        ]),
        Symbol::Download => {
            stroke(&[(10, 2), (10, 14), (5, 9)]);
            stroke(&[(10, 14), (15, 9)]);
            stroke(&[(3, 18), (17, 18)]);
        }
        Symbol::Folder => stroke(&[(2, 5), (8, 5), (10, 7), (18, 7), (18, 17), (2, 17), (2, 5)]),
        Symbol::Drive => {
            stroke(&[(3, 7), (17, 7), (17, 18), (3, 18), (3, 7)]);
            stroke(&[(6, 7), (6, 2), (14, 2), (14, 7)]);
        }
        Symbol::Grid => {
            for y in [3, 11] {
                for x in [3, 11] {
                    stroke(&[(x, y), (x + 5, y), (x + 5, y + 5), (x, y + 5), (x, y)]);
                }
            }
        }
        Symbol::List => {
            for y in [4, 10, 16] {
                stroke(&[(3, y), (17, y)]);
            }
        }
        Symbol::Power => {
            stroke(&[(10, 2), (10, 10)]);
            stroke(&[
                (6, 4),
                (3, 7),
                (2, 11),
                (4, 15),
                (7, 18),
                (13, 18),
                (16, 15),
                (18, 11),
                (17, 7),
                (14, 4),
            ]);
        }
        Symbol::Globe => {
            stroke(&[
                (10, 2),
                (14, 3),
                (17, 6),
                (18, 10),
                (17, 14),
                (14, 17),
                (10, 18),
                (6, 17),
                (3, 14),
                (2, 10),
                (3, 6),
                (6, 3),
                (10, 2),
            ]);
            stroke(&[
                (10, 2),
                (7, 6),
                (6, 10),
                (7, 14),
                (10, 18),
                (13, 14),
                (14, 10),
                (13, 6),
                (10, 2),
            ]);
            stroke(&[(2, 10), (18, 10)]);
        }
        Symbol::Link => {
            stroke(&[
                (11, 6),
                (14, 3),
                (17, 3),
                (19, 5),
                (19, 8),
                (14, 13),
                (11, 13),
            ]);
            stroke(&[(9, 14), (6, 17), (3, 17), (1, 15), (1, 12), (6, 7), (9, 7)]);
            stroke(&[(7, 13), (13, 7)]);
        }
        Symbol::Speaker => {
            stroke(&[(2, 8), (6, 8), (11, 3), (11, 17), (6, 12), (2, 12), (2, 8)]);
            stroke(&[(14, 7), (16, 10), (14, 13)]);
            stroke(&[(17, 4), (19, 7), (19, 13), (17, 16)]);
        }
    }
}
