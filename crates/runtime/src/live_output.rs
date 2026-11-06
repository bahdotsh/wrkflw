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
pub struct OutputTap(Arc<TapFn>);

/// What a tap calls with each chunk of a stream.
type TapFn = dyn Fn(Stream, &str) + Send + Sync;

/// Which of a process's output streams a chunk came from. The two are
/// written independently, so a partial line on one is no part of a line on
/// the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl OutputTap {
    pub fn new(f: impl Fn(Stream, &str) + Send + Sync + 'static) -> Self {
        OutputTap(Arc::new(f))
    }

    pub fn send(&self, stream: Stream, chunk: &str) {
        if !chunk.is_empty() {
            (self.0)(stream, chunk);
        }
    }
}

/// Turns a byte stream read in arbitrary chunks into text without breaking
/// a character that straddles two chunks: the incomplete tail of one chunk
/// is held and completed by the next. Bytes that can never form valid
/// UTF-8 still become U+FFFD, as `String::from_utf8_lossy` makes them.
#[derive(Default)]
pub struct Utf8Chunks {
    held: Vec<u8>,
}

impl Utf8Chunks {
    /// The text `bytes` completes, holding back a trailing partial character.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.held.extend_from_slice(bytes);
        let keep = incomplete_tail(&self.held);
        let complete_len = self.held.len() - keep;
        let text = String::from_utf8_lossy(&self.held[..complete_len]).into_owned();
        self.held.drain(..complete_len);
        text
    }

    /// Whatever is still held; a stream that ends mid-character ends with
    /// a replacement character, as a lossy conversion of the whole would.
    pub fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.held).into_owned();
        self.held.clear();
        text
    }
}

/// How many bytes at the end of `bytes` begin a character that the next
/// chunk may still complete. A character is at most four bytes, so only the
/// last three can be such a prefix; std's decoder says whether they are one.
fn incomplete_tail(bytes: &[u8]) -> usize {
    let mut start = bytes.len().saturating_sub(3);
    // Continuation bytes at the window's start belong to an earlier character.
    while start < bytes.len() && bytes[start] & 0b1100_0000 == 0b1000_0000 {
        start += 1;
    }
    loop {
        match std::str::from_utf8(&bytes[start..]) {
            // `error_len() == None`: the input ended inside a valid prefix.
            Err(e) if e.error_len().is_none() => return bytes.len() - start - e.valid_up_to(),
            // An invalid byte; a valid prefix may still follow it.
            Err(e) => start += e.valid_up_to() + e.error_len().unwrap_or(1),
            Ok(_) => return 0,
        }
        if start >= bytes.len() {
            return 0;
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

/// Send `chunk` of `stream` to the active tap, if any.
pub fn send(stream: Stream, chunk: &str) {
    if let Some(tap) = current() {
        tap.send(stream, chunk);
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
        read_live(stdout, Stream::Stdout, &tap),
        read_live(stderr, Stream::Stderr, &tap),
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
    stream: Stream,
    tap: &OutputTap,
) -> std::io::Result<Vec<u8>> {
    let mut all = Vec::new();
    let Some(mut pipe) = pipe else {
        return Ok(all);
    };
    let mut buf = [0u8; 8192];
    let mut text = Utf8Chunks::default();
    loop {
        let n = pipe.read(&mut buf).await?;
        if n == 0 {
            tap.send(stream, &text.finish());
            return Ok(all);
        }
        tap.send(stream, &text.push(&buf[..n]));
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
        stream: Stream,
        tap: OutputTap,
    ) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
        std::thread::spawn(move || {
            let mut all = Vec::new();
            let Some(mut pipe) = pipe else {
                return Ok(all);
            };
            let mut buf = [0u8; 8192];
            let mut text = Utf8Chunks::default();
            loop {
                let n = pipe.read(&mut buf)?;
                if n == 0 {
                    tap.send(stream, &text.finish());
                    return Ok(all);
                }
                tap.send(stream, &text.push(&buf[..n]));
                all.extend_from_slice(&buf[..n]);
            }
        })
    }
    let stdout = read_pipe(child.stdout.take(), Stream::Stdout, tap.clone());
    let stderr = read_pipe(child.stderr.take(), Stream::Stderr, tap);
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
            OutputTap::new(move |_, chunk| sink.lock().unwrap().push_str(chunk)),
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
    fn a_character_split_across_reads_arrives_whole() {
        let text = "ok — 日本 🦀 done";
        let bytes = text.as_bytes();
        // Every split point, including inside each multibyte character.
        for cut in 0..=bytes.len() {
            let mut chunks = Utf8Chunks::default();
            let mut out = chunks.push(&bytes[..cut]);
            out.push_str(&chunks.push(&bytes[cut..]));
            out.push_str(&chunks.finish());
            assert_eq!(out, text, "split at byte {cut}");
        }
        // One byte at a time.
        let mut chunks = Utf8Chunks::default();
        let out: String = bytes.iter().map(|b| chunks.push(&[*b])).collect();
        assert_eq!(out + &chunks.finish(), text);
    }

    #[test]
    fn a_partial_character_after_an_invalid_byte_is_still_held() {
        let mut text = Utf8Chunks::default();
        // 0xFF can never start a character; "日" is E6 97 A5.
        assert_eq!(text.push(&[b'a', 0xFF, 0xE6, 0x97]), "a\u{FFFD}");
        assert_eq!(text.push(&[0xA5, b'b']), "日b");
    }

    #[test]
    fn invalid_bytes_are_replaced_not_held() {
        let mut chunks = Utf8Chunks::default();
        // 0xFF never starts a character; 0xE0 0x80 can never be completed.
        assert_eq!(chunks.push(b"a\xFFb"), "a\u{FFFD}b");
        assert_eq!(chunks.push(b"c\xE0\x80"), "c\u{FFFD}\u{FFFD}");
        // A stream that ends mid-character ends with a replacement.
        assert_eq!(chunks.push(b"\xE6\x97"), "");
        assert_eq!(chunks.finish(), "\u{FFFD}");
    }

    #[test]
    fn no_tap_outside_a_scope() {
        assert!(current().is_none());
    }
}
