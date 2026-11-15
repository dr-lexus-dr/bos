//! Pure bitmap search shared by the frame allocator and host regression tests.
//! Set bits are free; padding bits and firmware holes must remain clear.
pub fn find_run(bitmap: &[u64], frames: usize, hint: usize, count: usize) -> Option<usize> {
    if count == 0 || count > frames {
        return None;
    }
    fn scan(bitmap: &[u64], from: usize, to: usize, count: usize) -> Option<usize> {
        let mut run = 0;
        for i in from..to {
            if bitmap
                .get(i / 64)
                .is_some_and(|word| word & (1u64 << (i % 64)) != 0)
            {
                run += 1;
                if run == count {
                    return Some(i + 1 - count);
                }
            } else {
                run = 0;
            }
        }
        None
    }
    scan(bitmap, hint.min(frames), frames, count).or_else(|| scan(bitmap, 0, frames, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn never_wraps_a_contiguous_run() {
        assert_eq!(find_run(&[0b11000011], 8, 6, 4), None);
    }
    #[test]
    fn finds_a_run_crossing_hint_but_not_memory_end() {
        assert_eq!(find_run(&[0b00111100], 8, 4, 4), Some(2));
    }
    #[test]
    fn holes_padding_and_invalid_counts() {
        assert_eq!(find_run(&[0b101010], 6, 0, 2), None);
        assert_eq!(find_run(&[u64::MAX], 3, 0, 4), None);
        assert_eq!(find_run(&[u64::MAX], 3, 0, 0), None);
        assert_eq!(find_run(&[0], 64, 0, 1), None);
        assert_eq!(find_run(&[0, 3], 66, 64, 2), Some(64));
    }
}
