// vim: tw=80
use futures::{
    Future,
    task::{Context, Poll}
};
use mio::unix::UnixReady;
pub use mio_aio::LioError;
use tokio::io::PollEvented;
use std::{fs, io, mem};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;
use std::pin::Pin;

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

// LCOV_EXCL_START
#[derive(Debug)]
enum AioOp<'a> {
    Fsync(PollEvented<mio_aio::AioCb<'static>>),
    Read(PollEvented<mio_aio::AioCb<'a>>),
    Write(PollEvented<mio_aio::AioCb<'a>>),
}
// LCOV_EXCL_STOP

// LCOV_EXCL_START
/// Represents the progress of a single AIO operation
#[derive(Debug, Eq, PartialEq)]
enum AioState {
    /// The AioFut has been allocated, but not submitted to the system
    Allocated,
    /// The AioFut has been submitted, ie with `write_at`, and is currently in
    /// progress, but its status has not been returned with `aio_return`
    InProgress,
    /// A `ReadvAt` or `WritevAt` has been submitted and some of its constituent
    /// `AioCb`s are in-progress, but some are not due to resource limitations.
    Incomplete,
}
// LCOV_EXCL_STOP

// LCOV_EXCL_START
/// A Future representing an AIO operation.
#[must_use = "futures do nothing unless polled"]
#[derive(Debug)]
pub struct AioFut<'a> {
    op: AioOp<'a>,
    state: AioState,
}
// LCOV_EXCL_STOP

impl<'a> AioFut<'a> {
    // Used internally by `futures::Future::poll`.  Should not be called by the
    // user.
    #[doc(hidden)]
    fn aio_return(&mut self) -> Result<Option<isize>, nix::Error> {
        match self.op {
            AioOp::Fsync(ref mut io) =>
                io.get_mut().aio_return().map(|_| None),
            AioOp::Read(ref mut io) | AioOp::Write(ref mut io) =>
                io.get_mut().aio_return().map(Some),
        }
    }
}

/// Holds the result of an individual aio operation
pub struct AioResult {
    /// This is what the AIO operation would've returned, had it been
    /// synchronous and successful.  fsync operations return `None`, read and
    /// write operations return an `isize`
    pub value: Option<isize>,
}

/// The return value of [`readv_at`]
#[must_use = "futures do nothing unless polled"]
#[allow(clippy::type_complexity)]
pub struct ReadvAt<'a> {
    op: Option<PollEvented<mio_aio::LioCb<'a>>>,
    /// If needed, bufsav.0 combines [`readv_at`]'s argument slices into a
    /// bigger slice that satisfies sectorsize requirements.  After completion,
    /// the data will be copied back to bufsav.1
    bufsav: Option<(Pin<Box<[u8]>>, &'a mut [&'a mut [u8]])>,
    state: AioState,
}

/// The return value of [`writev_at`]
#[must_use = "futures do nothing unless polled"]
pub struct WritevAt<'a> {
    op: Option<PollEvented<mio_aio::LioCb<'a>>>,
    /// If needed, _accumulator combines [`writev_at`]'s argument slices into a
    /// bigger slice that satisfies sectorsize requirements, and owns the data
    /// for the lifetime of [`op`].
    _accumulator: Option<Pin<Box<[u8]>>>,
    state: AioState,
}

impl<'a> Future for ReadvAt<'a> {
    type Output = Result<usize, nix::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let poll_result = self.op
                              .as_mut()
                              .unwrap()
                              .poll_read_ready(cx, UnixReady::lio().into());
        if let AioState::Allocated = self.state {
            let result = self.op.as_mut().unwrap().get_mut().submit();
            match result {
                Ok(()) => self.state = AioState::InProgress,
                Err(LioError::EINCOMPLETE) => {
                    // EINCOMPLETE means that some requests failed, but some
                    // were initiated
                    self.state = AioState::Incomplete;
                },
                Err(LioError::EAGAIN) =>
                    return Poll::Ready(Err(nix::Error::Sys(nix::errno::Errno::EAGAIN))),
                Err(LioError::EIO(_)) => {
                    return Poll::Ready(Err(nix::Error::Sys(nix::errno::Errno::EIO)))
                },
            }
        }
        if !poll_result.is_ready() {
            return Poll::Pending;
        }
        if AioState::Incomplete == self.state {
            // Some requests must've completed; now issue the rest.
            let result = self.op.as_mut().unwrap().get_mut().resubmit();
            self.op
                .as_mut()
                .unwrap()
                .clear_read_ready(cx, UnixReady::lio().into())
                .unwrap();
            match result {
                Ok(()) => {
                    self.state = AioState::InProgress;
                    return Poll::Pending;
                },
                Err(LioError::EINCOMPLETE) => {
                    return Poll::Pending;
                },
                Err(LioError::EAGAIN) =>
                    return Poll::Ready(Err(nix::Error::Sys(nix::errno::Errno::EAGAIN))),
                Err(LioError::EIO(_)) =>
                    return Poll::Ready(Err(nix::Error::Sys(nix::errno::Errno::EIO))),
            }
        }
        let r = self.op.take()
            .unwrap()
            .into_inner().unwrap()
            .into_results(|mut iter|
                iter.try_fold(0, |total, lr|
                    lr.result.map(|r| total + r as usize)
                )
            );
        if let Ok(v) = r {
            if let Some((accum, ob)) = &mut self.bufsav  {
                // Copy results back into the individual buffers
                let mut i = 0;
                let mut j = 0;
                let mut total = 0;
                while total < v {
                    let z = (v - total).min(ob[i].len() - j);
                    ob[i][j..j + z].copy_from_slice(&accum[total..total + z]);
                    j += z;
                    total += z;
                    if j == ob[i].len() {
                        j = 0;
                        i += 1;
                    }
                }
            }
        }
        Poll::Ready(r)
}
}

impl<'a> Future for WritevAt<'a> {
    type Output = Result<usize, nix::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let poll_result = self.op
                              .as_mut()
                              .unwrap()
                              .poll_read_ready(cx, UnixReady::lio().into());
        if let AioState::Allocated = self.state {
            let result = self.op.as_mut().unwrap().get_mut().submit();
            match result {
                Ok(()) => self.state = AioState::InProgress,
                Err(LioError::EINCOMPLETE) => {
                    // EINCOMPLETE means that some requests failed, but some
                    // were initiated
                    self.state = AioState::Incomplete;
                },
                Err(LioError::EAGAIN) =>
                    return Poll::Ready(Err(nix::Error::Sys(nix::errno::Errno::EAGAIN))),
                Err(LioError::EIO(_)) => {
                    return Poll::Ready(Err(nix::Error::Sys(nix::errno::Errno::EIO)))
                },
            }
        }
        if !poll_result.is_ready() {
            return Poll::Pending;
        }
        if AioState::Incomplete == self.state {
            // Some requests must've completed; now issue the rest.
            let result = self.op.as_mut().unwrap().get_mut().resubmit();
            self.op
                .as_mut()
                .unwrap()
                .clear_read_ready(cx, UnixReady::lio().into())
                .unwrap();
            match result {
                Ok(()) => {
                    self.state = AioState::InProgress;
                    return Poll::Pending;
                },
                Err(LioError::EINCOMPLETE) => {
                    return Poll::Pending;
                },
                Err(LioError::EAGAIN) =>
                    return Poll::Ready(Err(nix::Error::Sys(nix::errno::Errno::EAGAIN))),
                Err(LioError::EIO(_)) =>
                    return Poll::Ready(Err(nix::Error::Sys(nix::errno::Errno::EIO))),
            }
        }
        let r = self.op.take()
            .unwrap()
            .into_inner().unwrap()
            .into_results(|mut iter|
                iter.try_fold(0, |total, lr|
                    lr.result.map(|r| total + r as usize)
                )
            );
        Poll::Ready(r)
    }
}

/// Basically a Tokio file handle.  This is the starting point for tokio-file.
// LCOV_EXCL_START
#[derive(Debug)]
pub struct File {
    file: fs::File,
    /// The preferred (not necessarily minimum) sector size for accessing
    /// the device
    sectorsize: usize
}
// LCOV_EXCL_STOP

// is_empty doesn't make much sense for files
#[cfg_attr(feature = "cargo-clippy", allow(clippy::len_without_is_empty))]
impl File {
    /// Get the file's size in bytes
    pub fn len(&self) -> io::Result<u64> {
        let md = self.metadata()?;
        if self.sectorsize > 1 {
            let mut mediasize = mem::MaybeUninit::<nix::libc::off_t>::uninit();
            // This ioctl is always safe
            unsafe {
                diocgmediasize(self.file.as_raw_fd(), mediasize.as_mut_ptr())
            }.map_err(|_| io::Error::from_raw_os_error(nix::errno::errno()))?;
            // Safe because we know the ioctl succeeded
            unsafe { Ok(mediasize.assume_init() as u64) }
        } else {
            Ok(md.len())
        }
    }

    /// Get metadata from the underlying file
    ///
    /// POSIX AIO doesn't provide a way to do this asynchronously, so it must be
    /// synchronous.
    pub fn metadata(&self) -> io::Result<fs::Metadata> {
        self.file.metadata()
    }

    /// Create a new Tokio File from an ordinary `std::fs::File` object
    ///
    /// # Examples
    ///
    /// ```
    /// use std::fs;
    /// use tokio_file;
    ///
    /// fs::OpenOptions::new()
    ///     .read(true)
    ///     .write(true)
    ///     .create(true)
    ///     .open("/tmp/tokio-file-new-example")
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// # fs::remove_file("/tmp/tokio-file-new-example").unwrap();
    /// ```
    pub fn new(file: fs::File) -> File {
        let md = file.metadata().unwrap();
        let ft = md.file_type();
        let sectorsize = if ft.is_block_device() || ft.is_char_device() {
            let mut sectorsize = mem::MaybeUninit::<u32>::uninit();
            let mut stripesize = mem::MaybeUninit::<nix::libc::off_t>::uninit();
            let fd = file.as_raw_fd();
            unsafe {
                diocgsectorsize(fd, sectorsize.as_mut_ptr()).unwrap();
                diocgstripesize(fd, stripesize.as_mut_ptr()).unwrap();
                if stripesize.assume_init() > 0 {
                    stripesize.assume_init() as usize
                } else {
                    sectorsize.assume_init() as usize
                }
            }
        } else {
            1
        };
        File {file, sectorsize}
    }

    /// Open a new Tokio file with mode `O_RDWR | O_CREAT`.
    // Technically, sfd::fs::File::open can block, so we should make a
    // nonblocking File::open method and have it return a Future.  That's what
    // Seastar does.  But POSIX AIO doesn't have any kind of asynchronous open
    // function, so there's no straightforward way to implement such a method.
    // Instead, we'll block.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<File> {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .map( File::new)
    }

    /// Asynchronous equivalent of `std::fs::File::read_at`
    ///
    /// # Examples
    ///
    /// ```
    /// use std::fs;
    /// use std::io::Write;
    /// use tempfile::TempDir;
    /// use tokio::runtime::Runtime;
    ///
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
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let mut rt = Runtime::new().unwrap();
    /// let r = rt.block_on(async {
    ///     file.read_at(&mut rbuf[..], 2).unwrap().await
    /// }).unwrap();
    /// assert_eq!(&rbuf[..], &EXPECT[..]);
    /// ```
    pub fn read_at<'a>(&self, buf: &'a mut [u8], offset: u64)
        -> io::Result<AioFut<'a>>
    {
        let aiocb = mio_aio::AioCb::from_mut_slice(self.file.as_raw_fd(),
                            offset,  //offset
                            buf,
                            0,  //priority
                            mio_aio::LioOpcode::LIO_NOP);
        PollEvented::new(aiocb)
        .map(|pe| AioFut {
            op: AioOp::Read(pe),
            state: AioState::Allocated
        })
    }

    /// Asynchronous equivalent of `preadv`.
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
    /// - `Ok(x)`:  The operation was successfully created.  The future may be
    ///             polled and will eventually return the final status of the
    ///             operation.  If the operation was partially successful, the
    ///             future will complete an error with no indication of which
    ///             parts of `bufs` are valid.
    /// - `Err(x)`: An error occurred before issueing the operation.  The result
    ///             may be `drop`ped.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::borrow::BorrowMut;
    /// use std::fs;
    /// use std::io::Write;
    /// use tempfile::TempDir;
    /// use tokio::runtime::Runtime;
    ///
    /// const WBUF: &[u8] = b"abcdefghijklmnopqrwtuvwxyz";
    /// const EXPECT0: &[u8] = b"cdef";
    /// const EXPECT1: &[u8] = b"ghijklmn";
    /// let l0 = 4;
    /// let l1 = 8;
    /// let mut rbuf0 = vec![0; l0];
    /// let mut rbuf1 = vec![0; l1];
    /// let mut rbufs = [&mut rbuf0[..], &mut rbuf1[..]];
    ///
    /// let dir = TempDir::new().unwrap();
    /// let path = dir.path().join("foo");
    /// let mut f = fs::File::create(&path).unwrap();
    /// f.write(WBUF).unwrap();
    ///
    /// let file = fs::OpenOptions::new()
    ///     .read(true)
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let mut rt = Runtime::new().unwrap();
    /// let mut r = rt.block_on(async {
    ///     file.readv_at(&mut rbufs[..], 2).unwrap().await
    /// }).unwrap();
    ///
    /// assert_eq!(l0 + l1, r);
    /// assert_eq!(&rbuf0[..], &EXPECT0[..]);
    /// assert_eq!(&rbuf1[..], &EXPECT1[..]);
    /// ```
    pub fn readv_at<'a>(&self, bufs: &'a mut [&'a mut [u8]],
                        offset: u64) -> io::Result<ReadvAt<'a>>
    {
        let mut builder = mio_aio::LioCbBuilder::with_capacity(bufs.len());
        let mut offs = offset;
        let fd = self.file.as_raw_fd();
        let mut bufsav = None;
        if self.sectorsize > 1 &&
            bufs.iter().any(|buf| buf.len() % self.sectorsize != 0)
        {
            let l = bufs.iter().map(|buf| buf.len()).sum();
            let mut accumulator: Pin<Box<[u8]>> =
                vec![0; l].into_boxed_slice().into();
            let original_buffers = bufs;
            let buf: &'static mut [u8] = unsafe{
                // Safe because the liocb's lifetime is equal to accumulator's
                // (or rather, it will be once we move it into the ReadvAt
                // struct).
                mem::transmute::<&mut [u8], &'static mut [u8]>(
                    &mut (accumulator.as_mut())
                )
            };
            bufsav = Some((accumulator, original_buffers));
            builder = builder.emplace_mut_slice(
                fd,
                offs,
                buf,
                0,
                mio_aio::LioOpcode::LIO_READ
            );
        } else {
            for buf in bufs.iter_mut() {
                let l = buf.len();
                builder = builder.emplace_mut_slice(
                    fd,
                    offs,
                    *buf,
                    0,
                    mio_aio::LioOpcode::LIO_READ
                );
                offs += l as u64;
            }
        }
        let liocb = builder.finish();
        PollEvented::new(liocb)
        .map(|pe| ReadvAt {
            op: Some(pe),
            bufsav,
            //accumulator,
            state: AioState::Allocated,
            //original_buffers
        })
    }

    /// Asynchronous equivalent of `std::fs::File::write_at`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::fs;
    /// use std::io::Read;
    /// use tempfile::TempDir;
    /// use tokio::runtime::Runtime;
    ///
    /// let contents = b"abcdef";
    /// let mut rbuf = Vec::new();
    ///
    /// let dir = TempDir::new().unwrap();
    /// let path = dir.path().join("foo");
    /// let file = fs::OpenOptions::new()
    ///     .create(true)
    ///     .write(true)
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let mut rt = Runtime::new().unwrap();
    /// let r = rt.block_on(async {
    ///     file.write_at(contents, 0).unwrap().await
    /// }).unwrap();
    /// assert_eq!(r.value.unwrap() as usize, contents.len());
    /// drop(file);
    ///
    /// let mut file = fs::File::open(&path).unwrap();
    /// assert_eq!(file.read_to_end(&mut rbuf).unwrap(), contents.len());
    /// assert_eq!(&contents[..], &rbuf[..]);
    /// ```
    pub fn write_at<'a>(&self, buf: &'a [u8],
                    offset: u64) -> io::Result<AioFut<'a>>
    {
        let fd = self.file.as_raw_fd();
        let aiocb = mio_aio::AioCb::from_slice(fd, offset, buf, 0,
            mio_aio::LioOpcode::LIO_NOP);
        PollEvented::new(aiocb)
        .map(|pe| AioFut{
            op: AioOp::Write(pe),
            state: AioState::Allocated
        })
    }

    /// Asynchronous equivalent of `pwritev`
    ///
    /// Similar to
    /// [pwritev(2)](https://www.freebsd.org/cgi/man.cgi?query=write&sektion=2)
    /// but asynchronous.  Writes a scatter-gather list of buffers into a
    /// contiguous portion of a file.  Unlike `pwritev`, there is no guarantee
    /// of overall atomicity.  Each scatter gather element's contents are
    /// written independently.
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
    ///             operation.  If the operation was partially successful, the
    ///             future will complete an error with no indication of which
    ///             parts of the file were actually written.
    /// - `Err(x)`: An error occurred before issueing the operation.  The result
    ///             may be `drop`ped.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::fs;
    /// use std::io::Read;
    /// use tempfile::TempDir;
    /// use tokio::runtime::Runtime;
    ///
    /// const EXPECT: &[u8] = b"abcdefghij";
    /// let wbuf0 = b"abcdef";
    /// let wbuf1 = b"ghij";
    /// let wbufs = vec![&wbuf0[..], &wbuf1[..]];
    /// let mut rbuf = Vec::new();
    ///
    /// let dir = TempDir::new().unwrap();
    /// let path = dir.path().join("foo");
    /// let file = fs::OpenOptions::new()
    ///     .create(true)
    ///     .write(true)
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let mut rt = Runtime::new().unwrap();
    /// let r = rt.block_on(async {
    ///     file.writev_at(&wbufs[..], 0).unwrap().await
    /// }).unwrap();
    ///
    /// assert_eq!(r, 10);
    ///
    /// let mut f = fs::File::open(&path).unwrap();
    /// let len = f.read_to_end(&mut rbuf).unwrap();
    /// assert_eq!(len, EXPECT.len());
    /// assert_eq!(rbuf, EXPECT);
    pub fn writev_at<'a>(&self, bufs: &[&'a [u8]], offset: u64)
        -> io::Result<WritevAt<'a>>
    {
        let mut builder = mio_aio::LioCbBuilder::with_capacity(bufs.len());
        let mut offs = offset;
        let fd = self.file.as_raw_fd();
        let mut accumulator: Option<Pin<Box<[u8]>>> = None;
        if self.sectorsize > 1 &&
            bufs.iter().any(|buf| buf.len() % self.sectorsize != 0)
        {
            let mut accum = Vec::<u8>::new();
            for buf in bufs.iter() {
                accum.extend_from_slice(&buf[..]);
            }
            accumulator = Some(accum.into_boxed_slice().into());
            let buf: &'static [u8] = unsafe{
                // Safe because the liocb's lifetime is equal to accumulator's
                // (or rather, it will be once we move it into the WritevAt
                // struct).
                mem::transmute::<&[u8], &'static [u8]>(
                    &(accumulator.as_ref().unwrap().as_ref())
                )
            };
            builder = builder.emplace_slice(
                fd,
                offs,
                buf,
                0,
                mio_aio::LioOpcode::LIO_WRITE
            );
        } else {
            for buf in bufs {
                let l = buf.len();
                builder = builder.emplace_slice(
                    fd,
                    offs,
                    buf,
                    0,
                    mio_aio::LioOpcode::LIO_WRITE
                );
                offs += l as u64;
            }
        }
        let liocb = builder.finish();
        PollEvented::new(liocb)
        .map(|pe| 
             WritevAt {
                _accumulator: accumulator,
                op: Some(pe),
                state: AioState::Allocated,
            }
        )
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
    /// use tokio::runtime::Runtime;
    ///
    /// let dir = TempDir::new().unwrap();
    /// let path = dir.path().join("foo");
    ///
    /// let file = fs::OpenOptions::new()
    ///     .write(true)
    ///     .create(true)
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let mut rt = Runtime::new().unwrap();
    /// let r = rt.block_on(async {
    ///     file.sync_all().unwrap().await
    /// }).unwrap();
    /// ```
    // TODO: add sync_all_data, for supported operating systems
    pub fn sync_all(&self) -> io::Result<AioFut<'static>> {
        let aiocb = mio_aio::AioCb::from_fd(self.file.as_raw_fd(),
                            0,  //priority
                            );
        PollEvented::new(aiocb)
        .map(|pe| AioFut{
            op: AioOp::Fsync(pe),
            state: AioState::Allocated
        })
    }
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

impl<'a> Future for AioFut<'a> {
    type Output = Result<AioResult, nix::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let poll_result = match self.op {
                AioOp::Fsync(ref mut io) =>
                    io.poll_read_ready(cx, UnixReady::aio().into()),
                AioOp::Read(ref mut io) =>
                    io.poll_read_ready(cx, UnixReady::aio().into()),
                AioOp::Write(ref mut io) =>
                    io.poll_read_ready(cx, UnixReady::aio().into()),
        };
        if !poll_result.is_ready() {
            if self.state == AioState::Allocated {
                let r = match self.op {
                    AioOp::Fsync(ref mut pe) => pe.get_mut()
                        .fsync(mio_aio::AioFsyncMode::O_SYNC),
                    AioOp::Read(ref mut pe) => pe.get_mut().read(),
                    AioOp::Write(ref mut pe) => pe.get_mut().write(),
                };
                if let Err(e) = r {
                    return Poll::Ready(Err(e));
                }
                self.state = AioState::InProgress;
            }
            return Poll::Pending;
        }
        let result = self.aio_return();
        match result {
            Ok(x) => Poll::Ready(Ok(AioResult{value: x})),
            Err(x) => Poll::Ready(Err(x))
        }
    }
}
