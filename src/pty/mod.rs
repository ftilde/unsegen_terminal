// Adapted and modified from https://github.com/hibariya/pty-rs
//
// The MIT License (MIT)
//
// Copyright (c) 2015 Hika Hibariya
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

#![deny(unstable_features, unused_import_braces, unused_qualifications)]
#![cfg_attr(feature = "dev", allow(unstable_features))]
#![cfg_attr(feature = "dev", feature(plugin))]
#![cfg_attr(feature = "dev", plugin(clippy))]

use libc;
use nix::errno;
use nix::errno::Errno;
use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::result;

macro_rules! unsafe_try {
    ( $x:expr ) => {{
        let ret = unsafe { $x };

        if ret < 0 {
            return Err(last_error());
        } else {
            ret
        }
    }};
}

pub type Result<T> = result::Result<T, Errno>;

fn last_error() -> Errno {
    errno::from_i32(errno::errno())
}

/// A type representing a pty.
pub struct PTY {
    fd: libc::c_int,
}

use std::sync::{Arc, Mutex};
pub struct PTYInput {
    pty: Arc<Mutex<PTY>>,
}

pub struct PTYOutput {
    pty: Arc<Mutex<PTY>>,
}

impl PTY {
    pub fn open() -> Result<PTY> {
        open_ptm().map(|fd| PTY { fd: fd })
    }

    pub fn name(&self) -> &OsStr {
        // man ptsname:
        // "On success, ptsname() returns a pointer to a string in _static_ storage which
        // will be overwritten by subsequent calls. This pointer must not be freed."
        let pts_name = unsafe { libc::ptsname(self.fd) };

        // This should not happen, as fd is always valid from open to drop.
        if (pts_name as *const i32) == ::std::ptr::null() {
            panic!("ptsname failed. ({})", last_error());
        }

        let pts_name_cstr = unsafe { ::std::ffi::CStr::from_ptr(pts_name) };
        let pts_name_slice = pts_name_cstr.to_bytes();

        use std::os::unix::ffi::OsStrExt;
        OsStr::from_bytes(pts_name_slice)
    }

    pub fn split_io(self) -> (PTYInput, PTYOutput) {
        let read = Arc::new(Mutex::new(self));
        let write = read.clone();
        (PTYInput { pty: read }, PTYOutput { pty: write })
    }
}

impl Drop for PTY {
    fn drop(&mut self) {
        // There is no way to handle closing failure anyway.
        let _ = unsafe { libc::close(self.as_raw_fd()) };
    }
}

impl AsRawFd for PTY {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl Read for PTY {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        read(self.fd, buf)
    }
}

impl Write for PTY {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        write(self.fd, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for PTYOutput {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        //Panics while reading/writing should not happen
        let fd = self.pty.lock().expect("lock pty for read").fd;
        read(fd, buf)
    }
}

impl Write for PTYInput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        //Panics while reading/writing should not happen
        let fd = self.pty.lock().expect("lock pty for write").fd;
        write(fd, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl PTYInput {
    pub fn resize(&self, w: u16, h: u16, wpixel: u16, hpixel: u16) -> io::Result<()> {
        let size = libc::winsize {
            ws_row: h as libc::c_ushort,
            ws_col: w as libc::c_ushort,
            ws_xpixel: wpixel as libc::c_ushort,
            ws_ypixel: hpixel as libc::c_ushort,
        };

        let res = {
            let lock = self.pty.lock().expect("lock pty for resize");
            unsafe { libc::ioctl(lock.fd, libc::TIOCSWINSZ, &size as *const libc::winsize) }
        };

        if res < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

fn open_ptm() -> Result<libc::c_int> {
    let pty_master = unsafe_try!(libc::posix_openpt(libc::O_RDWR));

    unsafe_try!(libc::grantpt(pty_master));
    unsafe_try!(libc::unlockpt(pty_master));

    Ok(pty_master)
}

fn read(fd: libc::c_int, buf: &mut [u8]) -> io::Result<usize> {
    let nread = unsafe {
        libc::read(
            fd,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as usize,
        )
    };

    if nread < 0 {
        //Ok(0)
        //panic!("read: {:?}", io::Error::last_os_error());
        Err(io::Error::last_os_error())
    } else {
        Ok(nread as usize)
    }
}

fn write(fd: libc::c_int, buf: &[u8]) -> io::Result<usize> {
    let ret = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len() as usize) };

    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret as usize)
    }
}
