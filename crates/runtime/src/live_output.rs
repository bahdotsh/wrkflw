//! Step output as it is printed, for anyone watching a run.
//!
//! Runtimes still return a step's whole output when it ends. While a tap is
//! active they also pass each chunk to it as it arrives, so a long build
//! shows progress instead of nothing. The executor installs a tap around
//! each step; outside one, runtimes collect output as before.
//!
//! The tap is a task-local, so it follows the step through every await on
//! its task. Code that reads output on another thread takes the tap with
//! [`current`] first and passes it along.

use std::future::Future;
use std::sync::Arc;
use tokio::io::AsyncReadExt;

/// Receives raw output chunks, stdout and stderr alike. It may be called
/// from any thread and must not block for long.
#[derive(Clone)]
pub struct OutputTap(Arc<dyn Fn(&str) + Send + Sync>);

impl OutputTap {
    pub fn new(f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        OutputTap(Arc::new(f))
    }

    pub fn send(&self, chunk: &str) {
        if !chunk.is_empty() {
            (self.0)(chunk);
        }
    }
}

tokio::task_local! {
    static TAP: OutputTap;
}

/// Run `future` with `tap` receiving the output of whatever it runs.
pub async fn with_output_tap<F: Future>(tap: OutputTap, future: F) -> F::Output {
    TAP.scope(tap, future).await
}

/// The tap active on this task, if any.
pub fn current() -> Option<OutputTap> {
    TAP.try_with(|tap| tap.clone()).ok()
}

/// Send `chunk` to the active tap, if any.
pub fn send(chunk: &str) {
    if let Some(tap) = current() {
        tap.send(chunk);
    }
}

/// `child.wait_with_output()`, passing output to the active tap as it is
/// printed. Both pipes are read at once, so neither can fill and stall the
/// child.
pub async fn wait_with_live_output(
    mut child: tokio::process::Child,
) -> std::io::Result<std::process::Output> {
    let Some(tap) = current() else {
        return child.wait_with_output().await;
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (stdout, stderr, status) = tokio::join!(
        read_live(stdout, &tap),
        read_live(stderr, &tap),
        child.wait()
    );
    Ok(std::process::Output {
        status: status?,
        stdout: stdout?,
        stderr: stderr?,
    })
}

async fn read_live<R: tokio::io::AsyncRead + Unpin>(
    pipe: Option<R>,
    tap: &OutputTap,
) -> std::io::Result<Vec<u8>> {
    let mut all = Vec::new();
    let Some(mut pipe) = pipe else {
        return Ok(all);
    };
    let mut buf = [0u8; 8192];
    loop {
        let n = pipe.read(&mut buf).await?;
        if n == 0 {
            return Ok(all);
        }
        tap.send(&String::from_utf8_lossy(&buf[..n]));
        all.extend_from_slice(&buf[..n]);
    }
}

/// The blocking counterpart of [`wait_with_live_output`], for runtimes that
/// drive `std::process`. Each pipe is read on its own thread.
pub fn blocking_wait_with_live_output(
    mut child: std::process::Child,
    tap: Option<OutputTap>,
) -> std::io::Result<std::process::Output> {
    let Some(tap) = tap else {
        return child.wait_with_output();
    };
    fn read_pipe<R: std::io::Read + Send + 'static>(
        pipe: Option<R>,
        tap: OutputTap,
    ) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
        std::thread::spawn(move || {
            let mut all = Vec::new();
            let Some(mut pipe) = pipe else {
                return Ok(all);
            };
            let mut buf = [0u8; 8192];
            loop {
                let n = pipe.read(&mut buf)?;
                if n == 0 {
                    return Ok(all);
                }
                tap.send(&String::from_utf8_lossy(&buf[..n]));
                all.extend_from_slice(&buf[..n]);
            }
        })
    }
    let stdout = read_pipe(child.stdout.take(), tap.clone());
    let stderr = read_pipe(child.stderr.take(), tap);
    let status = child.wait()?;
    let joined = |handle: std::thread::JoinHandle<std::io::Result<Vec<u8>>>| {
        handle
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("output reader panicked")))
    };
    Ok(std::process::Output {
        status,
        stdout: joined(stdout)?,
        stderr: joined(stderr)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn recording_tap() -> (OutputTap, Arc<Mutex<String>>) {
        let seen = Arc::new(Mutex::new(String::new()));
        let sink = seen.clone();
        (
            OutputTap::new(move |chunk| sink.lock().unwrap().push_str(chunk)),
            seen,
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn output_reaches_the_tap_and_the_result() {
        let (tap, seen) = recording_tap();
        let child = tokio::process::Command::new("sh")
            .args(["-c", "echo out; echo err >&2"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let output = with_output_tap(tap, wait_with_live_output(child))
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), "out\n");
        assert_eq!(String::from_utf8_lossy(&output.stderr), "err\n");
        let seen = seen.lock().unwrap();
        assert!(seen.contains("out\n") && seen.contains("err\n"));
    }

    #[cfg(unix)]
    #[test]
    fn blocking_output_reaches_the_tap_and_the_result() {
        let (tap, seen) = recording_tap();
        let child = std::process::Command::new("sh")
            .args(["-c", "echo out; echo err >&2"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let output = blocking_wait_with_live_output(child, Some(tap)).unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), "out\n");
        assert!(seen.lock().unwrap().contains("err\n"));
    }

    #[test]
    fn no_tap_outside_a_scope() {
        assert!(current().is_none());
    }
}
