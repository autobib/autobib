use std::str::from_utf8;

use memchr::memchr_iter;

use crate::RecordId;

pub fn get_citekeys<T: Extend<RecordId>>(buffer: &[u8], container: &mut T) {
    let mut start = 0;
    container.extend(memchr_iter(b'\n', buffer).filter_map(|end| {
        let res = from_utf8(&buffer[start..end])
            .ok()
            .and_then(|s| match s.trim() {
                "" => None,
                s => Some(RecordId::from(s)),
            });
        start = end + 1;
        res
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_txt_citekeys() {
        let input = b"
a
bc\r
d e f \t

ghi
        ";
        let mut vec: Vec<RecordId> = Vec::new();
        get_citekeys(input, &mut vec);
        assert!(vec.len() == 4);
        for s in ["a", "bc", "d e f", "ghi"] {
            assert!(vec.contains(&RecordId::from(s)));
        }
    }
}
