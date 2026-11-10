//! bcoreutils multicall: ls cat mkdir rm cp mv ln stat wc head tail du df find chmod truncate echo uname info pwd help version...
mod fsutil;

use std::env;

fn main() {
    let a: Vec<String> = env::args().collect();
    // Support both `bcoreutils <cmd>` and symlinked `ls`/`cat`/etc. (argv[0]-based).
    let invoked = std::path::Path::new(a.first().map(|s| s.as_str()).unwrap_or(""))
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let (cmd, args) = if invoked != "bcoreutils" && invoked != "bcoreutils.exe" && !invoked.is_empty() {
        (invoked.to_string(), a[1..].to_vec())
    } else {
        (
            a.get(1).cloned().unwrap_or_else(|| "help".into()),
            a[2..].to_vec(),
        )
    };
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (out, rc) = fsutil::run(&cmd, &arg_refs, None);
    print!("{out}");
    if rc != 0 {
        std::process::exit(rc);
    }
}
