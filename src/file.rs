// vim: tw=80
use futures::{Async, Future, Poll};
use mio::unix::UnixReady;
use mio_aio;
pub use mio_aio::{BufRef, LioError};
use nix;
use tokio_reactor::PollEvented;
use std::{fs, io, mem};
use std::borrow::{Borrow, BorrowMut};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;

ioctl_read! {
    /// Get the size of the entire device in bytes.  This should be a multiple
    /// of the sector size.
    diocgmediasize, 'd', 129, nix::libc::off_t
}

ioctl_read! {
    diocgsectorsize, 'd', 128, nix::libc::c_uint
}

ioctl_read! {
    diocgstripesize, 'd', 139, nix::libc::off_t
}

// LCOV_EXCL_START
#[derive(Debug)]
enum AioOp {
    Fsync(PollEvented<mio_aio::AioCb<'static>>),
    Read(PollEvented<mio_aio::AioCb<'static>>),
    Write(PollEvented<mio_aio::AioCb<'static>>),
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
    /// An `LioFut` has been submitted and some of its constituent `AioCb`s are
    /// in-progress, but some are not due to resource limitations.
    Incomplete,
}
// LCOV_EXCL_STOP

// LCOV_EXCL_START
/// A Future representing an AIO operation.
#[must_use = "futures do nothing unless polled"]
#[derive(Debug)]
pub struct AioFut {
    op: AioOp,
    state: AioState,
}
// LCOV_EXCL_STOP

impl AioFut {
    // Used internally by `futures::Future::poll`.  Should not be called by the
    // user.
    #[doc(hidden)]
    fn aio_return(&mut self) -> Result<Option<isize>, nix::Error> {
        match self.op {
            AioOp::Fsync(ref io) =>
                io.get_ref().aio_return().map(|_| None),
            AioOp::Read(ref io) | AioOp::Write(ref io) =>
                io.get_ref().aio_return().map(Some),
        }
    }
}

/// Holds the result of an individual aio operation
pub struct AioResult {
    /// This is what the AIO operation would've returned, had it been
    /// synchronous and successful.  fsync operations return `None`, read and
    /// write operations return an `isize`
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

/// Holds the result of an individual lio operation
pub struct LioResult {
    /// This is what the AIO operation would've returned, had it been
    /// synchronous.
    // Since lio_listio can't include fsync operations, there's no need to
    // return a value of None, so we can use plain isize instead of
    // Option<isize>
    pub value: Result<isize, nix::Error>,

    /// Optionally, return ownership of the buffer that was used to create the
    /// LIO operation.
    pub buf: BufRef
}

impl LioResult {
    pub fn into_buf_ref(self) -> BufRef {
        self.buf
    }
}

/// A Future representing an LIO operation.
#[must_use = "futures do nothing unless polled"]
pub struct LioFut {
    op: Option<PollEvented<mio_aio::LioCb>>,
    state: AioState,
    original_buffers: Option<Vec<Option<(BufRef, bool)>>>
}

impl Future for LioFut {
    type Item = Box<Iterator<Item = LioResult>>;
    type Error = nix::Error;

    fn poll(&mut self) -> Poll<Box<Iterator<Item = LioResult>>, nix::Error> {
        let poll_result = self.op
                              .as_mut()
                              .unwrap()
                              .poll_read_ready(UnixReady::lio().into())
                              .unwrap();
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
                    return Err(nix::Error::Sys(nix::errno::Errno::EAGAIN)),
                Err(LioError::EIO(_)) => {
                    return Err(nix::Error::Sys(nix::errno::Errno::EIO))
                },
            }
        }
        if poll_result == Async::NotReady {
            return Ok(Async::NotReady);
        }
        if AioState::Incomplete == self.state {
            // Some requests must've completed; now issue the rest.
            let result = self.op.as_mut().unwrap().get_mut().resubmit();
            self.op
                .as_mut()
                .unwrap()
                .clear_read_ready(UnixReady::lio().into())
                .unwrap();
            match result {
                Ok(()) => {
                    self.state = AioState::InProgress;
                    return Ok(Async::NotReady);
                },
                Err(LioError::EINCOMPLETE) => {
                    return Ok(Async::NotReady);
                },
                Err(LioError::EAGAIN) =>
                    return Err(nix::Error::Sys(nix::errno::Errno::EAGAIN)),
                Err(LioError::EIO(_)) =>
                    return Err(nix::Error::Sys(nix::errno::Errno::EIO)),
            }
        }
        let mut op = None;
        mem::swap(&mut op, &mut self.op);
        let obuf_iter = self.original_buffers.take().unwrap().into_iter();
        let mut inner_iter = op.unwrap()
            .into_inner().unwrap()
            .into_results(|iter| {
            iter.map(|lr| {
                LioResult{value: lr.result, buf: lr.buf_ref}
            })
        });
        let mut last_inner_result: Option<LioResult> = None;
        let iter = obuf_iter.map(move |mut obuf_ref| {
            if let Some((buf_ref, mc)) = obuf_ref.take() {
                if mc {
                    last_inner_result = Some(inner_iter.next().unwrap());
                }
                let lir = last_inner_result.as_ref().unwrap();
                if lir.value.is_ok() {
                    let len = buf_ref.len().unwrap() as isize;
                    LioResult{value: Ok(len), buf: buf_ref}
                } else {
                    let r = lir.value;
                    LioResult{value: r, buf: buf_ref}
                }
            } else {
                inner_iter.next().unwrap()
            }
        });

        Ok(Async::Ready(Box::new(iter)))
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
            let mut sectorsize = mem::MaybeUninit::<u32>::uninit();;
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
    /// Note that it consumes `buf` rather than references it.  When the
    /// operation is complete, the buffer can be retrieved with
    /// [`AioResult::into_buf_ref`](struct.AioResult.html#method.into_buf_ref).
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate tempdir;
    /// # extern crate tokio;
    /// # extern crate tokio_file;
    /// use std::borrow::BorrowMut;
    /// use std::fs;
    /// use std::io::Write;
    /// use tempdir::TempDir;
    /// use tokio::runtime::current_thread;
    /// use tokio_file;
    ///
    /// const WBUF: &[u8] = b"abcdef";
    /// const EXPECT: &[u8] = b"cdef";
    /// let rbuf = Box::new(vec![0; 4].into_boxed_slice());
    /// let dir = TempDir::new("tokio-file").unwrap();
    /// let path = dir.path().join("foo");
    /// let mut f = fs::File::create(&path).unwrap();
    /// f.write(WBUF).unwrap();
    ///
    /// let file = fs::OpenOptions::new()
    ///     .read(true)
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let mut rt = current_thread::Runtime::new().unwrap();
    /// let r = rt.block_on(
    ///     file.read_at(rbuf, 2).unwrap()
    /// ).unwrap();
    ///
    /// let mut buf_ref = r.into_buf_ref();
    /// let borrowed : &mut BorrowMut<[u8]> = buf_ref.boxed_mut_slice()
    ///                                              .unwrap();
    /// assert_eq!(&borrowed.borrow_mut()[..], &EXPECT[..]);
    /// ```
    pub fn read_at(&self, buf: Box<BorrowMut<[u8]>>,
                   offset: u64) -> io::Result<AioFut> {
        let aiocb = mio_aio::AioCb::from_boxed_mut_slice(self.file.as_raw_fd(),
                            offset,  //offset
                            buf,
                            0,  //priority
                            mio_aio::LioOpcode::LIO_NOP);
        Ok(AioFut{
            op: AioOp::Read(PollEvented::new(aiocb)),
            state: AioState::Allocated })
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
    /// # extern crate tempdir;
    /// # extern crate tokio;
    /// # extern crate tokio_file;
    /// use std::borrow::BorrowMut;
    /// use std::fs;
    /// use std::io::Write;
    /// use tempdir::TempDir;
    /// use tokio::runtime::current_thread;
    /// use tokio_file;
    ///
    /// const WBUF: &[u8] = b"abcdefghijklmnopqrwtuvwxyz";
    /// const EXPECT0: &[u8] = b"cdef";
    /// const EXPECT1: &[u8] = b"ghijklmn";
    /// let rbuf0 = Box::new(vec![0; 4].into_boxed_slice());
    /// let rbuf1 = Box::new(vec![0; 8].into_boxed_slice());
    /// let rbufs : Vec<Box<BorrowMut<[u8]>>> = vec![rbuf0, rbuf1];
    ///
    /// let dir = TempDir::new("tokio-file").unwrap();
    /// let path = dir.path().join("foo");
    /// let mut f = fs::File::create(&path).unwrap();
    /// f.write(WBUF).unwrap();
    ///
    /// let file = fs::OpenOptions::new()
    ///     .read(true)
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let mut rt = current_thread::Runtime::new().unwrap();
    /// let mut ri = rt.block_on(
    ///     file.readv_at(rbufs, 2).unwrap()
    /// ).unwrap();
    ///
    /// let mut r0 = ri.next().unwrap();
    /// let b0 : &mut BorrowMut<[u8]> =
    ///     r0.buf.boxed_mut_slice().unwrap();
    /// assert_eq!(&b0.borrow_mut()[..], &EXPECT0[..]);
    ///
    /// let mut r1 = ri.next().unwrap();
    /// let b1 : &mut BorrowMut<[u8]> =
    ///     r1.buf.boxed_mut_slice().unwrap();
    /// assert_eq!(&b1.borrow_mut()[..], &EXPECT1[..]);
    ///
    /// assert!(ri.next().is_none());
    /// ```
    pub fn readv_at(&self, mut bufs: Vec<Box<BorrowMut<[u8]>>>,
                    offset: u64) -> io::Result<LioFut> {
        let mut liocb = mio_aio::LioCb::with_capacity(bufs.len());
        let mut offs = offset;
        let mut original_buffers = Vec::with_capacity(bufs.len());
        let fd = self.file.as_raw_fd();
        if self.sectorsize > 1 {
            // Accumulate unaligned buffers to meet the sectorsize requirement
            let mut oaccum: Option<Vec<u8>> = None;
            for mut buf in bufs.drain(..) {
                let l = buf.as_ref().borrow().len();
                if let Some(mut accum) = oaccum.take() {
                    let oldlen = accum.len();
                    accum.resize(oldlen + l, 0u8);
                    let l = accum.len();
                    let buf_ref = BufRef::BoxedMutSlice(buf);
                    original_buffers.push(Some((buf_ref, false)));
                    if l % self.sectorsize == 0 {
                        // issue the I/O
                        let b = Box::new(accum);
                        liocb.emplace_boxed_mut_slice(fd, offs, b, 0,
                            mio_aio::LioOpcode::LIO_READ);
                        offs += l as u64;
                    } else {
                        // Put it back
                        oaccum = Some(accum);
                    }
                } else if l % self.sectorsize == 0 {
                    liocb.emplace_boxed_mut_slice(fd, offs, buf, 0,
                        mio_aio::LioOpcode::LIO_READ);
                    offs += l as u64;
                    original_buffers.push(None);
                } else {
                    let mut accum = vec![0u8; l];
                    let buf_ref = BufRef::BoxedMutSlice(buf);
                    original_buffers.push(Some((buf_ref, true)));
                    oaccum = Some(accum);
                }
            }
            if let Some(accum) = oaccum {
                let l = accum.len();
                assert_eq!(l % self.sectorsize, 0, "Device nodes must be accessed in multiples if the sectorsize");
                let b = Box::new(accum);
                liocb.emplace_boxed_mut_slice(fd, offs, b, 0,
                    mio_aio::LioOpcode::LIO_READ);
            }
        } else {
            for buf in bufs.drain(..) {
                let l = buf.as_ref().borrow().len();
                liocb.emplace_boxed_mut_slice(fd, offs, buf, 0,
                    mio_aio::LioOpcode::LIO_READ);
                original_buffers.push(None);
                offs += l as u64;
            };
        }
        Ok(LioFut{
            op: Some(PollEvented::new(liocb)),
            state: AioState::Allocated,
            original_buffers: Some(original_buffers)})
    }

    /// Asynchronous equivalent of `std::fs::File::write_at`.
    ///
    /// Note that it consumes `buf` rather than references it.  When the
    /// operation is complete, the buffer can be retrieved with
    /// [`AioResult::into_buf_ref`](struct.AioResult.html#method.into_buf_ref).
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate tempdir;
    /// # extern crate tokio;
    /// # extern crate tokio_file;
    /// use std::borrow::Borrow;
    /// use std::fs;
    /// use std::io::Read;
    /// use tempdir::TempDir;
    /// use tokio::runtime::current_thread;
    /// use tokio_file;
    ///
    /// let contents = b"abcdef";
    /// let wbuf: Box<Borrow<[u8]>> = Box::new(&contents[..]);
    /// let mut rbuf = Vec::new();
    ///
    /// let dir = TempDir::new("tokio-file").unwrap();
    /// let path = dir.path().join("foo");
    /// let file = fs::OpenOptions::new()
    ///     .create(true)
    ///     .write(true)
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let mut rt = current_thread::Runtime::new().unwrap();
    /// let r = rt.block_on(
    ///     file.write_at(wbuf, 0).unwrap()
    /// ).unwrap();
    /// assert_eq!(r.value.unwrap() as usize, contents.len());
    /// drop(file);
    ///
    /// let mut file = fs::File::open(&path).unwrap();
    /// assert_eq!(file.read_to_end(&mut rbuf).unwrap(), contents.len());
    /// assert_eq!(&contents[..], &rbuf[..]);
    /// ```
    pub fn write_at(&self, buf: Box<Borrow<[u8]>>,
                    offset: u64) -> io::Result<AioFut> {
        let fd = self.file.as_raw_fd();
        let aiocb = mio_aio::AioCb::from_boxed_slice(fd, offset, buf, 0,
            mio_aio::LioOpcode::LIO_NOP);
        Ok(AioFut{
            op: AioOp::Write(PollEvented::new(aiocb)),
            state: AioState::Allocated })
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
    ///             parts of `bufs` are valid.
    /// - `Err(x)`: An error occurred before issueing the operation.  The result
    ///             may be `drop`ped.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate tempdir;
    /// # extern crate tokio;
    /// # extern crate tokio_file;
    /// use std::borrow::Borrow;
    /// use std::fs;
    /// use std::io::Read;
    /// use tempdir::TempDir;
    /// use tokio::runtime::current_thread;
    /// use tokio_file;
    ///
    /// const EXPECT: &[u8] = b"abcdefghij";
    /// let wbuf0: Box<Borrow<[u8]>> = Box::new(&b"abcdef"[..]);
    /// let wbuf1: Box<Borrow<[u8]>> = Box::new(&b"ghij"[..]);
    /// let wbufs : Vec<Box<Borrow<[u8]>>> = vec![wbuf0, wbuf1];
    /// let mut rbuf = Vec::new();
    ///
    /// let dir = TempDir::new("tokio-file").unwrap();
    /// let path = dir.path().join("foo");
    /// let file = fs::OpenOptions::new()
    ///     .create(true)
    ///     .write(true)
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let mut rt = current_thread::Runtime::new().unwrap();
    /// let mut wi = rt.block_on(
    ///     file.writev_at(wbufs, 0).unwrap()
    /// ).unwrap();
    ///
    /// let w0 = wi.next().unwrap();
    /// assert_eq!(w0.value.unwrap() as usize, 6);
    /// let w1 = wi.next().unwrap();
    /// assert_eq!(w1.value.unwrap() as usize, 4);
    ///
    /// let mut f = fs::File::open(&path).unwrap();
    /// let len = f.read_to_end(&mut rbuf).unwrap();
    /// assert_eq!(len, EXPECT.len());
    /// assert_eq!(rbuf, EXPECT);
    pub fn writev_at(&self, mut bufs: Vec<Box<Borrow<[u8]>>>,
                     offset: u64) -> io::Result<LioFut>
    {
        let buflen = |buf: &Borrow<[u8]>|{
            let slice : &[u8] = buf.borrow();
            slice.len()
        };
        let mut liocb = mio_aio::LioCb::with_capacity(bufs.len());
        let mut offs = offset;
        let mut original_buffers = Vec::with_capacity(bufs.len());
        let fd = self.file.as_raw_fd();
        if self.sectorsize > 1 {
            // Accumulate unaligned buffers to meet the sectorsize requirement
            let mut oaccum: Option<Vec<u8>> = None;
            for buf in bufs.drain(..) {
                let l = buflen(buf.as_ref());
                if let Some(mut accum) = oaccum.take() {
                    {
                        let sl: &[u8] = (*buf).borrow();
                        accum.extend_from_slice(&sl[..]);
                    }
                    let l = accum.len();
                    let buf_ref = BufRef::BoxedSlice(buf);
                    original_buffers.push(Some((buf_ref, false)));
                    if l % self.sectorsize == 0 {
                        // issue the I/O
                        let b = Box::new(accum);
                        liocb.emplace_boxed_slice(fd, offs, b, 0,
                            mio_aio::LioOpcode::LIO_WRITE);
                        offs += l as u64;
                    } else {
                        // Put it back
                        oaccum = Some(accum);
                    }
                } else if l % self.sectorsize == 0 {
                    liocb.emplace_boxed_slice(fd, offs, buf, 0,
                                              mio_aio::LioOpcode::LIO_WRITE);
                    offs += l as u64;
                    original_buffers.push(None);
                } else {
                    let mut accum = Vec::new();
                    {
                        let sl: &[u8] = (*buf).borrow();
                        accum.extend_from_slice(&sl[..]);
                    }
                    let buf_ref = BufRef::BoxedSlice(buf);
                    original_buffers.push(Some((buf_ref, true)));
                    oaccum = Some(accum);
                }
            }
            if let Some(accum) = oaccum {
                let l = accum.len();
                assert_eq!(l % self.sectorsize, 0, "Device nodes must be accessed in multiples if the sectorsize");
                let b = Box::new(accum);
                liocb.emplace_boxed_slice(fd, offs, b, 0,
                    mio_aio::LioOpcode::LIO_WRITE);
            }
        } else {
            for buf in bufs.drain(..) {
                let l = buflen(buf.as_ref());
                liocb.emplace_boxed_slice(fd, offs, buf, 0,
                                          mio_aio::LioOpcode::LIO_WRITE);
                original_buffers.push(None);
                offs += l as u64;
            };
        }

        Ok(LioFut{
            op: Some(PollEvented::new(liocb)),
            state: AioState::Allocated,
            original_buffers: Some(original_buffers)})
    }

    /// Asynchronous equivalent of `std::fs::File::sync_all`
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate tempdir;
    /// # extern crate tokio;
    /// # extern crate tokio_file;
    /// use std::borrow::BorrowMut;
    /// use std::fs;
    /// use std::io::Write;
    /// use tempdir::TempDir;
    /// use tokio::runtime::current_thread;
    /// use tokio_file;
    ///
    /// let dir = TempDir::new("tokio-file").unwrap();
    /// let path = dir.path().join("foo");
    ///
    /// let file = fs::OpenOptions::new()
    ///     .write(true)
    ///     .create(true)
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let mut rt = current_thread::Runtime::new().unwrap();
    /// let r = rt.block_on(
    ///     file.sync_all().unwrap()
    /// ).unwrap();
    /// ```
    // TODO: add sync_all_data, for supported operating systems
    pub fn sync_all(&self) -> io::Result<AioFut> {
        let aiocb = mio_aio::AioCb::from_fd(self.file.as_raw_fd(),
                            0,  //priority
                            );
        Ok(AioFut{
            op: AioOp::Fsync(PollEvented::new(aiocb)),
            state: AioState::Allocated })
    }
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

impl Future for AioFut {
    type Item = AioResult;
    type Error = nix::Error;

    fn poll(&mut self) -> Poll<AioResult, nix::Error> {
        let poll_result = match self.op {
                AioOp::Fsync(ref mut io) =>
                    io.poll_read_ready(UnixReady::aio().into()),
                AioOp::Read(ref mut io) =>
                    io.poll_read_ready(UnixReady::aio().into()),
                AioOp::Write(ref mut io) =>
                    io.poll_read_ready(UnixReady::aio().into()),
        }.unwrap();
        if poll_result == Async::NotReady {
            if self.state == AioState::Allocated {
                let r = match self.op {
                    AioOp::Fsync(ref pe) => pe.get_ref()
                        .fsync(mio_aio::AioFsyncMode::O_SYNC),
                    AioOp::Read(ref pe) => pe.get_ref().read(),
                    AioOp::Write(ref pe) => pe.get_ref().write(),
                };
                if r.is_err() {
                    return Err(r.unwrap_err());
                }
                self.state = AioState::InProgress;
            }
            return Ok(Async::NotReady);
        }
        let result = self.aio_return();
        let buf = match self.op {
            AioOp::Fsync(ref mut op) => op.get_mut().buf_ref(),
            AioOp::Read(ref mut op) => op.get_mut().buf_ref(),
            AioOp::Write(ref mut op) => op.get_mut().buf_ref(),
        };
        match result {
            Ok(x) => Ok(Async::Ready(AioResult{value: x, buf})),
            Err(x) => Err(x)
        }
    }
}
