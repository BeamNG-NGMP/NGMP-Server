use log::{Record, Level, LevelFilter, Metadata, SetLoggerError};
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn init(level_filter: LevelFilter, enable_colors: bool) -> Result<(), SetLoggerError> {
    log::set_boxed_logger(Box::new(NGMPLogger { level_filter, enable_colors, max_record_level: AtomicUsize::new(0) }))
        .map(|()| log::set_max_level(level_filter))
}

struct NGMPLogger {
    level_filter: LevelFilter,
    enable_colors: bool,
    max_record_level: AtomicUsize,
}

impl log::Log for NGMPLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level_filter
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let level = record.level();
            let filler = if level == Level::Info || level == Level::Warn { " " } else { "" };
            let color = if self.enable_colors { match level {
                Level::Warn => "\x1b[0;33m",
                Level::Error => "\x1b[0;31m",
                Level::Debug => "\x1b[0;36m",
                _ => "\x1b[0;37m",
            } } else { "" };
            let color_reset = if self.enable_colors { "\x1b[0;37m" } else { "" };

            let target = record.target();
            let exp_len = self.max_record_level.fetch_max(target.len(), Ordering::AcqRel).max(target.len());
            let rep_len = exp_len - target.len();
            let mut target_filler = String::new();
            if rep_len > 0 {
                target_filler.push_str(&" ".repeat(rep_len));
            }

            println!("{color}[{level}]{color_reset} {filler} {target} {target_filler} {}", record.args())
        }
    }

    fn flush(&self) {}
}
