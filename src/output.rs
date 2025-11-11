macro_rules! owriteln {
    ($($arg:tt)*) => {{
        use std::io::{Write, stdout};
        let mut lock = stdout().lock();
        writeln!(lock, $($arg)*)
    }};
}

pub(crate) use owriteln;
