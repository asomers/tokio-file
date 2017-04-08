//! Asynchronous File I/O module for Tokio
//!
//! This module provides methods for asynchronous file I/O.  On BSD-based
//! operating systems, it uses mio-aio.  On Linux, it can use libaio.

use libc::{off_t};
use futures::{Async, Future, Poll};
use mio_aio;
use nix::sys::aio;
use nix;
use tokio_core::reactor::{Handle, PollEvented};
use std::fs;
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::rc::Rc;

enum AioOpcode {
    Fsync,
    Read,
    Write
}

/// Represents the progress of a single AIO operation
enum AioState {
    /// The AioFut has been allocated, but not submitted to the system
    Allocated,
    /// The AioFut has been submitted, ie with `write_at`, and is currently in
    /// progress, but its status has not been returned with `aio_return`
    InProgress,
    // The AioFut is completed, its final status has been retrieved, and the
    // operating system is no longer aware of it.
    //Complete,
}

/// A Future representing an AIO operation.  `T` is the type that would be
/// returned by the underlying operation if it were synchronous.
#[must_use = "futures do nothing unless polled"]
pub struct AioReadFut<'a> {
    io: PollEvented<mio_aio::AioCb<'a>>,
    op: AioOpcode,
    state: AioState,
}

pub struct AioWriteFut<'a> {
    io: PollEvented<mio_aio::AioCb<'a>>,
    op: AioOpcode,
    state: AioState,
}

pub struct AioSyncFut<'a> {
    io: PollEvented<mio_aio::AioCb<'a>>,
    op: AioOpcode,
    state: AioState,
}

impl<'a> AioReadFut<'a> {
    fn aio_return(&self) -> Result<isize, nix::Error> {
        self.io.get_ref().aio_return().map(|x| x)
    }
}

impl<'a> AioSyncFut<'a> {
    fn aio_return(&self) -> Result<(), nix::Error> {
        self.io.get_ref().aio_return().map(|_| ())
    }
}

impl<'a> AioWriteFut<'a> {
    fn aio_return(&self) -> Result<isize, nix::Error> {
        self.io.get_ref().aio_return().map(|x| x)
    }
}

/// Basically a Tokio file handle
pub struct File {
    file: fs::File,
    handle: Handle
}

impl File {
    /// Open a new Tokio file
    // Technically, sfd::fs::File::open can block, so we should make a
    // nonblocking File::open method and have it return a Future.  That's what
    // Seastar does.  But POSIX AIO doesn't have any kind of asynchronous open
    // function, so there's no straightforward way to implement such a method.
    // Instead, we'll block.
    pub fn open<P: AsRef<Path>>(path: P, h: Handle) -> io::Result<File> {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .map(|f| File {file: f, handle: h})
    }

    /// Asynchronous equivalent of std::fs::File::read_at
    pub fn read_at<'a>(&'a self, buf: Rc<Box<[u8]>>, offset: off_t) -> io::Result<AioReadFut<'a>> {
        let aiocb = mio_aio::AioCb::from_boxed_slice(self.file.as_raw_fd(),
                            offset,  //offset
                            buf,
                            0,  //priority
                            aio::LioOpcode::LIO_NOP);
        Ok(AioReadFut::<'a>{ io: try!(PollEvented::new(aiocb, &self.handle)),
                   op: AioOpcode::Read,
                   state: AioState::Allocated})
    }

    /// Asynchronous equivalent of std::fs::File::write_at
    // TODO: write a method to write a static buffer
    pub fn write_at(&self, buf: Rc<Box<[u8]>>, offset: off_t) -> io::Result<AioWriteFut> {
        let aiocb = mio_aio::AioCb::from_boxed_slice(self.file.as_raw_fd(),
                            offset,  //offset
                            buf,
                            0,  //priority
                            aio::LioOpcode::LIO_NOP);
        Ok(AioWriteFut{ io: try!(PollEvented::new(aiocb, &self.handle)),
                   op: AioOpcode::Write,
                   state: AioState::Allocated})
    }

    /// Asynchronous equivalent of std::fs::File::sync_all
    // TODO: add sync_all_data, for supported operating systems
    pub fn sync_all(&self) -> io::Result<AioSyncFut> {
        let aiocb = mio_aio::AioCb::from_fd(self.file.as_raw_fd(),
                            0,  //priority
                            );
        Ok(AioSyncFut{ io: try!(PollEvented::new(aiocb, &self.handle)),
                   op: AioOpcode::Fsync,
                   state: AioState::Allocated})
    }
}

impl<'a> Future for AioSyncFut<'a> {
    type Item = ();
    type Error = nix::Error;

    fn poll(&mut self) -> Poll<(), nix::Error> {
        if let AioState::Allocated = self.state {
                let _ = match self.op {
                    AioOpcode::Fsync => self.io.get_ref().fsync(aio::AioFsyncMode::O_SYNC),
                    AioOpcode::Read => self.io.get_ref().read(),
                    AioOpcode::Write => self.io.get_ref().write()
                };  // TODO: handle failure at this point
                self.state = AioState::InProgress;
        }
        if self.io.poll_aio() == Async::NotReady {
            return Ok(Async::NotReady);
        }
        match self.aio_return() {
            Ok(x) => Ok(Async::Ready(x)),
            Err(x) => Err(x)
        }
    }
}

impl<'a> Future for AioReadFut<'a> {
    type Item = isize;
    type Error = nix::Error;

    fn poll(&mut self) -> Poll<isize, nix::Error> {
        if let AioState::Allocated = self.state {
                let _ = match self.op {
                    AioOpcode::Fsync => self.io.get_ref().fsync(aio::AioFsyncMode::O_SYNC),
                    AioOpcode::Read => self.io.get_ref().read(),
                    AioOpcode::Write => self.io.get_ref().write()
                };  // TODO: handle failure at this point
                self.state = AioState::InProgress;
        }
        if self.io.poll_aio() == Async::NotReady {
            return Ok(Async::NotReady);
        }
        match self.aio_return() {
            Ok(x) => Ok(Async::Ready(x)),
            Err(x) => Err(x)
        }
    }
}

impl<'a> Future for AioWriteFut<'a> {
    type Item = isize;
    type Error = nix::Error;

    fn poll(&mut self) -> Poll<isize, nix::Error> {
        if let AioState::Allocated = self.state {
                let _ = match self.op {
                    AioOpcode::Fsync => self.io.get_ref().fsync(aio::AioFsyncMode::O_SYNC),
                    AioOpcode::Read => self.io.get_ref().read(),
                    AioOpcode::Write => self.io.get_ref().write()
                };  // TODO: handle failure at this point
                self.state = AioState::InProgress;
        }
        if self.io.poll_aio() == Async::NotReady {
            return Ok(Async::NotReady);
        }
        match self.aio_return() {
            Ok(x) => Ok(Async::Ready(x)),
            Err(x) => Err(x)
        }
    }
}
