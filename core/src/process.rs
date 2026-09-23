//! Spawning system tools without a console window flashing up on Windows.

use std::process::{Command, Stdio};

/// A `Command` ready to run quietly: no inherited stdin, captured stdout,
/// and on Windows the CREATE_NO_WINDOW flag so `ping`/`tracert` don't open a
/// console window every second.
pub fn quiet(tool: &str) -> Command {
    let mut c = Command::new(tool);
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    // ping and traceroute print numbers in the C locale when asked; without
    // this a German system would print "12,3 ms".
    c.env("LC_ALL", "C").env("LANG", "C");
    c
}

/// Is a tool on the PATH (or at one of the usual absolute places)?
pub fn available(tool: &str) -> bool {
    if std::path::Path::new(tool).is_absolute() {
        return std::path::Path::new(tool).exists();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
    // sbin is often missing from an unprivileged PATH, and that's where traceroute lives
    for extra in ["/usr/sbin", "/sbin", "/usr/local/sbin", "/usr/bin", "/bin"] {
        dirs.push(extra.into());
    }
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{tool}.exe"), tool.to_string()]
    } else {
        vec![tool.to_string()]
    };
    dirs.iter().any(|d| names.iter().any(|n| d.join(n).is_file()))
}

/// The absolute path for a tool when it only lives in an sbin directory.
pub fn locate(tool: &str) -> String {
    for dir in ["/usr/sbin", "/sbin", "/usr/local/sbin", "/usr/bin", "/bin"] {
        let p = std::path::Path::new(dir).join(tool);
        if p.is_file() {
            return p.to_string_lossy().into_owned();
        }
    }
    tool.to_string()
}
