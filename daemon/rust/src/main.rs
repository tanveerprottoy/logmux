pub mod parser;
pub mod collector;
pub mod transport;

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use collector::{LogTailer, TailerMessage};
use parser::{ParsedLog, StreamType};
use transport::{SendResult, TcpClient, WireEvent};

const DEFAULT_LOG_DIR: &str = "/var/log/pods";
const DEFAULT_AGGREGATOR: &str = "127.0.0.1:8080";
const BATCH_MAX_EVENTS: usize = 64;
const BATCH_MAX_BYTES: usize = 56 * 1024;

#[derive(Debug, Clone)]
struct PodMetadata {
    namespace: String,
    pod_name: String,
    pod_uid: String,
    container_name: String,
}

#[derive(Debug)]
struct StreamState {
    stream_id: u32,
    metadata: PodMetadata,
    registered: bool,
    pending: Vec<StoredEvent>,
}

#[derive(Debug, Clone)]
struct StoredEvent {
    timestamp_ns: i64,
    is_stderr: bool,
    payload: Vec<u8>,
}

struct Dispatcher {
    client: TcpClient,
    streams: HashMap<PathBuf, StreamState>,
    next_stream_id: u32,
}

impl Dispatcher {
    fn new(client: TcpClient) -> Self {
        Self {
            client,
            streams: HashMap::new(),
            next_stream_id: 1,
        }
    }

    fn handle_event(&mut self, path: &Path, event: ParsedLog) -> bool {
        self.ensure_stream(path);

        if !self.streams.get(path).map(|s| s.registered).unwrap_or(false) {
            if self.register_stream(path) != SendResult::Sent {
                return false;
            }
        }

        let stream = self.streams.get_mut(path).expect("stream must exist");
        stream.pending.push(StoredEvent {
            timestamp_ns: event.timestamp_ns,
            is_stderr: matches!(event.stream, StreamType::Stderr),
            payload: event.payload.to_vec(),
        });

        let stream = self.streams.get(path).expect("stream must exist");
        let pending_len = stream.pending.len();
        let pending_bytes: usize = stream.pending.iter().map(|e| e.payload.len()).sum();
        if pending_len >= BATCH_MAX_EVENTS || pending_bytes >= BATCH_MAX_BYTES {
            return self.flush_path(path);
        }

        true
    }

    fn flush_if_pending(&mut self, path: &Path) -> bool {
        if self
            .streams
            .get(path)
            .map(|stream| stream.pending.is_empty())
            .unwrap_or(true)
        {
            return true;
        }
        self.flush_path(path)
    }

    fn register_stream(&mut self, path: &Path) -> SendResult {
        let (stream_id, metadata) = {
            let stream = self.streams.get(path).expect("stream must exist");
            (stream.stream_id, stream.metadata.clone())
        };

        match self.client.send_stream_register(
            stream_id,
            &metadata.namespace,
            &metadata.pod_name,
            &metadata.pod_uid,
            &metadata.container_name,
        ) {
            SendResult::Sent => {
                if let Some(stream) = self.streams.get_mut(path) {
                    stream.registered = true;
                }
                SendResult::Sent
            }
            other => other,
        }
    }

    fn flush_path(&mut self, path: &Path) -> bool {
        let needs_register = self
            .streams
            .get(path)
            .map(|stream| !stream.registered)
            .unwrap_or(false);
        if needs_register && self.register_stream(path) != SendResult::Sent {
            return false;
        }

        let mut pending = {
            let stream = match self.streams.get_mut(path) {
                Some(stream) => stream,
                None => return true,
            };
            if stream.pending.is_empty() {
                return true;
            }
            std::mem::take(&mut stream.pending)
        };

        let stream_id = self.streams.get(path).expect("stream must exist").stream_id;
        let batch: Vec<WireEvent<'_>> = pending
            .iter()
            .map(|event| WireEvent {
                timestamp_ns: event.timestamp_ns,
                is_stderr: event.is_stderr,
                payload: &event.payload,
            })
            .collect();

        match self.client.send_log_event_batch(stream_id, &batch) {
            SendResult::Sent => true,
            SendResult::Backpressure | SendResult::Disconnected => {
                if let Some(stream) = self.streams.get_mut(path) {
                    if stream.pending.is_empty() {
                        stream.pending = pending;
                    } else {
                        pending.append(&mut stream.pending);
                        stream.pending = pending;
                    }
                }
                false
            }
        }
    }

    fn ensure_stream(&mut self, path: &Path) {
        if self.streams.contains_key(path) {
            return;
        }

        let metadata = parse_pod_metadata(path);
        let stream_id = self.next_stream_id;
        self.next_stream_id += 1;
        self.streams.insert(
            path.to_path_buf(),
            StreamState {
                stream_id,
                metadata,
                registered: false,
                pending: Vec::with_capacity(BATCH_MAX_EVENTS),
            },
        );
    }
}

fn parse_pod_metadata(path: &Path) -> PodMetadata {
    // Expected: .../pods/<namespace>_<pod>_<uid>/<container>/<n>.log
    let mut namespace = "default".to_string();
    let mut pod_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut pod_uid = "unknown".to_string();
    let mut container_name = "unknown".to_string();

    if let Some(parent) = path.parent() {
        if let Some(container) = parent.file_name().and_then(|n| n.to_str()) {
            container_name = container.to_string();
        }

        if let Some(pod_dir) = parent.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str())
        {
            if let Some((ns, rest)) = pod_dir.split_once('_') {
                namespace = ns.to_string();
                if let Some((name, uid)) = rest.rsplit_once('_') {
                    pod_name = name.to_string();
                    pod_uid = uid.to_string();
                }
            }
        }
    }

    PodMetadata {
        namespace,
        pod_name,
        pod_uid,
        container_name,
    }
}

fn main() {
    let log_dir = env::var("LOGMUX_LOG_DIR").unwrap_or_else(|_| DEFAULT_LOG_DIR.to_string());
    let aggregator_addr =
        env::var("LOGMUX_AGGREGATOR").unwrap_or_else(|_| DEFAULT_AGGREGATOR.to_string());

    println!("LogMux Daemon starting");
    println!("  log directory: {}", log_dir);
    println!("  aggregator:    {}", aggregator_addr);

    let mut tailer = match LogTailer::new(&log_dir) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("Failed to initialize tailer: {err}");
            std::process::exit(1);
        }
    };

    if let Err(err) = tailer.initialize() {
        eprintln!("Failed to initialize watches: {err}");
        std::process::exit(1);
    }

    let mut dispatcher = Dispatcher::new(TcpClient::new(aggregator_addr));

    if let Err(err) = tailer.run(|path, message| match message {
        TailerMessage::Event(event) => dispatcher.handle_event(path, event),
        TailerMessage::Flush => dispatcher.flush_if_pending(path),
    }) {
        eprintln!("Tailer exited with error: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pod_metadata_from_k8s_path() {
        let path = Path::new("/var/log/pods/default_nginx-abc_9f3c2b1/web/0.log");
        let meta = parse_pod_metadata(path);

        assert_eq!(meta.namespace, "default");
        assert_eq!(meta.pod_name, "nginx-abc");
        assert_eq!(meta.pod_uid, "9f3c2b1");
        assert_eq!(meta.container_name, "web");
    }
}
