// Run the same browser executable and platform ABI as the boot image.
use bos_web_runtime::{Library, Platform, Session};
use std::{
    fs,
    io::Read,
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
struct Host;
impl Platform for Host {
    fn monotonic_ms() -> u64 {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_millis() as u64
    }
    fn epoch_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
    fn random(b: &mut [u8]) -> bool {
        fs::File::open("/dev/urandom")
            .unwrap()
            .read_exact(b)
            .is_ok()
    }
    fn log(s: &str) {
        eprintln!("{s}")
    }
}
fn main() {
    let lib = Library::new(&fs::read(std::env::args().nth(1).expect("WASM executable")).unwrap())
        .unwrap();
    let mut s = Session::<Host>::new(&lib, 384 << 20).unwrap();
    let start = Instant::now();
    s.event(1, 480, 280, "http://test.invalid/page");
    let mut stage = 0;
    loop {
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "browser timed out"
        );
        s.step(5).unwrap();
        if let Some(r) = s.take_request() {
            let path = r.meta.lines().nth(1).unwrap();
            eprintln!("fetch {path}");
            let (typ,data)=match path{
                "http://test.invalid/page"=>("text/html","<title>Local engine</title><link rel='stylesheet' href='/site.css'><div id='box'></div><script src='/site.js'></script><form action='/search'><input name='q'><button>Go</button></form>"),
                "http://test.invalid/site.css"=>("text/css","body{margin:0}#box{width:160px;height:80px;background:#123456}"),
                "http://test.invalid/site.js"=>("text/javascript","document.getElementById('box').style.backgroundColor='#a84275'"),
                "http://test.invalid/search?q=bOS"=>("text/html","<title>Search result</title><h1>Submitted</h1>"),
                _=>panic!("unexpected request {path}"),
            };
            s.respond(Ok((
                format!("200\nContent-Type: {typ}\n"),
                data.as_bytes().to_vec(),
            )))
        }
        if let Some(e) = s.take_error() {
            panic!("{e}")
        }
        if let Some(f) = s.take_frame() {
            if !f.ready {
                continue;
            }
            eprintln!(
                "frame {}x{} {} stage {stage} after {:?}",
                f.width,
                f.height,
                f.title,
                start.elapsed()
            );
            match stage {
                0 => {
                    assert_eq!(f.pixels[10 * f.width + 10], 0xa84275);
                    s.event(7, 0, 0, "");
                }
                1 => s.event(3, 0, 0, "bOS"),
                2 => s.event(6, 0, 0, ""),
                _ => {
                    assert_eq!(f.title, "Search result");
                    break;
                }
            }
            stage += 1;
        }
        if !s.runnable() {
            std::thread::sleep(Duration::from_millis(1))
        }
    }
}
