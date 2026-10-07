//! Resident memory of a process group, and the end of one.
//!
//! macOS has no cgroup to put a ceiling on a group of processes, and `RLIMIT_AS`/`RLIMIT_RSS` are not enforced on
//! it. What can be enforced is a watchdog: sample the resident memory of everything in the command's process group
//! on a short interval and kill the group when the sum passes the ceiling. That is a real limit with one honest
//! caveat, which callers and documentation must keep: memory is checked every [`SAMPLE_INTERVAL`], so a process can
//! overshoot by what it allocates between two samples before it is stopped.
//!
//! A process that leaves the group (`setsid`, `setpgid`) is not counted. The commands this runs (a package manager,
//! a bundler) keep their children in the group they start in.
//!
//! Sampling and killing use `ps` and `kill`, which are on every macOS; nothing here needs `unsafe`.

use std::time::Duration;
use tokio::process::Command;

/// How often the group's memory is read.
pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(250);

/// Sum the resident memory, in KiB, of the processes in group `pgid` from `ps -A -o pgid=,rss=` output.
/// Lines that are not two numbers are ignored: a process may exit between `ps` listing it and reading it.
#[must_use]
pub fn group_rss_kib(ps_output: &str, pgid: u32) -> u64 {
    ps_output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let group: u32 = fields.next()?.parse().ok()?;
            let rss: u64 = fields.next()?.parse().ok()?;
            fields.next().is_none().then_some((group, rss))
        })
        .filter(|(group, _)| *group == pgid)
        .map(|(_, rss)| rss)
        .sum()
}

/// The resident memory of group `pgid` right now, in KiB; `None` when `ps` could not be read.
pub async fn sample(pgid: u32) -> Option<u64> {
    let output = Command::new("/bin/ps").args(["-A", "-o", "pgid=,rss="]).output().await.ok()?;
    output.status.success().then(|| group_rss_kib(&String::from_utf8_lossy(&output.stdout), pgid))
}

/// Kill every process in group `pgid`. Not finding any is not a failure: the group may already be gone.
pub async fn kill_group(pgid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
}

/// The same, for a context that cannot await (a drop). The kill is short and the caller does not wait for more.
pub fn kill_group_blocking(pgid: u32) {
    let _ = std::process::Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_named_group_is_summed() {
        let ps = "  100 2048\n  100  1024\n  200 99999\n    1   512\n";
        assert_eq!(group_rss_kib(ps, 100), 3072);
        assert_eq!(group_rss_kib(ps, 200), 99999);
        assert_eq!(group_rss_kib(ps, 300), 0);
    }

    #[test]
    fn lines_that_are_not_two_numbers_are_ignored() {
        let ps = "garbage\n100\n100 abc\n100 10 extra\n\n100 7\n";
        assert_eq!(group_rss_kib(ps, 100), 7);
    }

    #[tokio::test]
    async fn this_process_group_has_a_nonzero_resident_size() {
        // `ps` reads the real table: the test process is in some group, and the sum over every group is not zero.
        let output = Command::new("/bin/ps").args(["-A", "-o", "pgid=,rss="]).output().await.unwrap();
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let first = text.split_whitespace().next().and_then(|v| v.parse::<u32>().ok()).unwrap();
        assert!(group_rss_kib(&text, first) > 0);
        assert!(sample(first).await.is_some());
    }
}
