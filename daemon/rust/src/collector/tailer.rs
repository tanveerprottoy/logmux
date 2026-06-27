use std::collections::HashMap;
use std::fs::{self, File};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::ffi::CString;

use crate::collector::sys;
use crate::parser::{cri::CriParser, json::JsonParser, LogParser, ParsedLog};

pub enum TailerMessage<'a> {
    Event(ParsedLog<'a>),
    Flush,
}

pub struct LogFile {
    pub file: File,
    pub path: PathBuf,
    pub wd: i32,
    pub offset: u64,
    pub line_buf: Vec<u8>,
}

pub struct LogTailer {
    inotify_fd: i32,
    log_dir: PathBuf,
    watch_to_path: HashMap<i32, PathBuf>,
    path_to_watch: HashMap<PathBuf, i32>,
    active_files: HashMap<PathBuf, LogFile>,
    cri_parser: CriParser,
    json_parser: JsonParser,
}

impl LogTailer {
    pub fn new<P: AsRef<Path>>(log_dir: P) -> std::io::Result<Self> {
        let fd = unsafe { sys::inotify_init1(sys::IN_NONBLOCK | sys::IN_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self {
            inotify_fd: fd,
            log_dir: log_dir.as_ref().to_path_buf(),
            watch_to_path: HashMap::new(),
            path_to_watch: HashMap::new(),
            active_files: HashMap::new(),
            cri_parser: CriParser::new(),
            json_parser: JsonParser::new(),
        })
    }

    /// Add a watch to a directory or a file
    fn add_watch(&mut self, path: &Path, mask: u32) -> std::io::Result<i32> {
        if let Some(&wd) = self.path_to_watch.get(path) {
            return Ok(wd);
        }

        let path_str = path.to_string_lossy();
        let c_path = CString::new(path_str.as_bytes()).unwrap();
        
        let wd = unsafe {
            sys::inotify_add_watch(self.inotify_fd, c_path.as_ptr() as *const u8, mask)
        };

        if wd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        self.watch_to_path.insert(wd, path.to_path_buf());
        self.path_to_watch.insert(path.to_path_buf(), wd);
        Ok(wd)
    }

    /// Remove a watch
    fn remove_watch(&mut self, path: &Path) {
        if let Some(&wd) = self.path_to_watch.get(path) {
            unsafe {
                sys::inotify_rm_watch(self.inotify_fd, wd);
            }
            self.watch_to_path.remove(&wd);
            self.path_to_watch.remove(path);
        }
    }

    /// Initialize by scanning the base directory recursively and setting up watches
    pub fn initialize(&mut self) -> std::io::Result<()> {
        let base_path = self.log_dir.clone();
        
        // Ensure log directory exists
        if !base_path.exists() {
            fs::create_dir_all(&base_path)?;
        }

        println!("Initializing watches starting from root: {:?}", base_path);
        self.scan_directory_recursive(&base_path)?;
        Ok(())
    }

    fn scan_directory_recursive(&mut self, dir: &Path) -> std::io::Result<()> {
        // Watch this directory for creations and deletions
        let dir_mask = sys::IN_CREATE | sys::IN_DELETE | sys::IN_MOVED_FROM | sys::IN_MOVED_TO;
        self.add_watch(dir, dir_mask)?;

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;

            if file_type.is_dir() {
                self.scan_directory_recursive(&path)?;
            } else if file_type.is_file() {
                if path.extension().map_or(false, |ext| ext == "log") {
                    self.start_tailing_file(&path, false)?;
                }
            }
        }
        Ok(())
    }

    fn start_tailing_file(&mut self, path: &Path, from_beginning: bool) -> std::io::Result<()> {
        if self.active_files.contains_key(path) {
            return Ok(());
        }

        println!("Opening log file for tailing: {:?}", path);
        let file = File::open(path)?;
        let metadata = file.metadata()?;

        let file_mask = sys::IN_MODIFY | sys::IN_DELETE_SELF | sys::IN_MOVE_SELF;
        let wd = self.add_watch(path, file_mask)?;

        let offset = if from_beginning { 0 } else { metadata.len() };

        let log_file = LogFile {
            file,
            path: path.to_path_buf(),
            wd,
            offset,
            line_buf: Vec::new(),
        };

        self.active_files.insert(path.to_path_buf(), log_file);
        Ok(())
    }

    /// Read any new writes to the tracked log file and process lines.
    /// The callback returns false when downstream backpressure is encountered.
    /// Uncommitted lines remain in line_buf and the file offset is not advanced past them.
    fn read_file_deltas<F>(&mut self, path: &Path, callback: &mut F) -> bool
    where
        F: FnMut(&Path, TailerMessage<'_>) -> bool,
    {
        let mut can_continue = true;

        if let Some(log_file) = self.active_files.get_mut(path) {
            let metadata = match log_file.file.metadata() {
                Ok(m) => m,
                Err(_) => return callback(path, TailerMessage::Flush),
            };

            let current_len = metadata.len();
            if current_len < log_file.offset {
                println!("File truncated, resetting offset: {:?}", path);
                log_file.offset = 0;
                log_file.line_buf.clear();
            }

            let mut read_buf = vec![0u8; 64 * 1024];
            loop {
                if !can_continue {
                    break;
                }

                let read_at = log_file.offset + log_file.line_buf.len() as u64;
                match log_file.file.read_at(&mut read_buf, read_at) {
                    Ok(0) => break,
                    Ok(n) => {
                        log_file.line_buf.extend_from_slice(&read_buf[..n]);

                        while let Some(pos) = log_file.line_buf.iter().position(|&b| b == b'\n') {
                            let line = &log_file.line_buf[..pos];

                            let parsed = if line.first().map_or(false, |&b| b == b'{') {
                                self.json_parser.parse(line)
                            } else {
                                self.cri_parser.parse(line)
                            };

                            if let Some(event) = parsed {
                                if !callback(&log_file.path, TailerMessage::Event(event)) {
                                    can_continue = false;
                                    break;
                                }
                            }

                            log_file.line_buf.drain(..pos + 1);
                            log_file.offset += (pos + 1) as u64;
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        }

        if can_continue {
            can_continue = callback(path, TailerMessage::Flush);
        }

        can_continue
    }

    /// Primary execution loop. Blocks on the inotify file descriptor.
    pub fn run<F>(&mut self, mut callback: F) -> std::io::Result<()>
    where
        F: FnMut(&Path, TailerMessage<'_>) -> bool,
    {
        let mut poll_fds = [sys::PollFd {
            fd: self.inotify_fd,
            events: sys::POLLIN,
            revents: 0,
        }];

        println!("Starting event loop...");

        loop {
            // Block indefinitely until an event arrives
            let ret = unsafe { sys::poll(poll_fds.as_mut_ptr(), 1, -1) };
            if ret < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }

            if poll_fds[0].revents & sys::POLLIN != 0 {
                let mut buf = [0u8; 8192];
                let bytes_read = unsafe {
                    sys::read(
                        self.inotify_fd,
                        buf.as_mut_ptr() as *mut _,
                        buf.len(),
                    )
                };

                if bytes_read < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() == std::io::ErrorKind::WouldBlock {
                        continue;
                    }
                    return Err(err);
                }

                let mut pos = 0usize;
                while pos < bytes_read as usize {
                    let event = unsafe {
                        &*(buf.as_ptr().add(pos) as *const sys::InotifyEventHeader)
                    };

                    let name = if event.len > 0 {
                        let name_ptr = unsafe {
                            buf.as_ptr().add(
                                pos + std::mem::size_of::<sys::InotifyEventHeader>(),
                            )
                        };
                        let mut name_len = 0;
                        while name_len < event.len as usize && unsafe { *name_ptr.add(name_len) } != 0 {
                            name_len += 1;
                        }
                        std::str::from_utf8(unsafe {
                            std::slice::from_raw_parts(name_ptr, name_len)
                        })
                        .ok()
                    } else {
                        None
                    };

                    if !self.process_event(event.wd, event.mask, name, &mut callback) {
                        // Downstream backpressure: pause ingestion until transport recovers.
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    pos += std::mem::size_of::<sys::InotifyEventHeader>() + event.len as usize;
                }
            }
        }
    }

    fn process_event<F>(
        &mut self,
        wd: i32,
        mask: u32,
        name: Option<&str>,
        callback: &mut F,
    ) -> bool
    where
        F: FnMut(&Path, TailerMessage<'_>) -> bool,
    {
        let parent_path = match self.watch_to_path.get(&wd) {
            Some(p) => p.clone(),
            None => return true,
        };

        if self.active_files.contains_key(&parent_path) {
            if mask & sys::IN_MODIFY != 0 {
                if !self.read_file_deltas(&parent_path, callback) {
                    return false;
                }
            }
            if mask & (sys::IN_DELETE_SELF | sys::IN_MOVE_SELF) != 0 {
                println!("File rotated or deleted: {:?}", parent_path);
                self.remove_watch(&parent_path);
                self.active_files.remove(&parent_path);
            }
            return true;
        }

        if let Some(file_name) = name {
            let target_path = parent_path.join(file_name);

            if mask & (sys::IN_CREATE | sys::IN_MOVED_TO) != 0 {
                if let Ok(metadata) = fs::metadata(&target_path) {
                    if metadata.is_dir() {
                        println!("New subdirectory discovered: {:?}", target_path);
                        let _ = self.scan_directory_recursive(&target_path);
                    } else if metadata.is_file() && target_path.extension().map_or(false, |ext| ext == "log") {
                        println!("New log file discovered: {:?}", target_path);
                        let _ = self.start_tailing_file(&target_path, true);
                        if !self.read_file_deltas(&target_path, callback) {
                            return false;
                        }
                    }
                }
            } else if mask & (sys::IN_DELETE | sys::IN_MOVED_FROM) != 0 {
                println!("Path removed or moved away: {:?}", target_path);
                self.remove_watch(&target_path);
                self.active_files.remove(&target_path);
            }
        }

        true
    }
}

impl Drop for LogTailer {
    fn drop(&mut self) {
        unsafe {
            sys::close(self.inotify_fd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use crate::parser::StreamType;

    #[test]
    fn test_tailer_scan_and_read() {
        // 1. Create a temp directory
        let temp_dir = std::env::temp_dir().join("logmux_test_tailer");
        if temp_dir.exists() {
            let _ = fs::remove_dir_all(&temp_dir);
        }
        fs::create_dir_all(&temp_dir).unwrap();

        // 2. Create a log file with some CRI content
        let log_path = temp_dir.join("container.log");
        {
            let mut f = File::create(&log_path).unwrap();
            writeln!(f, "2026-06-27T20:13:09.123456789Z stdout F initial log line").unwrap();
        }

        // 3. Initialize tailer
        let mut tailer = LogTailer::new(&temp_dir).unwrap();
        tailer.initialize().unwrap();

        // Check if file is tracked
        assert!(tailer.active_files.contains_key(&log_path));
        let active_file = tailer.active_files.get(&log_path).unwrap();
        assert!(active_file.offset > 0);

        // 4. Append more logs to the file
        {
            let mut f = fs::OpenOptions::new().append(true).open(&log_path).unwrap();
            writeln!(f, "2026-06-27T20:13:10.111222333Z stderr F second log line").unwrap();
        }

        // 5. Trigger read_file_deltas manually and collect events
        let mut events = Vec::new();
        tailer.read_file_deltas(
            &log_path,
            &mut |path, message| {
                if let TailerMessage::Event(event) = message {
                    events.push((path.to_path_buf(), event.stream, event.payload.to_vec()));
                }
                true
            },
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, log_path);
        assert_eq!(events[0].1, StreamType::Stderr);
        assert_eq!(events[0].2, b"second log line");

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
