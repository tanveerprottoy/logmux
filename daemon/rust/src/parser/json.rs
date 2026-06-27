use crate::parser::{LogParser, ParsedLog, StreamType, parse_rfc3339_timestamp};

pub struct JsonParser;

impl JsonParser {
    pub fn new() -> Self {
        Self
    }

    // Helper to find the value of a string field in a simple flat JSON block.
    // Example: Searching for `"log"` in `{"log":"hello\n","stream":"stdout"}`
    fn find_string_field<'a>(&self, json: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
        let mut pos = 0;
        let key_len = key.len();
        
        while pos + key_len <= json.len() {
            if &json[pos..pos + key_len] == key {
                // Confirm it's wrapped in quotes (e.g. "log") to prevent partial matches
                let has_leading_quote = pos > 0 && json[pos - 1] == b'"';
                let has_trailing_quote = pos + key_len < json.len() && json[pos + key_len] == b'"';
                
                if has_leading_quote && has_trailing_quote {
                    // Match found. Skip key and closing quote, look for ':'
                    let mut search_pos = pos + key_len + 1;
                    while search_pos < json.len() && json[search_pos] != b':' {
                        search_pos += 1;
                    }
                    if search_pos >= json.len() {
                        return None;
                    }
                    search_pos += 1; // skip ':'
                    
                    // Look for opening quote '"' of the string value
                    while search_pos < json.len() && json[search_pos] != b'"' {
                        search_pos += 1;
                    }
                    if search_pos >= json.len() {
                        return None;
                    }
                    search_pos += 1; // skip '"'
                    let val_start = search_pos;
                    
                    // Scan for the closing quote, skipping escaped characters like \"
                    while search_pos < json.len() {
                        if json[search_pos] == b'"' {
                            return Some(&json[val_start..search_pos]);
                        } else if json[search_pos] == b'\\' {
                            search_pos += 2;
                        } else {
                            search_pos += 1;
                        }
                    }
                    return None;
                }
            }
            pos += 1;
        }
        None
    }
}

impl LogParser for JsonParser {
    fn parse<'a>(&self, line: &'a [u8]) -> Option<ParsedLog<'a>> {
        // Extract the log string
        let raw_payload = self.find_string_field(line, b"log")?;
        
        // Extract the stream value
        let stream_bytes = self.find_string_field(line, b"stream")?;
        let stream = match stream_bytes {
            b"stdout" => StreamType::Stdout,
            b"stderr" => StreamType::Stderr,
            _ => return None,
        };

        // Extract the timestamp value
        let ts_bytes = self.find_string_field(line, b"time")?;
        let timestamp_ns = parse_rfc3339_timestamp(ts_bytes)?;

        // Trim trailing newlines from log payload if present
        let mut payload = raw_payload;
        
        // Note: Docker logs payload inside JSON might contain literal backslash-n (\n) 
        // as two bytes b'\\' and b'n' or actual bytes b'\n'. We trim both.
        while !payload.is_empty() && (payload[payload.len() - 1] == b'\n' || payload[payload.len() - 1] == b'\r') {
            payload = &payload[..payload.len() - 1];
        }
        
        // Also trim literal escaped "\n" or "\r" at the end of the JSON string
        if payload.len() >= 2 && payload[payload.len() - 2] == b'\\' && payload[payload.len() - 1] == b'n' {
            payload = &payload[..payload.len() - 2];
        }
        if payload.len() >= 2 && payload[payload.len() - 2] == b'\\' && payload[payload.len() - 1] == b'r' {
            payload = &payload[..payload.len() - 2];
        }

        Some(ParsedLog {
            timestamp_ns,
            stream,
            is_partial: false,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::utc_to_nanoseconds;

    #[test]
    fn test_json_parser_success() {
        let parser = JsonParser::new();
        let log_line = b"{\"log\":\"hello world\\n\",\"stream\":\"stdout\",\"time\":\"2026-06-27T20:13:09.123456789Z\"}";
        let parsed = parser.parse(log_line).unwrap();

        assert_eq!(parsed.stream, StreamType::Stdout);
        assert_eq!(parsed.is_partial, false);
        assert_eq!(parsed.payload, b"hello world");
        
        let expected_ts = utc_to_nanoseconds(2026, 6, 27, 20, 13, 9, 123456789);
        assert_eq!(parsed.timestamp_ns, expected_ts);
    }

    #[test]
    fn test_json_parser_different_order() {
        let parser = JsonParser::new();
        let log_line = b"{\"time\":\"2026-06-27T20:13:09.123Z\",\"stream\":\"stderr\",\"log\":\"error log\"}";
        let parsed = parser.parse(log_line).unwrap();

        assert_eq!(parsed.stream, StreamType::Stderr);
        assert_eq!(parsed.payload, b"error log");
        let expected_ts = utc_to_nanoseconds(2026, 6, 27, 20, 13, 9, 123000000);
        assert_eq!(parsed.timestamp_ns, expected_ts);
    }

    #[test]
    fn test_json_parser_malformed() {
        let parser = JsonParser::new();
        assert!(parser.parse(b"{\"stream\":\"stdout\"}").is_none());
        assert!(parser.parse(b"bad json content").is_none());
    }
}
