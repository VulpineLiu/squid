//! Stopping one process, because the interface is allowed to ask for it.
//!
//! A file cleaner that can list what is running can also be asked to stop what
//! is running, and the ask is honest: a hung app is the reason people open a
//! dashboard at all. The two halves of the job are not equally interesting.
//!
//! The *how* is boring and short. Unix sends `SIGKILL`; Windows opens the
//! process for `PROCESS_TERMINATE` and calls `TerminateProcess`. Both are the
//! same promise — this process stops now and saves nothing — which is why the
//! interface calls it a forced stop on every platform instead of a polite quit
//! it did not perform.
//!
//! The *whether* is where the damage is, so it is answered twice. First here,
//! by rule: squid never stops itself, never stops the first process of the
//! machine, and on Windows never stops the handful of processes a boot rests
//! on. Those rules exist because the OS does not enforce all of them. An
//! unelevated process on macOS cannot reach `WindowServer` anyway, but an
//! elevated `squid` on Windows *can* open `csrss.exe` for termination, and the
//! reward for one misplaced Enter is a machine that reboots without asking. A
//! guard that costs six names is cheaper than that conversation.
//!
//! Then by the operating system, whose answer is the only one that carries any
//! authority about this user, this token, this moment. The reason it says no is
//! kept: `EPERM` and `ERROR_ACCESS_DENIED` mean "you are not that user", which
//! tells the reader to reach for an elevated terminal, while `ESRCH` means the
//! process already left on its own, which asks for nothing at all. Guessing one
//! message for both is how a tool teaches people to distrust its errors.
//!
//! Note that this module answers for a pid and nothing else. Whether the pid in
//! front of the user is still the process they read the name of is a question
//! about the sample that produced it, and it is asked where the samples live
//! (`squid_stats::Sampler::stop`).

use std::fmt;

/// The rule that stands between squid and a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffLimits {
    /// The process is this squid. Stopping it would end the answer along with
    /// the question.
    Itself,
    /// The first process of the machine: the kernel and `init`/`launchd` on
    /// unix, the System Idle Process and `System` on Windows.
    First,
    /// A Windows process the session or the boot depends on, named here so the
    /// message can say which.
    Boot(&'static str),
}

impl OffLimits {
    /// The rule in English, for logs and for the command line. The interface
    /// translates the *rule*, not this sentence, so that one rule cannot drift
    /// into two wordings.
    pub fn reason(self) -> &'static str {
        match self {
            OffLimits::Itself => "this is squid itself",
            OffLimits::First => "the first process of the machine",
            OffLimits::Boot(name) => name,
        }
    }
}

/// Why a process was not stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopError {
    /// It was already gone: either it exited between the list being drawn and
    /// the key being pressed, or the pid now belongs to something else.
    Gone { pid: u32 },
    /// A rule in this module declined, before anything was attempted.
    Refused(OffLimits),
    /// The operating system refused, in its own words.
    Denied { reason: String },
}

impl fmt::Display for StopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StopError::Gone { pid } => write!(f, "process {pid} is no longer running"),
            StopError::Refused(rule) => write!(f, "refused: {}", rule.reason()),
            StopError::Denied { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for StopError {}

/// The processes a Windows session cannot afford to lose, without their
/// extension. `svchost` is deliberately absent: it hosts services a user may
/// genuinely need to restart, and the machine survives it.
#[cfg(windows)]
const CRITICAL: [&str; 6] = ["csrss", "smss", "wininit", "services", "lsass", "winlogon"];

/// The half of the rule a pid answers on its own, before anything is read
/// about it.
///
/// Split from [`refusal`] because the two halves get asked at different moments:
/// this one is asked of the number the interface is holding, where the name
/// rule needs the process that is really behind the number. A process that is
/// off limits should be refused as that, rather than reported as missing merely
/// because no listing happened to describe it.
pub fn pid_refusal(pid: u32) -> Option<OffLimits> {
    if pid == std::process::id() {
        return Some(OffLimits::Itself);
    }
    // Unix: 0 is the kernel, 1 is init. Windows: 0 is the System Idle Process
    // and 4 is the System process, and both show up in a memory-sorted list.
    if pid <= 1 || (cfg!(windows) && pid == 4) {
        return Some(OffLimits::First);
    }
    None
}

/// Which rule declines this process, if one does.
///
/// `name` is what the process calls itself, which is the only thing the user
/// read. The comparison is on the extensionless stem because a listing may or
/// may not carry `.exe` depending on where it was taken from.
pub fn refusal(pid: u32, name: &str) -> Option<OffLimits> {
    if let Some(rule) = pid_refusal(pid) {
        return Some(rule);
    }
    // The name decides anything only on Windows, where a process is named as
    // firmly as it is numbered. Taking it anyway keeps one question — "may this
    // pid be stopped" — answerable from one place on every platform.
    let _ = &name;
    #[cfg(windows)]
    {
        let stem = name
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(name)
            .to_lowercase();
        let stem = stem.strip_suffix(".exe").unwrap_or(&stem);
        if let Some(found) = CRITICAL.iter().copied().find(|critical| *critical == stem) {
            return Some(OffLimits::Boot(found));
        }
    }
    None
}

/// Force a process to stop, now.
///
/// Only the pid is passed, so a caller that knows the name it displayed has to
/// check the name itself — which is the point: `refusal` and a fresh read of the
/// pid are the caller's job, and this function makes no promises about *which*
/// process the pid belongs to today.
pub fn force_stop(pid: u32) -> Result<(), StopError> {
    // `cfg` rather than `cfg!`, because the halves of this function are not two
    // branches of one program: on Windows there is no `unix` module to name, and
    // on macOS there is no `windows` one. An `if` would ask both to exist.
    #[cfg(unix)]
    {
        unix::force_stop(pid)
    }
    #[cfg(windows)]
    {
        windows::force_stop(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(StopError::Denied {
            reason: "stopping processes is not implemented for this platform".into(),
        })
    }
}

#[cfg(unix)]
mod unix {
    use super::StopError;

    pub fn force_stop(pid: u32) -> Result<(), StopError> {
        let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        if rc == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        // ESRCH: nothing answers to this pid any more. The user asked to stop a
        // process and there is no process; that is a report, not a failure to
        // explain away.
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Err(StopError::Gone { pid });
        }
        Err(StopError::Denied {
            reason: error.to_string(),
        })
    }
}

#[cfg(windows)]
mod windows {
    use super::StopError;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    /// The exit status a forced stop reports. `1` is what Task Manager's
    /// "End task" leaves behind, and nothing reads it: the process is simply
    /// not running afterwards.
    const FORCED: u32 = 1;

    pub fn force_stop(pid: u32) -> Result<(), StopError> {
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
        if handle.is_null() {
            return Err(denied(pid));
        }
        let ok = unsafe { TerminateProcess(handle, FORCED) };
        unsafe { CloseHandle(handle) };
        if ok == 0 {
            return Err(denied(pid));
        }
        Ok(())
    }

    /// The message the system gave, in the words it gives the user: "Access is
    /// denied." for a process owned by another account, "The parameter is
    /// incorrect." once the pid has stopped existing.
    fn denied(pid: u32) -> StopError {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error().is_some_and(|code| code == 87) {
            // ERROR_INVALID_PARAMETER: no such process.
            return StopError::Gone { pid };
        }
        StopError::Denied {
            reason: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};

    #[test]
    fn squid_never_stops_itself() {
        let here = std::process::id();
        assert_eq!(refusal(here, "squid"), Some(OffLimits::Itself));
    }

    #[test]
    fn the_first_processes_are_off_limits_everywhere() {
        assert_eq!(pid_refusal(0), Some(OffLimits::First));
        assert_eq!(pid_refusal(1), Some(OffLimits::First));
        assert_eq!(refusal(0, "kernel_task"), Some(OffLimits::First));
        assert_eq!(refusal(1, "launchd"), Some(OffLimits::First));
        // A normal process, on any platform, is fair game.
        assert_eq!(refusal(4_242, "TextEdit"), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_processes_a_boot_rests_on_are_named_not_guessed() {
        assert_eq!(refusal(4, "System"), Some(OffLimits::First));
        assert_eq!(
            refusal(520, "C:\\Windows\\System32\\csrss.exe"),
            Some(OffLimits::Boot("csrss"))
        );
        // Extensionless, upper case, and the folder it was read from must not
        // change the answer.
        assert_eq!(refusal(640, "LSASS"), Some(OffLimits::Boot("lsass")));
        // `svchost` hosts services a user may need to restart, and the machine
        // survives losing one, so the guard stays out of the way.
        assert_eq!(refusal(1234, "svchost.exe"), None);
    }

    /// The whole point of the feature: a process that is running stops.
    ///
    /// `ping` against the loopback is used because it exists on every platform
    /// this runs on, takes a minute, and losing it hurts nothing. The test
    /// waits for the exit rather than trusting the return code of the kill: on
    /// Windows the kill is synchronous but the handle we spawned through is
    /// not, and a zombie that has stopped running is still a stopped process.
    #[test]
    fn a_running_process_stops_when_it_is_asked() {
        let mut child = sleeper();
        let pid = child.id();
        assert!(
            child.try_wait().expect("child is running").is_none(),
            "the child died before being asked"
        );
        let outcome = force_stop(pid);
        if outcome.is_err() {
            let _ = child.kill();
        }
        outcome.expect("stopping a child of this test must be allowed");
        let status = child.wait().expect("the child is reaped");
        assert!(
            !status.success(),
            "a killed process reported success: {status:?}"
        );
    }

    fn sleeper() -> Child {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("ping");
            command.args(["-n", "60", "127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("60");
            command
        };
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a loopback ping / sleep should start")
    }

    /// `ESRCH` and `EPERM` ask for opposite things from the reader — one says
    /// "nothing to do", the other says "become a more privileged user" — so the
    /// two must not arrive as the same message. A pid nobody owns is the
    /// reproducible half of that pair.
    #[cfg(unix)]
    #[test]
    fn a_pid_that_is_not_there_reports_itself_as_gone() {
        let vacant = first_vacant_pid();
        assert!(refusal(vacant, "Nothing").is_none());
        let error = force_stop(vacant).expect_err("nothing owns this pid");
        assert!(
            matches!(error, StopError::Gone { pid } if pid == vacant),
            "expected Gone({vacant}), got {error:?}"
        );
    }

    /// PIDs are handed out upwards and then wrap, so just below the ceiling the
    /// range is empty. The ceiling itself is not usable: `pid_t` is an `i32` on
    /// macOS, and `kill` answers `EINVAL` rather than `ESRCH` outside its range.
    #[cfg(unix)]
    fn first_vacant_pid() -> u32 {
        let top = if cfg!(target_os = "macos") {
            999_999
        } else {
            4_194_304
        };
        (top - 64..top)
            .filter(|pid| *pid != std::process::id())
            .find(|pid| unsafe { libc::getpgid(*pid as libc::pid_t) } != 0)
            .expect("a vacant pid near the top of the range")
    }
}
