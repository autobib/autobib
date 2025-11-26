use std::io::{self, IsTerminal, StdoutLock, Write};

macro_rules! owriteln {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let mut lock = $crate::output::stdout_lock_wrap();
        writeln!(lock, $($arg)*)
    }};
}

pub(crate) use owriteln;

// The following section is copied with modification from the `pipecheck` crate by Alex Hamlin
// under the MIT License (included below).

/*

Copyright (c) 2025 Alex Hamlin

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

*/

// *** START OF LICENSED SECTION ***

/// A stdout writer that silently terminates the program on broken pipe errors.
///
/// When any call to its underlying writer returns a [`BrokenPipe`](io::ErrorKind::BrokenPipe)
/// error, a [`StdoutWriter`] terminates the current process with a SIGPIPE signal, or exits with code 1
/// on non-Unix systems.
pub(crate) struct StdoutWriter {
    sol: StdoutLock<'static>,
}

pub(crate) fn stdout_lock_wrap() -> StdoutWriter {
    StdoutWriter {
        sol: std::io::stdout().lock(),
    }
}

impl Write for StdoutWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        check_for_broken_pipe(self.sol.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        check_for_broken_pipe(self.sol.flush())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        check_for_broken_pipe(self.sol.write_all(buf))
    }

    fn write_fmt(&mut self, fmt: std::fmt::Arguments<'_>) -> io::Result<()> {
        check_for_broken_pipe(self.sol.write_fmt(fmt))
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        check_for_broken_pipe(self.sol.write_vectored(bufs))
    }
}

impl StdoutWriter {
    pub fn supports_styled_output(&self) -> bool {
        // FIXME: maybe this isn't the best?
        self.sol.is_terminal()
    }
}

fn check_for_broken_pipe<T>(result: io::Result<T>) -> io::Result<T> {
    match result {
        Err(ref err) if err.kind() == io::ErrorKind::BrokenPipe => exit_for_broken_pipe(),
        result => result,
    }
}

fn exit_for_broken_pipe() -> ! {
    #[cfg(unix)]
    // SAFETY: These are FFI calls to libc, which we assume is implemented
    // correctly. Because everything in the block comes from libc, there are no
    // Rust invariants to violate.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        libc::raise(libc::SIGPIPE);
    }

    // Non-Unix systems fall back to a normal silent exit (and Unix systems
    // should not reach this line).
    std::process::exit(1);
}

// *** END OF LICENSED SECTION ***
