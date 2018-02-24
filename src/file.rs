//! Asynchronous File I/O module for Tokio
//!
//! This module provides methods for asynchronous file I/O.  On BSD-based
//! operating systems, it uses mio-aio.  On Linux, it can use libaio.

use bytes::{Bytes, BytesMut};
use libc::{off_t};
use futures::{Async, Future, Poll};
use mio::unix::UnixReady;
use mio_aio;
use nix::sys::aio;
use nix;
use tokio::reactor::{Handle, PollEvented};
use std::{fs, io, mem};
use std::os::unix::io::AsRawFd;
use std::os::unix::io::RawFd;
use std::path::Path;

#[derive(Debug)]
enum AioOp {
    Fsync(PollEvented<mio_aio::AioCb<'static>>),
    Read(PollEvented<mio_aio::AioCb<'static>>),
    Write(PollEvented<mio_aio::AioCb<'static>>),
}

/// Represents the progress of a single AIO operation
#[derive(Debug)]
enum AioState {
    /// The AioFut has been allocated, but not submitted to the system
    Allocated,
    /// The AioFut has been submitted, ie with `write_at`, and is currently in
    /// progress, but its status has not been returned with `aio_return`
    InProgress,
}

/// A Future representing an AIO operation.
#[must_use = "futures do nothing unless polled"]
#[derive(Debug)]
pub struct AioFut {
    op: AioOp,
    state: AioState,
}

impl AioFut {
    // Used internally by `futures::Future::poll`.  Should not be called by the
    // user.
    #[doc(hidden)]
    fn aio_return(&mut self) -> Result<Option<isize>, nix::Error> {
        match self.op {
            AioOp::Fsync(ref io) =>
                io.get_ref().aio_return().map(|_| None),
            AioOp::Read(ref io) =>
                io.get_ref().aio_return().map(|x| Some(x)),
            AioOp::Write(ref io) =>
                io.get_ref().aio_return().map(|x| Some(x)),
            //AioOp::Lio(ref mut io) => {
                //let r = io.get_mut()
                //.iter_mut()
                //.fold(0, |acc, res| {
                    //acc + res.aio_return().unwrap()
                //});
                //Ok(Some(r))
            //}
        }
    }
}

/// Stores the buffer used by the underlying `AioCb`, if any.
///
/// For read operations, this will usually be the only reference to the buffer
pub enum BufRef {
    /// Either the `AioCb` has no buffer, as for an fsync operation, or a
    /// reference can't be stored, as when constructed from a slice.
    None,
    /// Immutable shared ownership `Bytes` object
    Bytes(Bytes),
    /// Mutable uniquely owned `BytesMut` object
    BytesMut(BytesMut)
}

impl BufRef {
    /// Return the inner `Bytes`, if any
    pub fn into_bytes(self) -> Option<Bytes> {
        match self {
            BufRef::Bytes(x) => Some(x),
            _ => None
        }
    }

    /// Return the inner `BytesMut`, if any
    pub fn into_bytes_mut(self) -> Option<BytesMut> {
        match self {
            BufRef::BytesMut(x) => Some(x),
            _ => None
        }
    }

    /// Is this `BufRef` `None`?
    pub fn is_none(&self) -> bool {
        match self {
            &BufRef::None => true,
            _ => false,
        }
    }
}

/// Holds the result of an individual aio or lio operation
pub struct AioResult {
    /// This is what the AIO operation would've returned, had it been
    /// synchronous.  fsync operations return `()`, read and write operations
    /// return an `isize`
    pub value: Option<isize>,

    /// Optionally, return ownership of the buffer that was used to create the
    /// AIO operation.
    pub buf: BufRef
}

impl AioResult {
    pub fn into_buf_ref(self) -> BufRef {
        self.buf
    }
}

/// A Future representing an LIO operation.
#[must_use = "futures do nothing unless polled"]
#[derive(Debug)]
pub struct LioFut {
    op: Option<PollEvented<mio_aio::LioCb<'static>>>,
    state: AioState,
}

impl Future for LioFut {
    type Item = Box<Iterator<Item = AioResult>>;
    type Error = nix::Error;

    // perhaps should return an iterator instead of a vec?
    fn poll(&mut self) -> Poll<Box<Iterator<Item = AioResult>>, nix::Error> {
        if let AioState::Allocated = self.state {
            self.op.as_mut().unwrap().get_mut()
                .listio().expect("mio_aio::listio");
            self.state = AioState::InProgress;
        }
        let poll_result = self.op.as_mut().unwrap().poll_ready(UnixReady::lio().into());
        if poll_result == Async::NotReady {
            return Ok(Async::NotReady);
        }
        let mut op = None;
        mem::swap(&mut op, &mut self.op);
        let liocb : mio_aio::LioCb<'static> = op.unwrap().into_inner();
        let iter = Box::new(liocb.into_aiocbs().map(|aiocb| {
            // TODO: handle the error case
            let value = aiocb.aio_return().unwrap();
            let buf = match aiocb.into_buf_ref() {
                mio_aio::BufRef::None => BufRef::None,
                mio_aio::BufRef::Bytes(x) => BufRef::Bytes(x),
                mio_aio::BufRef::BytesMut(x) => BufRef::BytesMut(x),
            };
            AioResult{value: Some(value), buf: buf}
        }));
        Ok(Async::Ready(iter))
    }
}

/// Basically a Tokio file handle
#[derive(Debug)]
pub struct File {
    file: fs::File,
    handle: Handle
}

/// Base trait of objects that can be passed to File::write_at
///
/// Tokio-file consumers should never call these methods directly.
pub trait WriteAtable {
    fn emplace(&self, liocb: &mut mio_aio::LioCb, fd: RawFd, offs: off_t);
    fn length(&self) -> usize;
    fn to_aiocb(self, fd: RawFd, offs: off_t) -> mio_aio::AioCb<'static>;
}

impl<'a> WriteAtable for Bytes {
    fn emplace(&self, liocb: &mut mio_aio::LioCb, fd: RawFd, offs: off_t) {
        liocb.emplace_bytes(fd, offs, self.clone(), 0,
                            mio_aio::LioOpcode::LIO_WRITE);
    }

    fn length(&self) -> usize {
        self.len()
    }

    fn to_aiocb(self, fd: RawFd, offs: off_t) -> mio_aio::AioCb<'static> {
        mio_aio::AioCb::from_bytes(fd,
            offs,
            self.clone(),
            0,  //priority
            aio::LioOpcode::LIO_NOP)
    }
}

impl WriteAtable for &'static [u8] {
    fn emplace(&self, liocb: &mut mio_aio::LioCb, fd: RawFd, offs: off_t) {
        liocb.emplace_slice(fd, offs, self, 0, mio_aio::LioOpcode::LIO_WRITE);
    }

    fn length(&self) -> usize {
        self.len()
    }

    fn to_aiocb(self, fd: RawFd, offs: off_t) -> mio_aio::AioCb<'static> {
        mio_aio::AioCb::from_slice(fd,
            offs,
            self,
            0,  //priority
            aio::LioOpcode::LIO_NOP)
    }
}

impl File {
    /// Get metadata from the underlying file
    ///
    /// POSIX AIO doesn't provide a way to do this asynchronously, so it must be
    /// synchronous.
    pub fn metadata(&self) -> io::Result<fs::Metadata> {
        self.file.metadata()
    }

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

    /// Asynchronous equivalent of `std::fs::File::read_at`
    pub fn read_at(&self, buf: BytesMut, offset: off_t) -> io::Result<AioFut> {
        let aiocb = mio_aio::AioCb::from_bytes_mut(self.file.as_raw_fd(),
                            offset,  //offset
                            buf,
                            0,  //priority
                            aio::LioOpcode::LIO_NOP);
        Ok(AioFut{
            op: AioOp::Read(try!(PollEvented::new(aiocb, &self.handle))),
            state: AioState::Allocated })
    }

    /// Asynchronous equivalent of `preadv`
    ///
    /// Similar to
    /// [preadv(2)](https://www.freebsd.org/cgi/man.cgi?query=read&sektion=2)
    /// but asynchronous.  Reads a contiguous portion of a file into a
    /// scatter-gather list of buffers.  Unlike `preadv`, there is no guarantee
    /// of overall atomicity.  Each scatter gather element's contents could
    /// reflect the state of the file at a different point in time.
    ///
    /// # Parameters
    ///
    /// - `bufs`:   The destination for the read.  A scatter-gather list of
    ///             buffers.
    /// - `offset`: Offset within the file at which to begin the read
    ///
    /// # Returns
    ///
    /// `Ok(x)`:    The operation was successfully issued.  The future
    ///             will eventually return the final status of the operation.
    ///             If the operation was partially successful, the future will
    ///             return an error with no indication of which parts of `bufs`
    ///             are valid.
    /// `Err(x)`:   An error occurred before issueing the operation.  The result
    ///             may be `drop`ped.
    pub fn readv_at(&self, mut bufs: Vec<BytesMut>, offset: off_t) -> io::Result<LioFut> {
        let mut liocb = mio_aio::LioCb::with_capacity(bufs.len());
        let mut offs = offset;
        for buf in bufs.drain(..) {
            let buflen = buf.len();
            liocb.emplace_bytes_mut(self.file.as_raw_fd(), offs, buf,
                                      0, mio_aio::LioOpcode::LIO_READ);
            offs += buflen as off_t;
        };
        Ok(LioFut{
            op: Some(try!(PollEvented::new(liocb, &self.handle))),
            state: AioState::Allocated })
    }

    /// Asynchronous equivalent of `std::fs::File::write_at`
    pub fn write_at<T: WriteAtable>(&self, buf: T, offset: off_t) -> io::Result<AioFut> {
        let aiocb = buf.to_aiocb(self.file.as_raw_fd(), offset);
        Ok(AioFut{
            op: AioOp::Write(try!(PollEvented::new(aiocb, &self.handle))),
            state: AioState::Allocated })
    }

    /// Asynchronous equivalent of `pwritev`
    pub fn writev_at<T: WriteAtable>(&self, bufs: &[T], offset: off_t) -> io::Result<LioFut> {
        let mut liocb = mio_aio::LioCb::with_capacity(bufs.len());
        let mut offs = offset;
        for buf in bufs {
            buf.emplace(&mut liocb, self.file.as_raw_fd(), offs);
            offs += buf.length() as off_t;
        };

        Ok(LioFut{
            op: Some(try!(PollEvented::new(liocb, &self.handle))),
            state: AioState::Allocated })
    }

    /// Asynchronous equivalent of `std::fs::File::sync_all`
    // TODO: add sync_all_data, for supported operating systems
    pub fn sync_all(&self) -> io::Result<AioFut> {
        let aiocb = mio_aio::AioCb::from_fd(self.file.as_raw_fd(),
                            0,  //priority
                            );
        Ok(AioFut{
            op: AioOp::Fsync(try!(PollEvented::new(aiocb, &self.handle))),
            state: AioState::Allocated })
    }
}

impl Future for AioFut {
    type Item = AioResult;
    type Error = nix::Error;

    fn poll(&mut self) -> Poll<AioResult, nix::Error> {
        if let AioState::Allocated = self.state {
            let _ = match self.op {
                AioOp::Fsync(ref pe) => pe.get_ref()
                    .fsync(aio::AioFsyncMode::O_SYNC).expect("mio_aio::fsync"),
                AioOp::Read(ref pe) => pe.get_ref()
                    .read().expect("mio_aio::read"),
                AioOp::Write(ref pe) => pe.get_ref()
                    .write().expect("mio_aio::write"),
            };
            self.state = AioState::InProgress;
        }
        let poll_result = match self.op {
                AioOp::Fsync(ref mut io) =>
                    io.poll_ready(UnixReady::aio().into()),
                AioOp::Read(ref mut io) =>
                    io.poll_ready(UnixReady::aio().into()),
                AioOp::Write(ref mut io) =>
                    io.poll_ready(UnixReady::aio().into()),
                //AioOp::Lio(ref mut io) =>
                    //io.poll_ready(UnixReady::lio().into())
        };
        if poll_result == Async::NotReady {
            return Ok(Async::NotReady);
        }
        let buf = {
            let mut op = match self.op {
                AioOp::Fsync(ref mut op) => op,
                AioOp::Read(ref mut op) => op,
                AioOp::Write(ref mut op) => op,
                //AioOp::Lio(_) => panic!("TODO")
            };
            let aiocb_ref = op.get_mut();
            let mio_bufref = unsafe { aiocb_ref.buf_ref() };
            match mio_bufref {
                mio_aio::BufRef::None => BufRef::None,
                mio_aio::BufRef::Bytes(x) => BufRef::Bytes(x),
                mio_aio::BufRef::BytesMut(x) => BufRef::BytesMut(x),
            }
        };
        match self.aio_return() {
            Ok(x) => Ok(Async::Ready(AioResult{value: x, buf: buf})),
            Err(x) => Err(x)
        }
    }
}
