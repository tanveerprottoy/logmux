pub mod cri;
pub mod json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLog<'a> {
    pub timestamp_ns: i64,
    pub stream: StreamType,
    pub is_partial: bool,
    pub payload: &'a [u8],
}

/// A parser trait that extracts log attributes from a raw buffer without allocation.
pub trait LogParser {
    fn parse<'a>(&self, line: &'a [u8]) -> Option<ParsedLog<'a>>;
}

// Helper to convert Date/Time components to Unix timestamp in nanoseconds.
pub fn utc_to_nanoseconds(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    min: u32,
    sec: u32,
    nsec: u32,
) -> i64 {
    let mut days = 0;
    
    // Accumulate days from year 1970 to year-1
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    
    // Days in current year up to month-1
    let month_days = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += month_days[m as usize];
        if m == 2 && is_leap_year(year) {
            days += 1;
        }
    }
    
    days += (day as i64) - 1;
    
    let seconds = days * 86400 
        + (hour as i64) * 3600 
        + (min as i64) * 60 
        + (sec as i64);
        
    seconds * 1_000_000_000 + (nsec as i64)
}

#[inline]
fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Zero-copy RFC3339 timestamp parser to Unix nanoseconds.
pub fn parse_rfc3339_timestamp(bytes: &[u8]) -> Option<i64> {
    if bytes.len() < 20 {
        return None;
    }

    let parse_digits = |b: &[u8]| -> Option<u32> {
        let mut val = 0u32;
        for &x in b {
            if !x.is_ascii_digit() {
                return None;
            }
            val = val * 10 + (x - b'0') as u32;
        }
        Some(val)
    };

    let year = parse_digits(&bytes[0..4])? as i32;
    if bytes[4] != b'-' { return None; }
    let month = parse_digits(&bytes[5..7])? as u32;
    if bytes[7] != b'-' { return None; }
    let day = parse_digits(&bytes[8..10])? as u32;
    if bytes[10] != b'T' { return None; }
    let hour = parse_digits(&bytes[11..13])? as u32;
    if bytes[13] != b':' { return None; }
    let min = parse_digits(&bytes[14..16])? as u32;
    if bytes[16] != b':' { return None; }
    let sec = parse_digits(&bytes[17..19])? as u32;

    let mut nsec = 0u32;
    let mut idx = 19;

    if idx < bytes.len() && bytes[idx] == b'.' {
        idx += 1;
        let start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        let len = idx - start;
        if len > 0 {
            let mut val = 0u32;
            for i in 0..len {
                val = val * 10 + (bytes[start + i] - b'0') as u32;
            }
            if len <= 9 {
                let multiplier = 10u32.pow(9 - len as u32);
                nsec = val * multiplier;
            } else {
                nsec = val / 10u32.pow(len as u32 - 9);
            }
        }
    }

    let mut offset_sec = 0i64;
    if idx < bytes.len() {
        if bytes[idx] == b'+' || bytes[idx] == b'-' {
            let is_negative = bytes[idx] == b'-';
            if idx + 5 < bytes.len() {
                let h_offset = parse_digits(&bytes[idx+1..idx+3])? as i64;
                let m_offset = parse_digits(&bytes[idx+4..idx+6])? as i64;
                offset_sec = h_offset * 3600 + m_offset * 60;
                if is_negative {
                    offset_sec = -offset_sec;
                }
            }
        }
    }

    let ts = utc_to_nanoseconds(year, month, day, hour, min, sec, nsec);
    Some(ts - offset_sec * 1_000_000_000)
}

