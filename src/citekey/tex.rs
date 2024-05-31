use std::collections::HashSet;

use memchr::{memchr, memchr2, memchr3};

// Move forward until all comments and whitespace are consumed.
fn comment_and_ws(buffer: &[u8], mut pos: usize) -> usize {
    while pos < buffer.len() {
        match buffer[pos] {
            b'%' => {
                if let Some(skip) = memchr(b'\n', &buffer[pos..]) {
                    pos += skip + 1;
                }
            }
            s if s.is_ascii_whitespace() => pos += 1,
            _ => return pos,
        }
    }
    buffer.len()
}

/// Try to parse a macro `\<name>` where `<name>` is ascii lowercase.
fn ascii_macro(buffer: &[u8], mut pos: usize) -> (Option<&str>, usize) {
    // check the first char
    if buffer[pos] == b'\\' {
        pos += 1
    } else {
        return (None, pos);
    }

    // take characters as long as they are ascii lowercase
    let mut end = pos;
    while end < buffer.len() && buffer[end].is_ascii_lowercase() {
        end += 1;
    }

    // found some: convert to string (safe since chars are ascii lowercase)
    if pos < end {
        (
            Some(unsafe { std::str::from_utf8_unchecked(&buffer[pos..end]) }),
            end,
        )
    } else {
        (None, pos)
    }
}

/// Skip an optional argument to a macro.
fn macro_opt_argument(buffer: &[u8], mut pos: usize) -> usize {
    if let Some(b'[') = buffer.get(pos) {
        pos += 1;
        loop {
            if let Some(offset) = memchr2(b']', b'%', &buffer[pos..]) {
                pos = pos + offset;
                match &buffer[pos] {
                    b']' => break pos + 1,
                    b'%' => pos = comment_and_ws(buffer, pos),
                    _ => unreachable!(),
                }
                // if let Some(offset) = memchr(b']', &buffer[pos + 1..]) {
            } else {
                break pos;
            }
        }
    } else {
        pos
    }
}

/// Return the argument of a macro, skipping any optional arguments and pruning comments and some
/// whitespace.
fn macro_argument(buffer: &[u8], mut pos: usize) -> (Option<String>, usize) {
    pos = macro_opt_argument(buffer, pos);
    pos = comment_and_ws(buffer, pos);
    if let Some(b'{') = buffer.get(pos) {
        pos += 1;
        let mut start = pos;
        let mut contents: Vec<u8> = Vec::new();
        loop {
            if let Some(offset) = memchr3(b'{', b'}', b'%', &buffer[pos..]) {
                pos = pos + offset;
                match &buffer[pos] {
                    b'{' => {
                        break (None, pos + 1);
                    }
                    b'}' => {
                        contents.extend(&buffer[start..pos]);
                        break (String::from_utf8(contents).ok(), pos + 1);
                    }
                    b'%' => {
                        contents.extend(&buffer[start..pos]);
                        pos = comment_and_ws(buffer, pos);
                        start = pos;
                    }
                    _ => unreachable!(),
                }
            } else {
                break (None, pos);
            }
        }
    } else {
        (None, pos)
    }
}

/// parse the citation contents and append new keys to `keys`.
fn parse_cite_contents<'a>(contents: &str, keys: &mut HashSet<String>) {
    keys.extend(
        contents
            .split(',')
            .map(|k| k.trim())
            .filter(|k| *k != "*")
            .map(|k| k.into()),
    )
}

/// get all citation keys in the buffer
/// Citekeys appear in the buffer in the form `\...cite{key1, key2}`, when they are not embedded
/// inside comments.
pub fn get_citekeys(buffer: &[u8], keys: &mut HashSet<String>) -> () {
    let mut pos: usize = 0;

    while pos < buffer.len() {
        match buffer[pos] {
            b'%' => match memchr(b'\n', &buffer[pos..]) {
                Some(skip) => {
                    pos += skip + 1;
                }
                None => todo!(),
            },
            b'\\' => {
                let (opt_cmd, next) = ascii_macro(buffer, pos);
                pos = next;
                if let Some(cmd) = opt_cmd {
                    if cmd.ends_with("cite") {
                        let (opt_contents, next) = macro_argument(buffer, pos);
                        pos = next;
                        if let Some(contents) = opt_contents {
                            parse_cite_contents(&contents, keys);
                        }
                    }
                }
            }
            _ => {
                pos += 1;
            }
        }
    }
}
