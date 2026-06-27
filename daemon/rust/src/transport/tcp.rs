use std::io::{self, IoSlice, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::transport::codec::{
    PreparedEvent, WireEvent, encode_log_event_batch_slices, encode_stream_register,
};

const INITIAL_BACKOFF_MS: u64 = 250;
const MAX_BACKOFF_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendResult {
    Sent,
    Backpressure,
    Disconnected,
}

pub struct TcpClient {
    addr: String,
    stream: Option<TcpStream>,
    backoff: Duration,
}

impl TcpClient {
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            stream: None,
            backoff: Duration::from_millis(INITIAL_BACKOFF_MS),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    pub fn ensure_connected(&mut self) -> io::Result<()> {
        if self.stream.is_some() {
            return Ok(());
        }

        loop {
            match TcpStream::connect(&self.addr) {
                Ok(stream) => {
                    stream.set_nodelay(true)?;
                    self.stream = Some(stream);
                    self.backoff = Duration::from_millis(INITIAL_BACKOFF_MS);
                    println!("[Transport] Connected to aggregator at {}", self.addr);
                    return Ok(());
                }
                Err(err) => {
                    eprintln!(
                        "[Transport] Connect failed ({}), retrying in {:?}",
                        err, self.backoff
                    );
                    std::thread::sleep(self.backoff);
                    self.backoff = (self.backoff * 2).min(Duration::from_millis(MAX_BACKOFF_MS));
                }
            }
        }
    }

    pub fn send_stream_register(
        &mut self,
        stream_id: u32,
        namespace: &str,
        pod_name: &str,
        pod_uid: &str,
        container_name: &str,
    ) -> SendResult {
        if self.ensure_connected().is_err() {
            return SendResult::Disconnected;
        }

        let (header, body) = encode_stream_register(
            stream_id,
            namespace,
            pod_name,
            pod_uid,
            container_name,
        );

        let mut register_slices = [
            IoSlice::new(header.as_slice()),
            IoSlice::new(&body),
        ];
        let mut slice_refs: &mut [IoSlice<'_>] = &mut register_slices;
        match self.write_all_vectored(&mut slice_refs) {
            Ok(()) => SendResult::Sent,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => SendResult::Backpressure,
            Err(_) => {
                self.disconnect();
                SendResult::Disconnected
            }
        }
    }

    pub fn send_log_event_batch(&mut self, stream_id: u32, events: &[WireEvent<'_>]) -> SendResult {
        if events.is_empty() {
            return SendResult::Sent;
        }

        if self.ensure_connected().is_err() {
            return SendResult::Disconnected;
        }

        let prepared: Vec<PreparedEvent<'_>> =
            events.iter().copied().map(PreparedEvent::from_wire).collect();
        let (header, batch_prefix, mut event_slices) =
            encode_log_event_batch_slices(stream_id, &prepared);

        let mut slices = vec![
            IoSlice::new(header.as_slice()),
            IoSlice::new(&batch_prefix),
        ];
        slices.append(&mut event_slices);

        let mut slice_refs: &mut [IoSlice<'_>] = &mut slices;
        match self.write_all_vectored(&mut slice_refs) {
            Ok(()) => SendResult::Sent,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => SendResult::Backpressure,
            Err(_) => {
                self.disconnect();
                SendResult::Disconnected
            }
        }
    }

    fn write_all_vectored(&mut self, bufs: &mut &mut [IoSlice<'_>]) -> io::Result<()> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "not connected"))?;

        while !bufs.is_empty() {
            let written = stream.write_vectored(bufs)?;
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "socket closed during write",
                ));
            }
            IoSlice::advance_slices(bufs, written);
        }

        stream.flush()?;
        Ok(())
    }

    fn disconnect(&mut self) {
        if self.stream.take().is_some() {
            eprintln!("[Transport] Disconnected from aggregator, will reconnect");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn test_send_log_event_batch_over_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 4096];
            let n = conn.read(&mut buf).unwrap();
            tx.send(buf[..n].to_vec()).unwrap();
        });

        let mut client = TcpClient::new(addr.to_string());
        let result = client.send_log_event_batch(
            11,
            &[WireEvent {
                timestamp_ns: 42,
                is_stderr: false,
                payload: b"hello tcp",
            }],
        );

        assert_eq!(result, SendResult::Sent);

        let frame = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(&frame[0..4], &30u32.to_be_bytes());
        assert_eq!(frame[4], crate::transport::codec::FRAME_LOG_EVENT_BATCH);
        assert_eq!(&frame[5..9], &11u32.to_be_bytes());
        assert_eq!(&frame[9..13], &1u32.to_be_bytes());
        assert!(frame[13..].windows(9).any(|w| w.ends_with(b"hello tcp")));
    }
}
