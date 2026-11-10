//! bOS config (TOML-ish key=value) parser.
use std::collections::BTreeMap;
pub fn parse(s: &str) -> BTreeMap<String,String> {
    s.lines().filter_map(|l| { let l=l.trim(); if l.is_empty()||l.starts_with('#'){None}else{l.split_once('=').map(|(k,v)|(k.trim().into(),v.trim().into()))} }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kv() {
        let m = parse("# c\nloglevel=debug\nconsole=serial\n");
        assert_eq!(m["loglevel"], "debug");
        assert_eq!(m["console"], "serial");
    }
}
