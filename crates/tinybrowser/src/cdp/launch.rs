//! Finding a Chrome on this host, and starting one.
//!
//! # Why the module launches at all
//!
//! A host that already runs a browser should point the module at it with
//! [`tinybrowser_bus::SessionOptions::endpoint`] — that is the better
//! arrangement, and it is the one a sandbox or a container deployment will use.
//! Launching exists for the ordinary case where nobody has arranged anything and
//! an agent needs a browser now.
//!
//! # Discovery is a fixed list, not a search
//!
//! [`find_executable`] checks an environment override, then a short list of
//! conventional paths. It does not scan, and it does not consult a package
//! manager. A module that goes looking for executables to run is a module whose
//! behaviour depends on what else is installed on the host, and the failure mode
//! of guessing wrong — launching some unrelated binary that happens to sit at a
//! plausible path — is worse than reporting that no browser was found.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::error::{Error, Result};

/// Environment variable supplying extra launch flags for every session.
///
/// Whitespace-separated, appended after the session's own
/// [`tinybrowser_bus::SessionOptions::args`]. It exists because the flags a
/// browser needs are usually a property of the *host*, not of the caller: a
/// container without the right capabilities needs `--no-sandbox` for every
/// session anybody opens, and plumbing that through a host's configuration into
/// every call site is a lot of machinery to say one thing about the machine.
///
/// It cannot narrow anything the module does — flags only ever loosen a
/// browser — so an operator setting it is making a choice about their own host,
/// which is the person who should be making it.
pub(crate) const ARGS_ENV: &str = "TINYBROWSER_CHROME_ARGS";

/// Environment variable naming the browser to launch.
///
/// The escape hatch for a host whose Chrome is somewhere this module would never
/// guess — a Nix store path, a Chrome for Testing download, a container image
/// that puts it under `/opt`.
pub(crate) const EXECUTABLE_ENV: &str = "TINYBROWSER_CHROME";

/// How long to wait for a launched browser to print its debugger URL.
///
/// A cold Chrome on a loaded machine takes a few seconds; one that is going to
/// fail usually does so immediately. Twenty seconds is generous for the first
/// and irrelevant to the second.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

/// The paths a Chrome or Chromium is conventionally installed at.
///
/// Ordered by how likely each is to *work unattended*, which is not the same as
/// how likely it is to be somebody's preferred browser.
///
/// On Linux that means an ordinary packaged Chrome or Chromium first and
/// anything snap-packaged last. `/usr/bin/chromium` on Ubuntu is usually a shim
/// that execs the snap, and a confined snap is a poor automation target: it
/// cannot reach a profile directory under `/tmp`, and its first launch can spend
/// longer setting itself up than a browser is given to report its debugging
/// socket. The failure that produces — a process that starts, says nothing, and
/// is eventually timed out — looks like a bug in this module rather than a
/// packaging decision, so the shim is tried only when nothing else is present.
///
/// A host that cares which browser runs should not be relying on this list at
/// all: [`EXECUTABLE_ENV`] names one, and an endpoint avoids launching entirely.
#[cfg(target_os = "linux")]
const CANDIDATES: &[&str] = &[
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/opt/google/chrome/chrome",
    "/usr/bin/chromium-browser",
    "/usr/bin/brave-browser",
    // Snap-packaged, and last for the reasons above.
    "/usr/bin/chromium",
    "/snap/bin/chromium",
];

#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &[
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
];

#[cfg(target_os = "windows")]
const CANDIDATES: &[&str] = &[
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files\Chromium\Application\chrome.exe",
];

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
const CANDIDATES: &[&str] = &[];

/// The flags every launch carries, and why each one is here.
///
/// This list is short on purpose. Every flag is a behaviour difference between
/// what the module sees and what a person would see, and a module whose browser
/// no longer resembles a browser stops being useful for checking what a page
/// actually does.
const BASE_ARGS: &[&str] = &[
    // Port 0 asks the operating system for a free port. A fixed port makes two
    // sessions on one host collide, and makes the collision look like a browser
    // that will not start.
    "--remote-debugging-port=0",
    // Without this, the first run shows a profile picker and a welcome tab, and
    // the page under test is not the active one.
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    // A hidden window still schedules its timers at full rate, which is what a
    // page waiting on an animation or a poll needs.
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    // The popup that offers to save a password steals focus and covers the page.
    "--password-store=basic",
    "--use-mock-keychain",
];

/// A browser this module started.
///
/// Holding the [`Child`] is what makes the process ours to end: a launched
/// browser that outlives its session is a headless Chrome nobody knows about,
/// holding a profile directory open, until the host reboots.
#[derive(Debug)]
pub(crate) struct LaunchedBrowser {
    /// The browser WebSocket URL it printed on startup.
    pub(crate) websocket_url: String,
    child: Child,
    profile: Option<PathBuf>,
}

impl LaunchedBrowser {
    /// Ends the browser and removes the profile directory this module created.
    ///
    /// Errors are deliberately swallowed: this runs on the way out, and a
    /// browser that has already exited or a directory already gone are both the
    /// outcome being asked for.
    pub(crate) async fn shutdown(mut self) {
        let _ = self.child.kill().await;
        if let Some(profile) = self.profile.take() {
            let _ = tokio::fs::remove_dir_all(profile).await;
        }
    }
}

/// The browser this host should launch.
///
/// # Errors
///
/// [`Error::BrowserUnavailable`] when neither the override nor any conventional
/// path names an existing file.
pub(crate) fn find_executable(configured: Option<&str>) -> Result<PathBuf> {
    resolve_executable(
        configured,
        std::env::var(EXECUTABLE_ENV).ok().as_deref(),
        &|path: &std::path::Path| path.exists(),
    )
}

/// The decision behind [`find_executable`], with its two inputs and its one
/// filesystem question passed in.
///
/// Split out so the precedence can be tested exhaustively: a test that set
/// `TINYBROWSER_CHROME` to exercise the middle branch would race every other
/// test in the process, and the branch that matters most — a configured path
/// that does not exist must *fail* rather than fall through to whatever browser
/// happens to be installed — is unreachable on a machine that has one.
pub(crate) fn resolve_executable(
    configured: Option<&str>,
    from_env: Option<&str>,
    exists: &dyn Fn(&std::path::Path) -> bool,
) -> Result<PathBuf> {
    // The two overrides are checked in order and neither falls through. Falling
    // back on a typo would hide a host's misconfiguration behind a browser that
    // works, which is the worst outcome for a setting whose whole purpose is to
    // pin which binary runs.
    for (source, candidate) in [
        ("configured browser", configured),
        (EXECUTABLE_ENV, from_env),
    ] {
        let Some(candidate) = candidate else { continue };
        let path = PathBuf::from(candidate);

        return if exists(&path) {
            Ok(path)
        } else {
            Err(Error::browser_unavailable(format!(
                "{source} {} does not exist",
                path.display()
            )))
        };
    }

    CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|path| exists(path))
        .ok_or_else(|| {
            Error::browser_unavailable(format!(
                "no chrome or chromium found on this host; install one, or set {EXECUTABLE_ENV} \
                 to its path, or give the session an endpoint to attach to instead"
            ))
        })
}

/// Starts a browser and waits for it to announce its debugger socket.
///
/// `profile` is the user data directory. When it is `None` a fresh temporary
/// one is created and removed on [`LaunchedBrowser::shutdown`], so one session's
/// cookies and logins never reach the next.
///
/// # Errors
///
/// [`Error::BrowserUnavailable`] when the process cannot be spawned, exits
/// during startup, or does not print a debugger URL within the startup deadline.
pub(crate) async fn launch(
    executable: &std::path::Path,
    headless: bool,
    profile: Option<&str>,
    extra_args: &[String],
) -> Result<LaunchedBrowser> {
    launch_within(executable, headless, profile, extra_args, STARTUP_TIMEOUT).await
}

/// [`launch`] with the startup deadline supplied.
///
/// Split out for the tests: the failure worth checking is a browser that starts
/// and then never says anything, and waiting the real twenty seconds to check it
/// would put a twenty-second pause in the suite.
pub(crate) async fn launch_within(
    executable: &std::path::Path,
    headless: bool,
    profile: Option<&str>,
    extra_args: &[String],
    startup: Duration,
) -> Result<LaunchedBrowser> {
    let (profile_dir, owned) = match profile {
        Some(path) => (PathBuf::from(path), false),
        None => (profile_in(&std::env::temp_dir())?, true),
    };

    let mut command = Command::new(executable);
    command
        .args(BASE_ARGS)
        .arg(format!("--user-data-dir={}", profile_dir.display()));

    if headless {
        // The "new" headless mode is the same renderer as headed Chrome. The
        // old one was a separate implementation that quietly differed on
        // exactly the things a browser is used to check.
        command.arg("--headless=new");
        // Native scrollbars are drawn into headless screenshots and change the
        // page width by a scrollbar's worth between one run and the next.
        command.arg("--hide-scrollbars");
    }

    command
        .args(extra_args)
        .args(environment_args())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        // Chrome prints its debugger URL to stderr, and this is the only
        // discovery path that works with `--remote-debugging-port=0`: the port
        // is not known to anyone until the browser has chosen it.
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(|error| {
        Error::browser_unavailable(format!("launching {}: {error}", executable.display()))
    })?;

    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill().await;
        return Err(Error::browser_unavailable(
            "launched browser exposed no stderr to read its debugger url from".to_string(),
        ));
    };

    let websocket_url = match tokio::time::timeout(startup, read_websocket_url(stderr)).await {
        Ok(Ok(url)) => url,
        Ok(Err(error)) => {
            let _ = child.kill().await;
            return Err(error);
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err(Error::browser_unavailable(format!(
                "{} did not report a devtools url within {}s",
                executable.display(),
                startup.as_secs()
            )));
        }
    };

    Ok(LaunchedBrowser {
        websocket_url,
        child,
        profile: owned.then_some(profile_dir),
    })
}

/// Reads Chrome's startup banner until it names the debugger socket.
///
/// When the browser dies instead, the banner is the only account of why, so the
/// first few lines of it are kept and handed to [`diagnose`] rather than
/// discarded in favour of "it did not start".
async fn read_websocket_url(stderr: tokio::process::ChildStderr) -> Result<String> {
    const MARKER: &str = "DevTools listening on ";
    /// Enough to hold the fatal line and its context, and few enough that a
    /// browser logging steadily cannot grow this without bound.
    const KEPT_LINES: usize = 12;

    let mut lines = BufReader::new(stderr).lines();
    let mut banner: Vec<String> = Vec::with_capacity(KEPT_LINES);

    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(url) = line.split_once(MARKER) {
            return Ok(url.1.trim().to_string());
        }
        if banner.len() < KEPT_LINES {
            banner.push(line);
        }
    }

    Err(Error::browser_unavailable(diagnose(&banner)))
}

/// Turns what the browser printed on its way out into something actionable.
///
/// The sandbox case is called out by name because it is the one an operator
/// will actually hit — a container without `SYS_ADMIN`, or an Ubuntu 23.10 or
/// later host, where unprivileged user namespaces are restricted by `AppArmor` —
/// and because the raw stack trace Chrome prints buries the one line that says
/// what to do. The remedy is deliberately *reported* rather than applied:
/// `--no-sandbox` removes the renderer's isolation from the pages it visits,
/// which is not a default this module gets to choose on a host's behalf.
pub(crate) fn diagnose(banner: &[String]) -> String {
    if banner.iter().any(|line| line.contains("No usable sandbox")) {
        return concat!(
            "browser could not start because this host has no usable sandbox: unprivileged ",
            "user namespaces are restricted, which is the default on Ubuntu 23.10 and later ",
            "and in containers without the right capabilities. Either allow them for this ",
            "binary, or accept the reduced isolation by adding --no-sandbox to the session's ",
            "args",
        )
        .to_string();
    }

    let printed: Vec<&str> = banner
        .iter()
        .map(String::as_str)
        .filter(|line| !line.trim().is_empty())
        .take(3)
        .collect();

    if printed.is_empty() {
        "browser exited during startup without reporting a devtools url".to_string()
    } else {
        format!(
            "browser exited during startup without reporting a devtools url: {}",
            printed.join(" | ")
        )
    }
}

/// The extra flags [`ARGS_ENV`] supplies, if any.
pub(crate) fn environment_args() -> Vec<String> {
    std::env::var(ARGS_ENV)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// A private profile directory for one launched browser, created under `base`.
///
/// `base` is a parameter rather than a call to `std::env::temp_dir` inside so a
/// test can hand it somewhere unwritable: a module that reports "could not
/// create a profile directory" instead of launching is a module an operator can
/// diagnose, and that message is only reachable when the creation fails.
pub(crate) fn profile_in(base: &std::path::Path) -> Result<PathBuf> {
    let path = base.join(format!("tinybrowser-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).map_err(|error| {
        Error::browser_unavailable(format!(
            "creating profile directory {}: {error}",
            path.display()
        ))
    })?;
    Ok(path)
}
