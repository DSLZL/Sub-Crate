use crate::{Result, actions_only, require};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn scratch(label: &str) -> Result<PathBuf> {
    actions_only();
    require(label.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')), "Invalid scratch label")?;
    let parent = std::env::var_os("RUNNER_TEMP").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let path = parent.join(format!("sub-store-{label}-{}-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir(&path)?;
    Ok(path)
}

pub struct Running {
    pub child: Child,
    pub stdout: PathBuf,
    pub stderr: PathBuf,
}

impl Running {
    pub fn spawn(command: &mut Command, output: &Path, label: &str) -> Result<Self> {
        actions_only();
        std::fs::create_dir_all(output)?;
        let stdout = output.join(format!("{label}.stdout.log"));
        let stderr = output.join(format!("{label}.stderr.log"));
        let create = |path: &Path| std::fs::OpenOptions::new().write(true).create_new(true).open(path);
        command.stdout(Stdio::from(create(&stdout)?)).stderr(Stdio::from(create(&stderr)?));
        let child = command.spawn()?;
        Ok(Self { child, stdout, stderr })
    }

    pub fn wait(&mut self, timeout: Duration) -> Result<(ExitStatus, bool)> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok((status, false));
            }
            if started.elapsed() >= timeout {
                self.child.kill()?;
                return Ok((self.child.wait()?, true));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn stop(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.child.try_wait()? {
            return Ok(status);
        }
        self.child.kill()?;
        Ok(self.child.wait()?)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub struct Completed {
    pub status: ExitStatus,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Completed {
    pub fn success(&self) -> bool {
        self.status.success() && !self.timed_out
    }
}

pub fn run(command: &mut Command, output: &Path, label: &str, timeout: Duration) -> Result<Completed> {
    let argv: Vec<_> = std::iter::once(command.get_program()).chain(command.get_args()).map(|value| value.to_string_lossy().into_owned()).collect();
    let cwd = command.get_current_dir().map(|path| path.to_string_lossy().into_owned());
    let started = jiff::Timestamp::now().to_string();
    let mut process = Running::spawn(command, output, label)?;
    let (status, timed_out) = process.wait(timeout)?;
    crate::json::write_new(output.join(format!("{label}.command.json")), &json!({
        "argv":argv, "cwd":cwd, "started":started, "finished":jiff::Timestamp::now().to_string(),
        "exit_code":status.code(), "timed_out":timed_out
    }))?;
    Ok(Completed { status, timed_out, stdout:std::fs::read_to_string(&process.stdout)?, stderr:std::fs::read_to_string(&process.stderr)? })
}

pub fn checked(command: &mut Command, output: &Path, label: &str, timeout: Duration) -> Result<Completed> {
    let completed = run(command, output, label, timeout)?;
    require(completed.success(), format!("Command {label} failed; inspect preserved output"))?;
    Ok(completed)
}

pub fn provenance(output: &Path) -> Result<Value> {
    let base = "9cc6b5fac966993364ae33da8797f446bbf7f571";
    let cwd = crate::root();
    let revision = checked(Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&cwd), output, "backend-revision", Duration::from_secs(30))?;
    let patch = checked(Command::new("git").args(["diff", "--no-ext-diff", "--binary", "--no-color", "--full-index", "--unified=0", base, "--", "backend", "scripts/ip-flag-node.js"]).current_dir(&cwd), output, "backend-source", Duration::from_secs(30))?;
    let approved = std::fs::read_to_string(crate::compat().join("approved-reference-fixes.patch"))?.replace("\r\n", "\n");
    require(patch.stdout.replace("\r\n", "\n") == approved, "Backend source differs from frozen base plus approved RF01/RF02 patch")?;
    let dirty = run(Command::new("git").args(["diff", "--quiet", "HEAD", "--", "backend", "scripts/ip-flag-node.js"]).current_dir(&cwd), output, "backend-worktree", Duration::from_secs(30))?;
    require(matches!(dirty.status.code(), Some(0 | 1)), "Could not inspect reference state")?;
    Ok(json!({"backend_commit":revision.stdout.trim(), "backend_base_commit":base,
        "backend_worktree_changed":dirty.status.code()==Some(1),
        "approved_reference_patch_sha256":sha256(approved.as_bytes())}))
}

pub fn sha256(bytes: &[u8]) -> String {
    aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes).as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_evidence_cannot_be_overwritten() {
        let directory = scratch("evidence-self-check").unwrap();
        let path = directory.join("result.json");
        let failed = json!({"status":"failed"});
        crate::json::write_new(&path, &failed).unwrap();
        assert!(crate::json::write_new(&path, &json!({"status":"passed"})).is_err());
        assert_eq!(crate::json::read(path).unwrap(), failed);
    }

    #[test]
    fn hashes_use_the_standard_sha256_digest() {
        actions_only();
        assert_eq!(sha256(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
