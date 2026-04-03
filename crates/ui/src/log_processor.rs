// Background log processor for asynchronous log filtering and formatting
use crate::models::LogFilterLevel;
use crate::theme;
use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Row},
};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Processed log entry ready for rendering
#[derive(Debug, Clone)]
pub struct ProcessedLogEntry {
    pub timestamp: String,
    pub log_type: String,
    pub log_style: Style,
    pub content_spans: Vec<Span<'static>>,
}

impl ProcessedLogEntry {
    /// Convert to a table row for rendering
    pub fn to_row(&self) -> Row<'static> {
        Row::new(vec![
            Cell::from(self.timestamp.clone()).style(theme::muted_style()),
            Cell::from(format!(" {} ", self.log_type)).style(self.log_style),
            Cell::from(Line::from(self.content_spans.clone())),
        ])
    }
}

/// Request to update log processing parameters
#[derive(Debug, Clone)]
pub struct LogProcessingRequest {
    pub search_query: String,
    pub filter_level: Option<LogFilterLevel>,
    pub app_logs: Vec<String>,    // Complete app logs
    pub app_logs_count: usize,    // To detect changes in app logs
    pub system_logs_count: usize, // To detect changes in system logs
}

/// Response with processed logs
#[derive(Debug, Clone)]
pub struct LogProcessingResponse {
    pub processed_logs: Vec<ProcessedLogEntry>,
    pub total_log_count: usize,
    pub filtered_count: usize,
    pub search_matches: Vec<usize>, // Indices of logs that match search
}

/// Background log processor
pub struct LogProcessor {
    request_tx: mpsc::Sender<LogProcessingRequest>,
    response_rx: mpsc::Receiver<LogProcessingResponse>,
    _worker_handle: thread::JoinHandle<()>,
}

impl LogProcessor {
    /// Create a new log processor with a background worker thread
    pub fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<LogProcessingRequest>();
        let (response_tx, response_rx) = mpsc::channel::<LogProcessingResponse>();

        let worker_handle = thread::spawn(move || {
            Self::worker_loop(request_rx, response_tx);
        });

        Self {
            request_tx,
            response_rx,
            _worker_handle: worker_handle,
        }
    }

    /// Send a processing request (non-blocking)
    pub fn request_update(
        &self,
        request: LogProcessingRequest,
    ) -> Result<(), mpsc::SendError<LogProcessingRequest>> {
        self.request_tx.send(request)
    }

    /// Try to get the latest processed logs (non-blocking)
    pub fn try_get_update(&self) -> Option<LogProcessingResponse> {
        self.response_rx.try_recv().ok()
    }

    /// Background worker loop
    fn worker_loop(
        request_rx: mpsc::Receiver<LogProcessingRequest>,
        response_tx: mpsc::Sender<LogProcessingResponse>,
    ) {
        let mut last_request: Option<LogProcessingRequest> = None;
        let mut last_processed_time = Instant::now();
        let mut cached_logs: Vec<String> = Vec::new();
        let mut cached_app_logs_count = 0;
        let mut cached_system_logs_count = 0;

        loop {
            let request = match request_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(req) => Some(req),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };

            if let Some(req) = request {
                last_request = Some(req);
            }

            if let Some(ref req) = last_request {
                let should_process = last_processed_time.elapsed() > Duration::from_millis(50)
                    && (cached_app_logs_count != req.app_logs_count
                        || cached_system_logs_count != req.system_logs_count
                        || cached_logs.is_empty());

                if should_process {
                    if cached_app_logs_count != req.app_logs_count
                        || cached_system_logs_count != req.system_logs_count
                        || cached_logs.is_empty()
                    {
                        cached_logs = Self::get_combined_logs(&req.app_logs);
                        cached_app_logs_count = req.app_logs_count;
                        cached_system_logs_count = req.system_logs_count;
                    }

                    let response = Self::process_logs(&cached_logs, req);

                    if response_tx.send(response).is_err() {
                        break;
                    }

                    last_processed_time = Instant::now();
                }
            }
        }
    }

    /// Get combined app and system logs
    fn get_combined_logs(app_logs: &[String]) -> Vec<String> {
        let mut all_logs = Vec::new();

        for log in app_logs {
            all_logs.push(log.clone());
        }

        for log in wrkflw_logging::get_logs() {
            all_logs.push(log.clone());
        }

        all_logs
    }

    /// Process logs according to search and filter criteria
    fn process_logs(all_logs: &[String], request: &LogProcessingRequest) -> LogProcessingResponse {
        let mut filtered_logs = Vec::new();
        let mut search_matches = Vec::new();

        for (idx, log) in all_logs.iter().enumerate() {
            let passes_filter = match &request.filter_level {
                None => true,
                Some(level) => level.matches(log),
            };

            let matches_search = if request.search_query.is_empty() {
                true
            } else {
                log.to_lowercase()
                    .contains(&request.search_query.to_lowercase())
            };

            if passes_filter && matches_search {
                filtered_logs.push((idx, log));
                if matches_search && !request.search_query.is_empty() {
                    search_matches.push(filtered_logs.len() - 1);
                }
            }
        }

        let processed_logs: Vec<ProcessedLogEntry> = filtered_logs
            .iter()
            .map(|(_, log_line)| Self::process_log_entry(log_line, &request.search_query))
            .collect();

        LogProcessingResponse {
            processed_logs,
            total_log_count: all_logs.len(),
            filtered_count: filtered_logs.len(),
            search_matches,
        }
    }

    /// Process a single log entry into display format
    fn process_log_entry(log_line: &str, search_query: &str) -> ProcessedLogEntry {
        // Extract timestamp from log format [HH:MM:SS]
        let timestamp = if log_line.starts_with('[') && log_line.contains(']') {
            let end = log_line.find(']').unwrap_or(0);
            if end > 1 && log_line.is_char_boundary(1) && log_line.is_char_boundary(end) {
                log_line[1..end].to_string()
            } else {
                "??:??:??".to_string()
            }
        } else {
            "??:??:??".to_string()
        };

        // Determine log type and style using theme badge styles
        let (log_type, log_style) = if log_line.contains("Error")
            || log_line.contains("error")
            || log_line.contains(theme::symbols::FAILURE)
        {
            ("ERROR", theme::log_badge("ERROR"))
        } else if log_line.contains("Warning")
            || log_line.contains("warning")
            || log_line.contains(theme::symbols::WARNING)
        {
            ("WARN", theme::log_badge("WARN"))
        } else if log_line.contains("Success")
            || log_line.contains("success")
            || log_line.contains(theme::symbols::SUCCESS)
        {
            ("SUCCESS", theme::log_badge("SUCCESS"))
        } else if log_line.contains("Running")
            || log_line.contains("running")
            || log_line.contains(theme::symbols::RUNNING)
        {
            ("INFO", theme::log_badge("INFO"))
        } else if log_line.contains("Triggering") || log_line.contains("triggered") {
            ("TRIG", theme::log_badge("TRIG"))
        } else {
            ("INFO", theme::log_badge(""))
        };

        // Extract content after timestamp
        let content = if log_line.starts_with('[') && log_line.contains(']') {
            let start = log_line.find(']').unwrap_or(0) + 1;
            if log_line.is_char_boundary(start) {
                log_line[start..].trim()
            } else {
                log_line
            }
        } else {
            log_line
        };

        // Create content spans with search highlighting
        let content_spans = if !search_query.is_empty() {
            Self::highlight_search_matches(content, search_query)
        } else {
            vec![Span::raw(content.to_string())]
        };

        ProcessedLogEntry {
            timestamp,
            log_type: log_type.to_string(),
            log_style,
            content_spans,
        }
    }

    /// Highlight search matches in content
    fn highlight_search_matches(content: &str, search_query: &str) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let lowercase_content = content.to_lowercase();
        let lowercase_query = search_query.to_lowercase();

        if lowercase_content.contains(&lowercase_query) {
            let mut last_idx = 0;
            while let Some(idx) = lowercase_content[last_idx..].find(&lowercase_query) {
                let real_idx = last_idx + idx;

                if real_idx > last_idx {
                    spans.push(Span::raw(content[last_idx..real_idx].to_string()));
                }

                let match_end = real_idx + search_query.len();
                spans.push(Span::styled(
                    content[real_idx..match_end].to_string(),
                    theme::search_highlight(),
                ));

                last_idx = match_end;
            }

            if last_idx < content.len() {
                spans.push(Span::raw(content[last_idx..].to_string()));
            }
        } else {
            spans.push(Span::raw(content.to_string()));
        }

        spans
    }
}

impl Default for LogProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multibyte_log_line_does_not_panic() {
        // Emoji and multi-byte characters near bracket boundaries
        let entry = LogProcessor::process_log_entry("[🚀] deployed service", "");
        assert_eq!(entry.log_type, "INFO");

        let entry2 = LogProcessor::process_log_entry("[ñ] latin char", "");
        assert!(!entry2.timestamp.is_empty());
    }

    #[test]
    fn test_normal_timestamp_extraction() {
        let entry = LogProcessor::process_log_entry("[12:34:56] some log", "");
        assert_eq!(entry.timestamp, "12:34:56");
    }
}
