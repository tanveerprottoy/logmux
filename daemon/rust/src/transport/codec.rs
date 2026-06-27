use std::io::IoSlice;

pub const FRAME_STREAM_REGISTER: u8 = 0x01;
pub const FRAME_LOG_EVENT_BATCH: u8 = 0x02;

#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    bytes: [u8; 5],
}

impl FrameHeader {
    pub fn new(payload_len: u32, frame_type: u8) -> Self {
        let mut bytes = [0u8; 5];
        bytes[0..4].copy_from_slice(&payload_len.to_be_bytes());
        bytes[4] = frame_type;
        Self { bytes }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WireEvent<'a> {
    pub timestamp_ns: i64,
    pub is_stderr: bool,
    pub payload: &'a [u8],
}

#[derive(Debug)]
pub struct PreparedEvent<'a> {
    pub header: [u8; 13],
    pub payload: &'a [u8],
}

impl<'a> PreparedEvent<'a> {
    pub fn from_wire(event: WireEvent<'a>) -> Self {
        let mut header = [0u8; 13];
        header[0..8].copy_from_slice(&event.timestamp_ns.to_be_bytes());
        header[8] = u8::from(event.is_stderr);
        header[9..13].copy_from_slice(&(event.payload.len() as u32).to_be_bytes());
        Self {
            header,
            payload: event.payload,
        }
    }
}

pub fn encode_stream_register(
    stream_id: u32,
    namespace: &str,
    pod_name: &str,
    pod_uid: &str,
    container_name: &str,
) -> (FrameHeader, Vec<u8>) {
    let ns = namespace.as_bytes();
    let pod = pod_name.as_bytes();
    let uid = pod_uid.as_bytes();
    let container = container_name.as_bytes();

    let payload_len = 4
        + 2
        + ns.len()
        + 2
        + pod.len()
        + 2
        + uid.len()
        + 2
        + container.len();

    let header = FrameHeader::new(payload_len as u32, FRAME_STREAM_REGISTER);

    let mut body = Vec::with_capacity(payload_len);
    body.extend_from_slice(&stream_id.to_be_bytes());
    body.extend_from_slice(&(ns.len() as u16).to_be_bytes());
    body.extend_from_slice(ns);
    body.extend_from_slice(&(pod.len() as u16).to_be_bytes());
    body.extend_from_slice(pod);
    body.extend_from_slice(&(uid.len() as u16).to_be_bytes());
    body.extend_from_slice(uid);
    body.extend_from_slice(&(container.len() as u16).to_be_bytes());
    body.extend_from_slice(container);

    (header, body)
}

pub fn encode_log_event_batch_slices<'a>(
    stream_id: u32,
    events: &'a [PreparedEvent<'a>],
) -> (FrameHeader, [u8; 8], Vec<IoSlice<'a>>) {
    let mut payload_len = 8usize;
    for event in events {
        payload_len += 13 + event.payload.len();
    }

    let header = FrameHeader::new(payload_len as u32, FRAME_LOG_EVENT_BATCH);
    let mut batch_prefix = [0u8; 8];
    batch_prefix[0..4].copy_from_slice(&stream_id.to_be_bytes());
    batch_prefix[4..8].copy_from_slice(&(events.len() as u32).to_be_bytes());

    let mut slices = Vec::with_capacity(events.len() * 2);
    for event in events {
        slices.push(IoSlice::new(&event.header));
        slices.push(IoSlice::new(event.payload));
    }

    (header, batch_prefix, slices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_header_big_endian() {
        let header = FrameHeader::new(42, FRAME_LOG_EVENT_BATCH);
        assert_eq!(header.as_slice(), &[0, 0, 0, 42, FRAME_LOG_EVENT_BATCH]);
    }

    #[test]
    fn test_prepared_event_header_layout() {
        let event = PreparedEvent::from_wire(WireEvent {
            timestamp_ns: 1_000_000_001,
            is_stderr: true,
            payload: b"error",
        });

        assert_eq!(&event.header[0..8], &1_000_000_001i64.to_be_bytes());
        assert_eq!(event.header[8], 1);
        assert_eq!(&event.header[9..13], &5u32.to_be_bytes());
        assert_eq!(event.payload, b"error");
    }

    #[test]
    fn test_stream_register_payload_length() {
        let (header, body) = encode_stream_register(
            7,
            "default",
            "nginx",
            "uid-123",
            "web",
        );
        let expected = 4 + 2 + 7 + 2 + 5 + 2 + 7 + 2 + 3;
        assert_eq!(
            u32::from_be_bytes(header.as_slice()[0..4].try_into().unwrap()),
            expected as u32
        );
        assert_eq!(body.len(), expected);
    }
}
