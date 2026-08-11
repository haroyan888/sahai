//! コンテナログの読み出し。Dockerが保持しているものを行単位に整形して流すだけで、
//! 保存も加工もしない(要件定義書9章)。
//!
//! 実コンテナ名は`svc-{container_id}`に統一されているため、image型/compose型の
//! 区別は不要(docker/mod.rs参照)。

use bollard::container::{LogOutput, LogsOptions};
use bollard::Docker;
use futures_util::{Stream, StreamExt};

use super::DockerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStreamKind {
    Stdout,
    Stderr,
}

impl LogStreamKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LogStreamKind::Stdout => "stdout",
            LogStreamKind::Stderr => "stderr",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub stream: LogStreamKind,
    /// Dockerが記録した時刻。行の先頭に付いていなければNone。
    pub timestamp: Option<String>,
    pub message: String,
}

/// 直近`tail`行を送ってから、以降の追記を流し続ける(`docker logs --tail N --follow`相当)。
/// 呼び出し側がストリームを捨てた時点で読み出しも止まる。
pub fn stream_logs(
    docker: &Docker,
    container_name: &str,
    tail: u32,
) -> impl Stream<Item = Result<LogLine, DockerError>> {
    let options = LogsOptions::<String> {
        follow: true,
        stdout: true,
        stderr: true,
        timestamps: true,
        tail: tail.to_string(),
        ..Default::default()
    };
    let inner = Box::pin(docker.logs(container_name, Some(options)));

    // Dockerが返すフレームは行の区切りと一致しない(1フレームに複数行が入ることも、
    // 行の途中で切れることもある)。バッファに貯めて改行で切り出す。
    // 行の切り出しで1フレームから複数行が出るため、取り出し待ちのキューを挟む
    futures_util::stream::unfold(
        StreamState {
            inner,
            buffers: LineBuffers::default(),
            queue: std::collections::VecDeque::new(),
            finished: false,
        },
        |mut state| async move {
            loop {
                if let Some(item) = state.queue.pop_front() {
                    return Some((item, state));
                }
                if state.finished {
                    return None;
                }
                match state.inner.next().await {
                    Some(Ok(output)) => {
                        state
                            .queue
                            .extend(state.buffers.push(output).into_iter().map(Ok));
                    }
                    Some(Err(e)) => {
                        state.finished = true;
                        state.queue.push_back(Err(DockerError::Bollard(e)));
                    }
                    None => {
                        // 改行で終わらないまま終了した分を取りこぼさない。
                        // 異常終了の直前に出力された最後の1行が、まさにこれになりうる
                        state.finished = true;
                        state
                            .queue
                            .extend(state.buffers.flush().into_iter().map(Ok));
                    }
                }
            }
        },
    )
}

struct StreamState<S> {
    inner: S,
    buffers: LineBuffers,
    queue: std::collections::VecDeque<Result<LogLine, DockerError>>,
    finished: bool,
}

/// 標準出力・標準エラーは別々のフレームで届くため、行の途中で混ざらないよう
/// バッファも分ける。
#[derive(Default)]
struct LineBuffers {
    stdout: String,
    stderr: String,
}

impl LineBuffers {
    fn push(&mut self, output: LogOutput) -> Vec<LogLine> {
        let (kind, bytes) = match output {
            LogOutput::StdErr { message } => (LogStreamKind::Stderr, message),
            // ConsoleはTTY付きコンテナの出力。標準出力と区別できないためまとめる。
            // StdInは実際には届かないが、網羅のためこちらへ寄せる
            LogOutput::StdOut { message }
            | LogOutput::Console { message }
            | LogOutput::StdIn { message } => (LogStreamKind::Stdout, message),
        };
        let buffer = self.buffer_for(kind);
        buffer.push_str(&String::from_utf8_lossy(&bytes));
        take_complete_lines(buffer, kind)
    }

    fn flush(&mut self) -> Vec<LogLine> {
        let mut lines = Vec::new();
        for kind in [LogStreamKind::Stdout, LogStreamKind::Stderr] {
            let buffer = self.buffer_for(kind);
            if !buffer.is_empty() {
                let rest = std::mem::take(buffer);
                lines.push(build_line(&rest, kind));
            }
        }
        lines
    }

    fn buffer_for(&mut self, kind: LogStreamKind) -> &mut String {
        match kind {
            LogStreamKind::Stdout => &mut self.stdout,
            LogStreamKind::Stderr => &mut self.stderr,
        }
    }
}

/// バッファから改行までの完結した行を取り出す。改行が来ていない末尾は残す。
fn take_complete_lines(buffer: &mut String, kind: LogStreamKind) -> Vec<LogLine> {
    let mut lines = Vec::new();
    while let Some(pos) = buffer.find('\n') {
        let raw: String = buffer.drain(..=pos).collect();
        lines.push(build_line(&raw, kind));
    }
    lines
}

fn build_line(raw: &str, kind: LogStreamKind) -> LogLine {
    let trimmed = raw.trim_end_matches('\n').trim_end_matches('\r');
    let (timestamp, message) = split_timestamp(trimmed);
    LogLine {
        stream: kind,
        timestamp,
        message,
    }
}

/// `timestamps: true`で取得した行の先頭に付くRFC3339時刻を切り離す。
/// 時刻として解釈できない場合は行全体をそのままメッセージとして扱う
/// (先頭が空白区切りの何かであっても、勝手に落とさない)。
fn split_timestamp(raw: &str) -> (Option<String>, String) {
    let Some((head, rest)) = raw.split_once(' ') else {
        return (None, raw.to_string());
    };
    match chrono::DateTime::parse_from_rfc3339(head) {
        Ok(_) => (Some(head.to_string()), rest.to_string()),
        Err(_) => (None, raw.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdout(message: &str) -> LogOutput {
        LogOutput::StdOut {
            message: message.as_bytes().to_vec().into(),
        }
    }

    fn stderr(message: &str) -> LogOutput {
        LogOutput::StdErr {
            message: message.as_bytes().to_vec().into(),
        }
    }

    const TS: &str = "2026-08-11T07:05:25.472263000Z";

    #[test]
    fn 時刻を切り離してメッセージだけ残す() {
        let mut buffers = LineBuffers::default();
        let lines = buffers.push(stdout(&format!("{TS} Listening on :3000\n")));
        assert_eq!(
            lines,
            vec![LogLine {
                stream: LogStreamKind::Stdout,
                timestamp: Some(TS.to_string()),
                message: "Listening on :3000".to_string(),
            }]
        );
    }

    /// 時刻に見えない先頭語を時刻として食べてしまうと、メッセージが欠ける。
    #[test]
    fn 時刻でない先頭語は落とさない() {
        let mut buffers = LineBuffers::default();
        let lines = buffers.push(stdout("ERROR connection refused\n"));
        assert_eq!(lines[0].timestamp, None);
        assert_eq!(lines[0].message, "ERROR connection refused");
    }

    #[test]
    fn 複数行が入ったフレームを分割する() {
        let mut buffers = LineBuffers::default();
        let lines = buffers.push(stdout(&format!("{TS} one\n{TS} two\n")));
        let messages: Vec<_> = lines.iter().map(|l| l.message.as_str()).collect();
        assert_eq!(messages, vec!["one", "two"]);
    }

    /// フレームは行の途中で切れる。切れた分を即座に1行として出すと、
    /// 1行が2行に割れて表示される。
    #[test]
    fn 行の途中で切れたフレームは次と繋げる() {
        let mut buffers = LineBuffers::default();
        assert!(buffers.push(stdout(&format!("{TS} first half"))).is_empty());
        let lines = buffers.push(stdout(" second half\n"));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].message, "first half second half");
    }

    /// 異常終了の直前に出力された最後の1行が、改行を伴わないことがある。
    #[test]
    fn 改行で終わらない末尾をflushで取り出す() {
        let mut buffers = LineBuffers::default();
        assert!(buffers.push(stdout(&format!("{TS} panicked"))).is_empty());
        let lines = buffers.flush();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].message, "panicked");
        assert!(buffers.flush().is_empty());
    }

    /// 標準出力と標準エラーは別々のフレームで交互に届く。バッファを共有すると
    /// 片方の途中の行に他方が割り込む。
    #[test]
    fn 標準出力と標準エラーのバッファは混ざらない() {
        let mut buffers = LineBuffers::default();
        assert!(buffers.push(stdout("out-")).is_empty());
        assert!(buffers.push(stderr("err-")).is_empty());
        let out = buffers.push(stdout("done\n"));
        let err = buffers.push(stderr("done\n"));
        assert_eq!(out[0].message, "out-done");
        assert_eq!(out[0].stream, LogStreamKind::Stdout);
        assert_eq!(err[0].message, "err-done");
        assert_eq!(err[0].stream, LogStreamKind::Stderr);
    }

    #[test]
    fn crlfの改行を残さない() {
        let mut buffers = LineBuffers::default();
        let lines = buffers.push(stdout(&format!("{TS} windows\r\n")));
        assert_eq!(lines[0].message, "windows");
    }

    /// 不正なUTF-8で読み出し全体を止めない(ログの中身は任意のバイト列)。
    #[test]
    fn 不正なutf8を含んでも行として扱う() {
        let mut buffers = LineBuffers::default();
        let lines = buffers.push(LogOutput::StdOut {
            message: vec![0xff, 0xfe, b'\n'].into(),
        });
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].stream, LogStreamKind::Stdout);
    }
}
