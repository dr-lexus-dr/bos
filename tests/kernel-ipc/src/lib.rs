//! Exercise the actual pipe/open-description implementation on the host.
extern crate alloc;
fn input_byte() -> Option<u8> {
    None
}
mod syscall {
    pub fn fs_err(_: ()) -> u64 {
        (-9i64) as u64
    }
    pub fn print_bytes(_: &[u8]) {}
}
mod fs {
    pub struct UnusedFs;
    pub static VFS: spin::Mutex<UnusedFs> = spin::Mutex::new(UnusedFs);
    impl UnusedFs {
        pub fn read(&mut self, _: u32, _: &mut [u8]) -> Result<usize, ()> {
            Err(())
        }
        pub fn write(&mut self, _: u32, _: &[u8]) -> Result<usize, ()> {
            Err(())
        }
        pub fn close(&mut self, _: u32) -> Result<(), ()> {
            Ok(())
        }
        pub fn seek(&mut self, _: u32, _: i64, _: u32) -> Result<u64, ()> {
            Err(())
        }
    }
}
#[path = "../../../kernel/src/user/handles.rs"]
mod handles;
#[cfg(test)]
mod tests {
    use super::handles::Pipe;
    #[test]
    fn empty_pipe_blocks_and_last_writer_closes_to_eof() {
        let (reader, writer) = Pipe::pair();
        let alias = writer.clone();
        let mut bytes = [0; 8];
        assert_eq!(reader.read(&mut bytes), Err(-11));
        drop(writer);
        assert_eq!(reader.read(&mut bytes), Err(-11));
        drop(alias);
        assert_eq!(reader.read(&mut bytes), Ok(0));
    }
    #[test]
    fn reader_clones_control_epipe() {
        let (reader, writer) = Pipe::pair();
        let alias = reader.clone();
        drop(reader);
        assert_eq!(writer.write(b"x"), Ok(1));
        drop(alias);
        assert_eq!(writer.write(b"y"), Err(-32));
    }
    #[test]
    fn full_buffer_atomicity_wraparound_and_drain_before_eof() {
        let (reader, writer) = Pipe::pair();
        assert_eq!(writer.write(&[7; 4096]), Ok(4096));
        assert_eq!(writer.write(&[8]), Err(-11));
        let mut first = [0; 100];
        assert_eq!(reader.read(&mut first), Ok(100));
        assert_eq!(first, [7; 100]);
        assert_eq!(writer.write(&[8; 101]), Err(-11));
        assert_eq!(writer.write(&[8; 100]), Ok(100));
        drop(writer);
        let mut bytes = [0; 4096];
        assert_eq!(reader.read(&mut bytes), Ok(4096));
        assert!(bytes[..3996].iter().all(|b| *b == 7));
        assert!(bytes[3996..].iter().all(|b| *b == 8));
        assert_eq!(reader.read(&mut bytes), Ok(0));
    }
    #[test]
    fn direction_and_zero_length() {
        let (reader, writer) = Pipe::pair();
        assert_eq!(reader.write(b"bad"), Err(-9));
        assert_eq!(writer.read(&mut [0; 1]), Err(-9));
        assert_eq!(reader.read(&mut []), Ok(0));
        assert_eq!(writer.write(&[]), Ok(0));
    }
    #[test]
    fn concurrent_stream_and_close() {
        let (reader, writer) = Pipe::pair();
        let producer = std::thread::spawn(move || {
            for i in 0..10000 {
                let bytes = (i as u32).to_le_bytes();
                loop {
                    match writer.write(&bytes) {
                        Ok(4) => break,
                        Err(-11) => std::thread::yield_now(),
                        other => panic!("unexpected write {other:?}"),
                    }
                }
            }
        });
        let mut all = Vec::new();
        let mut buffer = [0; 317];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => all.extend_from_slice(&buffer[..n]),
                Err(-11) => std::thread::yield_now(),
                other => panic!("unexpected read {other:?}"),
            }
        }
        producer.join().unwrap();
        assert_eq!(all.len(), 40000);
        for (index, bytes) in all.chunks_exact(4).enumerate() {
            assert_eq!(
                u32::from_le_bytes(bytes.try_into().unwrap()) as usize,
                index
            );
        }
    }
}
