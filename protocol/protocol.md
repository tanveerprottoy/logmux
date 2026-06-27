# LogMux Custom Binary Wire Protocol Spec

LogMux uses a custom, lightweight, big-endian binary framing format over persistent TCP streams. This format facilitates zero-copy slicing in both Rust/Zig (sender) and Go (receiver) without requiring external serializing dependencies.

## 1. Frame Structuring

Every frame sent over the network is structured with a 5-byte header followed by the frame payload:

```
+-----------------------------------+-------------------+------------------------------+
|  Payload Length (4 bytes, uint32) |  Type (1 byte)    |  Payload (Variable Length)   |
+-----------------------------------+-------------------+------------------------------+
```

* **Payload Length**: Big-endian 32-bit unsigned integer representing the exact number of bytes in the payload following the 5-byte header.
* **Type**: 1-byte framing discriminator:
  * `0x01`: `StreamRegister` Frame
  * `0x02`: `LogEventBatch` Frame

---

## 2. Frames Specification

### 2.1 StreamRegister Frame (`Type = 0x01`)
Sent once by the daemon to establish the link between a locally managed `stream_id` (a monotonically increasing index) and the corresponding Kubernetes pod metadata.

```
+-----------------------------------+
|  stream_id (4 bytes, uint32)      |
+-----------------------------------+
|  namespace_len (2 bytes, uint16)  |
+-----------------------------------+
|  namespace (UTF-8 bytes)          |
+-----------------------------------+
|  pod_name_len (2 bytes, uint16)   |
+-----------------------------------+
|  pod_name (UTF-8 bytes)           |
+-----------------------------------+
|  pod_uid_len (2 bytes, uint16)    |
+-----------------------------------+
|  pod_uid (UTF-8 bytes)            |
+-----------------------------------+
|  container_name_len (2b, uint16)  |
+-----------------------------------+
|  container_name (UTF-8 bytes)     |
+-----------------------------------+
```

### 2.2 LogEventBatch Frame (`Type = 0x02`)
Used to stream batched log items. Batching reduces network transmission frequency and system call overhead.

```
+-----------------------------------+
|  stream_id (4 bytes, uint32)      |
+-----------------------------------+
|  event_count (4 bytes, uint32)    |
+-----------------------------------+
|  Event 1                          |
+-----------------------------------+
|  Event 2                          |
+-----------------------------------+
|  ...                              |
+-----------------------------------+
```

#### Event Format

Each individual event inside a batch is structured as follows:

```
+-----------------------------------+
|  timestamp_ns (8 bytes, int64)    |
+-----------------------------------+
|  is_stderr (1 byte, uint8)        |  --> 0x00 for stdout, 0x01 for stderr
+-----------------------------------+
|  payload_len (4 bytes, uint32)    |
+-----------------------------------+
|  payload (raw bytes)              |
+-----------------------------------+
```
* The payload length matches the exact byte boundary of the sliced container log line (with trailing newlines removed).
* Senders can construct this batch by direct sequential memory copies from the file read buffer.
