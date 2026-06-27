use std::os::raw::c_void;

// Inotify event masks
pub const IN_MODIFY: u32 = 0x0000_0002;
pub const IN_MOVED_FROM: u32 = 0x0000_0040;
pub const IN_MOVED_TO: u32 = 0x0000_0080;
pub const IN_CREATE: u32 = 0x0000_0100;
pub const IN_DELETE: u32 = 0x0000_0200;
pub const IN_DELETE_SELF: u32 = 0x0000_0400;
pub const IN_MOVE_SELF: u32 = 0x0000_0800;

// Inotify init flags
pub const IN_NONBLOCK: i32 = 0x0000_0800;
pub const IN_CLOEXEC: i32 = 0x0008_0000;

// Poll event flags
pub const POLLIN: i16 = 0x0001;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PollFd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
}

#[repr(C)]
pub struct InotifyEventHeader {
    pub wd: i32,
    pub mask: u32,
    pub cookie: u32,
    pub len: u32,
    // Followed by `len` bytes representing the name (null-terminated, padded)
}

extern "C" {
    pub fn inotify_init1(flags: i32) -> i32;
    pub fn inotify_add_watch(fd: i32, pathname: *const u8, mask: u32) -> i32;
    pub fn inotify_rm_watch(fd: i32, wd: i32) -> i32;
    pub fn poll(fds: *mut PollFd, nfds: usize, timeout: i32) -> i32;
    pub fn read(fd: i32, buf: *mut c_void, count: usize) -> isize;
    pub fn close(fd: i32) -> i32;
}
