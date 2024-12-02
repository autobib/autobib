use crossterm::style::{StyledContent, Stylize};
#[allow(unused_imports)]
pub use log::{debug, info, trace, warn};
use log::{Level, Log, Metadata, Record};
use std::{
    io::{self, IsTerminal},
    sync::atomic::{AtomicBool, Ordering},
};

static HAS_ERROR: AtomicBool = AtomicBool::new(false);

pub(crate) fn log_with_style<Y: FnOnce(&'static str) -> StyledContent<&'static str>>(
    style: Y,
    header: &'static str,
    args: &std::fmt::Arguments,
) {
    if io::stderr().is_terminal() {
        eprintln!("{} {args}", style(header));
    } else {
        eprintln!("{header} {args}");
    }
}

macro_rules! suggest {
    ($($arg:tt)+) => {
        if ::log::log_enabled!(::log::Level::Warn) {
            use ::crossterm::style::Stylize;
            crate::logger::log_with_style(
                |s| s.stylize().blue().bold(),
                "suggestion:",
                &format_args!($($arg)+),
            );
        }
    };
}

/// A convenience macro to combine the simultaneously error log with [`error`](log::error)  and
/// also call [`set_failed`].
macro_rules! error {
    ($($arg:tt)+) => {
        // macro must return a block since `error!` can be called in expression position
        {
            crate::logger::set_failed();
            ::log::error!($($arg)+)
        }
    };
}

pub(crate) use error;
pub(crate) use suggest;

pub struct Logger {}

pub fn set_failed() {
    HAS_ERROR.store(true, Ordering::Relaxed);
}

#[inline]
fn level_as_str(level: Level) -> &'static str {
    match level {
        Level::Error => "error:",
        Level::Warn => "warning:",
        Level::Info => "info:",
        Level::Debug => "debug:",
        Level::Trace => "trace:",
    }
}

#[inline]
fn level_formatter(level: Level) -> fn(&'static str) -> StyledContent<&'static str> {
    match level {
        Level::Error => |s| s.stylize().red().bold(),
        Level::Warn => |s| s.stylize().yellow().bold(),
        Level::Info => |s| s.stylize().blue().bold(),
        Level::Debug => |s| s.stylize().magenta().bold(),
        Level::Trace => |s| s.stylize().green().bold(),
    }
}

impl Logger {
    pub fn has_error() -> bool {
        HAS_ERROR.load(Ordering::Relaxed)
    }
}

impl Log for Logger {
    #[inline]
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    #[inline]
    fn log(&self, record: &Record) {
        let level = record.level();
        log_with_style(level_formatter(level), level_as_str(level), record.args());
    }

    #[inline]
    fn flush(&self) {}
}
