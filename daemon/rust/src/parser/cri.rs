use crate::parser::{LogParser, ParsedLog, StreamType, parse_rfc3339_timestamp};

pub struct CriParser;

impl CriParser {
    pub fn new() -> Self {
        Self
    }
}

impl LogParser for CriParser {
    fn parse<'a>(&self, line: &'a [u8]) -> Option<ParsedLog<'a>> {
        // Find first space (end of timestamp)
        let ts_end = line.iter().position(|&b| b == b' ')?;
        let ts_bytes = &line[0..ts_end];
        let timestamp_ns = parse_rfc3339_timestamp(ts_bytes)?;

        // Find second space (end of stream)
        let stream_start = ts_end + 1;
        if stream_start >= line.len() {
            return None;
        }
        let stream_len = line[stream_start..].iter().position(|&b| b == b' ')?;
        let stream_end = stream_start + stream_len;
        let stream_bytes = &line[stream_start..stream_end];

        let stream = match stream_bytes {
            b"stdout" => StreamType::Stdout,
            b"stderr" => StreamType::Stderr,
            _ => return None,
        };

        // Find third space (end of tag: 'F' or 'P')
        let tag_start = stream_end + 1;
        if tag_start >= line.len() {
            return None;
        }
        let tag_bytes = &line[tag_start..tag_start + 1];
        let is_partial = match tag_bytes[0] {
            b'F' => false,
            b'P' => true,
            _ => return None,
        };

        // The payload starts after the third space + tag character + space
        let payload_start = tag_start + 2;
        if payload_start > line.len() {
            return None;
        }

        let mut payload = &line[payload_start..];
        
        // Trim trailing newlines if present
        while !payload.is_empty() && (payload[payload.len() - 1] == b'\n' || payload[payload.len() - 1] == b'\r') {
            payload = &payload[..payload.len() - 1];
        }

        Some(ParsedLog {
            timestamp_ns,
            stream,
            is_partial,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::utc_to_nanoseconds;

    #[test]
    fn test_cri_parser_success() {
        let parser = CriParser::new();
        let log_line = b"2026-06-27T20:13:09.123456789Z stdout F hello world\n";
        let parsed = parser.parse(log_line).unwrap();

        assert_eq!(parsed.stream, StreamType::Stdout);
        assert_eq!(parsed.is_partial, false);
        assert_eq!(parsed.payload, b"hello world");
        
        let expected_ts = utc_to_nanoseconds(2026, 6, 27, 20, 13, 9, 123456789);
        assert_eq!(parsed.timestamp_ns, expected_ts);
    }

    #[test]
    fn test_cri_parser_partial() {
        let parser = CriParser::new();
        let log_line = b"2026-06-27T20:13:09.123Z stderr P part of a line";
        let parsed = parser.parse(log_line).unwrap();

        assert_eq!(parsed.stream, StreamType::Stderr);
        assert_eq!(parsed.is_partial, true);
        assert_eq!(parsed.payload, b"part of a line");
        let expected_ts = utc_to_nanoseconds(2026, 6, 27, 20, 13, 9, 123000000);
        assert_eq!(parsed.timestamp_ns, expected_ts);
    }

    #[test]
    fn test_cri_parser_malformed() {
        let parser = CriParser::new();
        assert!(parser.parse(b"bad timestamp stdout F log").is_none());
        assert!(parser.parse(b"2026-06-27T20:13:09.123Z bad_stream F log").is_none());
        assert!(parser.parse(b"2026-06-27T20:13:09.123Z stdout X log").is_none());
    }
}
