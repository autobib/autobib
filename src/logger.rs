use crossterm::{
    style::{StyledContent, Stylize},
    tty::IsTty,
};
use log::{Level, Log, Metadata, Record};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

static HAS_ERROR: AtomicBool = AtomicBool::new(false);

pub struct Logger {}

#[inline]
fn level_as_str(level: Level) -> &'static str {
    match level {
        Level::Error => "ERROR",
        Level::Warn => "WARNING",
        Level::Info => "INFO",
        Level::Debug => "DEBUG",
        Level::Trace => "TRACE",
    }
}

#[inline]
fn level_formatter(level: Level) -> fn(&'static str) -> StyledContent<&'static str> {
    match level {
        Level::Error => Stylize::red,
        Level::Warn => Stylize::yellow,
        Level::Info => Stylize::blue,
        Level::Debug => Stylize::magenta,
        Level::Trace => Stylize::green,
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
        if record.level() == Level::Error {
            HAS_ERROR.store(true, Ordering::Relaxed);
        }

        let level = record.level();

        if io::stderr().is_tty() {
            eprintln!(
                "{} {}",
                level_formatter(level)(level_as_str(level)),
                record.args()
            );
        } else {
            eprintln!("{} {}", level_as_str(level), record.args());
        }
    }

    #[inline]
    fn flush(&self) {}
}
