//! Where process data comes from. Each OS gets one backend behind
//! [`ProcessSource`]; everything above this module is platform-neutral.

use crate::process::Snapshot;

#[cfg(target_os = "macos")]
mod macos;
mod procargs;

pub trait ProcessSource {
    fn snapshot(&mut self) -> std::io::Result<Snapshot>;
}

/// The backend for the OS we were compiled for.
pub fn system() -> Box<dyn ProcessSource> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacSource::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        compile_error!("pss currently supports macOS only; Linux support is planned");
    }
}
