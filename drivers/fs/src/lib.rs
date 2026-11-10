//! FS driver registry (host build).
pub fn list() -> Vec<&'static str> { vec!["bFS", "fat32-ro", "ramfs"] }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn has_bfs() { assert!(list().contains(&"bFS")); }
}
