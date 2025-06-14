// vim: tw=80
// This lint isn't very helpful.  See
// https://github.com/rust-lang/rust-clippy/discussions/14256
#![allow(clippy::doc_overindented_list_items)]

use std::{
    io::{self, IoSlice, IoSliceMut},
    os::unix::io::{AsFd, RawFd},
    pin::Pin,
};

use futures::{
    task::{Context, Poll},
    Future,
};
use mio_aio::AioFsyncMode;
use tokio::io::bsd::{Aio, AioSource};

nix::ioctl_read! {
    /// Get the size of the entire device in bytes.  This should be a multiple
    /// of the sector size.
    diocgmediasize, 'd', 129, nix::libc::off_t
}

nix::ioctl_read! {
    diocgsectorsize, 'd', 128, nix::libc::c_uint
}

nix::ioctl_read! {
    diocgstripesize, 'd', 139, nix::libc::off_t
}

#[derive(Debug)]
struct TokioSource<T>(T);

impl<T: mio_aio::SourceApi> AioSource for TokioSource<T> {
    fn register(&mut self, kq: RawFd, token: usize) {
        self.0.register_raw(kq, token)
    }

    fn deregister(&mut self) {
        self.0.deregister_raw()
    }
}

#[must_use = "futures do nothing unless polled"]
#[derive(Debug)]
/// Future type used by all methods of [`File`].
pub struct TokioFileFut<T: mio_aio::SourceApi>(Aio<TokioSource<T>>);

impl<T: mio_aio::SourceApi> Future for TokioFileFut<T> {
    type Output = io::Result<T::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let poll_result = self.0.poll_ready(cx);
        match poll_result {
            Poll::Pending => {
                if !self.0 .0.in_progress() {
                    let p = unsafe { self.map_unchecked_mut(|s| &mut s.0 .0) };
                    match p.submit() {
                        Ok(()) => (),
                        Err(e) => {
                            return Poll::Ready(Err(
                                io::Error::from_raw_os_error(e as i32),
                            ))
                        }
                    }
                }
                Poll::Pending
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(_ev)) => {
                // At this point, we could clear readiness.  But there's no
                // point, since we're about to drop the Aio.
                let p = unsafe { self.map_unchecked_mut(|s| &mut s.0 .0) };
                let result = p.aio_return();
                match result {
                    Ok(r) => Poll::Ready(Ok(r)),
                    Err(e) => {
                        Poll::Ready(Err(io::Error::from_raw_os_error(e as i32)))
                    }
                }
            }
        }
    }
}

/// Return type of [`AioFileExt::read_at`].  Implements `Future`.
pub type ReadAt<'a> = TokioFileFut<mio_aio::ReadAt<'a>>;

/// Return type of [`AioFileExt::readv_at`].  Implements `Future`.
pub type ReadvAt<'a> = TokioFileFut<mio_aio::ReadvAt<'a>>;

/// Return type of [`AioFileExt::sync_all`].  Implements `Future`.
pub type SyncAll<'a> = TokioFileFut<mio_aio::Fsync<'a>>;

/// Return type of [`AioFileExt::write_at`].  Implements `Future`.
pub type WriteAt<'a> = TokioFileFut<mio_aio::WriteAt<'a>>;

/// Return type of [`AioFileExt::writev_at`].  Implements `Future`.
pub type WritevAt<'a> = TokioFileFut<mio_aio::WritevAt<'a>>;

/// Adds POSIX AIO-based asynchronous methods to files.
pub trait AioFileExt: AsFd {
    /// Asynchronous equivalent of `std::fs::File::read_at`
    ///
    /// # Examples
    ///
    /// ```
    /// use std::fs;
    /// use std::io::Write;
    /// use tempfile::TempDir;
    /// use tokio_file::AioFileExt;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// const WBUF: &[u8] = b"abcdef";
    /// const EXPECT: &[u8] = b"cdef";
    /// let mut rbuf = vec![0; 4];
    /// let dir = TempDir::new().unwrap();
    /// let path = dir.path().join("foo");
    /// let mut f = fs::File::create(&path).unwrap();
    /// f.write(WBUF).unwrap();
    ///
    /// let file = fs::OpenOptions::new()
    ///     .read(true)
    ///     .open(path)
    ///     .unwrap();
    /// let r = file.read_at(&mut rbuf[..], 2).unwrap().await.unwrap();
    /// assert_eq!(&rbuf[..], &EXPECT[..]);
    /// # }
    /// ```
    fn read_at<'a>(
        &'a self,
        buf: &'a mut [u8],
        offset: u64,
    ) -> io::Result<ReadAt<'a>> {
        let fd = self.as_fd();
        let source = TokioSource(mio_aio::ReadAt::read_at(fd, offset, buf, 0));
        Ok(TokioFileFut(Aio::new_for_aio(source)?))
    }

    /// Asynchronous equivalent of `preadv`.
    ///
    /// Similar to
    /// [preadv(2)](https://www.freebsd.org/cgi/man.cgi?query=read&sektion=2)
    /// but asynchronous.  Reads a contiguous portion of a file into a
    /// scatter-gather list of buffers.
    ///
    /// # Parameters
    ///
    /// - `bufs`:   The destination for the read.  A scatter-gather list of
    ///             buffers.
    /// - `offset`: Offset within the file at which to begin the read
    ///
    /// # Returns
    ///
    /// - `Ok(x)`:  The operation was successfully created.  The future may be
    ///             polled and will eventually return the final status of the
    ///             operation.
    /// - `Err(x)`: An error occurred before issueing the operation.  The result
    ///             may be `drop`ped.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::borrow::BorrowMut;
    /// use std::fs;
    /// use std::io::{IoSliceMut, Write};
    /// use tempfile::TempDir;
    /// use tokio_file::AioFileExt;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// const WBUF: &[u8] = b"abcdefghijklmnopqrwtuvwxyz";
    /// const EXPECT0: &[u8] = b"cdef";
    /// const EXPECT1: &[u8] = b"ghijklmn";
    /// let l0 = 4;
    /// let l1 = 8;
    /// let mut rbuf0 = vec![0; l0];
    /// let mut rbuf1 = vec![0; l1];
    /// let mut rbufs = [IoSliceMut::new(&mut rbuf0), IoSliceMut::new(&mut rbuf1)];
    ///
    /// let dir = TempDir::new().unwrap();
    /// let path = dir.path().join("foo");
    /// let mut f = fs::File::create(&path).unwrap();
    /// f.write(WBUF).unwrap();
    ///
    /// let file = fs::OpenOptions::new()
    ///     .read(true)
    ///     .open(path)
    ///     .unwrap();
    /// let mut r = file.readv_at(&mut rbufs[..], 2).unwrap().await.unwrap();
    ///
    /// assert_eq!(l0 + l1, r);
    /// assert_eq!(&rbuf0[..], &EXPECT0[..]);
    /// assert_eq!(&rbuf1[..], &EXPECT1[..]);
    /// # }
    /// ```
    fn readv_at<'a>(
        &'a self,
        bufs: &mut [IoSliceMut<'a>],
        offset: u64,
    ) -> io::Result<ReadvAt<'a>> {
        let fd = self.as_fd();
        let source =
            TokioSource(mio_aio::ReadvAt::readv_at(fd, offset, bufs, 0));
        Ok(TokioFileFut(Aio::new_for_aio(source)?))
    }

    /// Asynchronous equivalent of `std::fs::File::sync_all`
    ///
    /// # Examples
    ///
    /// ```
    /// use std::borrow::BorrowMut;
    /// use std::fs;
    /// use std::io::Write;
    /// use tempfile::TempDir;
    /// use tokio_file::AioFileExt;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let dir = TempDir::new().unwrap();
    /// let path = dir.path().join("foo");
    ///
    /// let file = fs::OpenOptions::new()
    ///     .write(true)
    ///     .create(true)
    ///     .open(path)
    ///     .unwrap();
    /// let r = AioFileExt::sync_all(&file).unwrap().await.unwrap();
    /// # }
    /// ```
    // TODO: add sync_all_data, for supported operating systems
    fn sync_all(&self) -> io::Result<SyncAll<'_>> {
        let mode = AioFsyncMode::O_SYNC;
        let fd = self.as_fd();
        let source = TokioSource(mio_aio::Fsync::fsync(fd, mode, 0));
        Ok(TokioFileFut(Aio::new_for_aio(source)?))
    }

    /// Asynchronous equivalent of `std::fs::File::write_at`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::fs;
    /// use std::io::Read;
    /// use tempfile::TempDir;
    /// use tokio_file::AioFileExt;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let contents = b"abcdef";
    /// let mut rbuf = Vec::new();
    ///
    /// let dir = TempDir::new().unwrap();
    /// let path = dir.path().join("foo");
    /// let file = fs::OpenOptions::new()
    ///     .create(true)
    ///     .write(true)
    ///     .open(&path)
    ///     .unwrap();
    /// let r = file.write_at(contents, 0).unwrap().await.unwrap();
    /// assert_eq!(r, contents.len());
    /// drop(file);
    ///
    /// let mut file = fs::File::open(path).unwrap();
    /// assert_eq!(file.read_to_end(&mut rbuf).unwrap(), contents.len());
    /// assert_eq!(&contents[..], &rbuf[..]);
    /// # }
    /// ```
    fn write_at<'a>(
        &'a self,
        buf: &'a [u8],
        offset: u64,
    ) -> io::Result<WriteAt<'a>> {
        let fd = self.as_fd();
        let source =
            TokioSource(mio_aio::WriteAt::write_at(fd, offset, buf, 0));
        Ok(TokioFileFut(Aio::new_for_aio(source)?))
    }

    /// Asynchronous equivalent of `pwritev`
    ///
    /// Similar to
    /// [pwritev(2)](https://www.freebsd.org/cgi/man.cgi?query=write&sektion=2)
    /// but asynchronous.  Writes a scatter-gather list of buffers into a
    /// contiguous portion of a file.
    ///
    /// # Parameters
    ///
    /// - `bufs`:   The data to write.  A scatter-gather list of buffers.
    /// - `offset`: Offset within the file at which to begin the write
    ///
    /// # Returns
    ///
    /// - `Ok(x)`:  The operation was successfully created.  The future may be
    ///             polled and will eventually return the final status of the
    ///             operation.
    /// - `Err(x)`: An error occurred before issueing the operation.  The result
    ///             may be `drop`ped.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::fs;
    /// use std::io::{IoSlice, Read};
    /// use tempfile::TempDir;
    /// use tokio_file::AioFileExt;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// const EXPECT: &[u8] = b"abcdefghij";
    /// let wbuf0 = b"abcdef";
    /// let wbuf1 = b"ghij";
    /// let wbufs = vec![IoSlice::new(wbuf0), IoSlice::new(wbuf1)];
    /// let mut rbuf = Vec::new();
    ///
    /// let dir = TempDir::new().unwrap();
    /// let path = dir.path().join("foo");
    /// let file = fs::OpenOptions::new()
    ///     .create(true)
    ///     .write(true)
    ///     .open(&path)
    ///     .unwrap();
    /// let r = file.writev_at(&wbufs[..], 0).unwrap().await.unwrap();
    ///
    /// assert_eq!(r, 10);
    ///
    /// let mut f = fs::File::open(path).unwrap();
    /// let len = f.read_to_end(&mut rbuf).unwrap();
    /// assert_eq!(len, EXPECT.len());
    /// assert_eq!(rbuf, EXPECT);
    /// # }
    fn writev_at<'a>(
        &'a self,
        bufs: &[IoSlice<'a>],
        offset: u64,
    ) -> io::Result<WritevAt<'a>> {
        let fd = self.as_fd();
        let source =
            TokioSource(mio_aio::WritevAt::writev_at(fd, offset, bufs, 0));
        Ok(TokioFileFut(Aio::new_for_aio(source)?))
    }
}

impl<T: AsFd> AioFileExt for T {}
