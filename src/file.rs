// vim: tw=80
use std::{
    fs,
    io::{self, IoSlice, IoSliceMut},
    mem,
    os::unix::{
        fs::FileTypeExt,
        io::{AsRawFd, RawFd}
    },
    path::Path,
    pin::Pin
};
use futures::{
    Future,
    task::{Context, Poll}
};
use mio_aio::AioFsyncMode;
use nix::errno::Errno;
use tokio::io::bsd::{AioSource, Aio};

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

fn conv_poll_err<T>(e: io::Error) -> Poll<Result<T, nix::Error>> {
    let raw = e.raw_os_error().unwrap_or(0);
    let errno = Errno::from_i32(raw);
    Poll::Ready(Err(errno))
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
    type Output = Result<T::Output, nix::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let poll_result = self.0.poll_ready(cx);
        match poll_result {
            Poll::Pending => {
                if !self.0.0.in_progress() {
                    let p = unsafe { self.map_unchecked_mut(|s| &mut s.0.0) };
                    match p.submit() {
                        Ok(()) => (),
                        Err(e) => return Poll::Ready(Err(e))
                    }
                }
                Poll::Pending
            },
            Poll::Ready(Err(e)) => conv_poll_err(e),
            Poll::Ready(Ok(_ev)) => {
                // At this point, we could clear readiness.  But there's no
                // point, since we're about to drop the Aio.
                let p = unsafe { self.map_unchecked_mut(|s| &mut s.0.0) };
                let result = p.aio_return();
                match result {
                    Ok(r) => Poll::Ready(Ok(r)),
                    Err(e) => Poll::Ready(Err(e)),
                }
            }
        }
    }
}

/// Return type of [`File::read_at`].  Implements `Future`.
pub type ReadAt<'a> = TokioFileFut<mio_aio::ReadAt<'a>>;

/// Return type of [`File::readv_at`].  Implements `Future`.
pub type ReadvAt<'a> = TokioFileFut<mio_aio::ReadvAt<'a>>;

/// Return type of [`File::sync_all`].  Implements `Future`.
pub type SyncAll = TokioFileFut<mio_aio::Fsync>;

/// Return type of [`File::write_at`].  Implements `Future`.
pub type WriteAt<'a> = TokioFileFut<mio_aio::WriteAt<'a>>;

/// Return type of [`File::writev_at`].  Implements `Future`.
pub type WritevAt<'a> = TokioFileFut<mio_aio::WritevAt<'a>>;

/// Basically a Tokio file handle.  This is the starting point for tokio-file.
#[derive(Debug)]
pub struct File {
    file: fs::File,
    /// The preferred (not necessarily minimum) sector size for accessing
    /// the device
    sectorsize: usize
}

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
            .map(File::new)
    }

    /// Asynchronous equivalent of `std::fs::File::read_at`
    ///
    /// # Examples
    ///
    /// ```
    /// use std::fs;
    /// use std::io::Write;
    /// use tempfile::TempDir;
    /// use tokio::runtime;
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
    /// let rt = runtime::Builder::new_current_thread()
    ///     .enable_io()
    ///     .build()
    ///     .unwrap();
    /// let r = rt.block_on(async {
    ///     file.read_at(&mut rbuf[..], 2).unwrap().await
    /// }).unwrap();
    /// assert_eq!(&rbuf[..], &EXPECT[..]);
    /// ```
    pub fn read_at<'a>(&self, buf: &'a mut [u8], offset: u64)
        -> io::Result<ReadAt<'a>>
    {
        let fd = self.file.as_raw_fd();
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
    /// use tokio::runtime;
    ///
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
    ///     .open(&path)
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let rt = runtime::Builder::new_current_thread()
    ///     .enable_io()
    ///     .build()
    ///     .unwrap();
    /// let mut r = rt.block_on(async {
    ///     file.readv_at(&mut rbufs[..], 2).unwrap().await
    /// }).unwrap();
    ///
    /// assert_eq!(l0 + l1, r);
    /// assert_eq!(&rbuf0[..], &EXPECT0[..]);
    /// assert_eq!(&rbuf1[..], &EXPECT1[..]);
    /// ```
    pub fn readv_at<'a>(&self, bufs: &'a mut [IoSliceMut<'a>],
                        offset: u64) -> io::Result<ReadvAt<'a>>
    {
        let fd = self.file.as_raw_fd();
        let source = TokioSource(mio_aio::ReadvAt::readv_at(fd, offset, bufs, 0));
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
    /// use tokio::runtime;
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
    /// let rt = runtime::Builder::new_current_thread()
    ///     .enable_io()
    ///     .build()
    ///     .unwrap();
    /// let r = rt.block_on(async {
    ///     file.sync_all().unwrap().await
    /// }).unwrap();
    /// ```
    // TODO: add sync_all_data, for supported operating systems
    pub fn sync_all(&self) -> io::Result<SyncAll> {
        let mode = AioFsyncMode::O_SYNC;
        let fd = self.file.as_raw_fd();
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
    /// use tokio::runtime;
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
    /// let rt = runtime::Builder::new_current_thread()
    ///     .enable_io()
    ///     .build()
    ///     .unwrap();
    /// let r = rt.block_on(async {
    ///     file.write_at(contents, 0).unwrap().await
    /// }).unwrap();
    /// assert_eq!(r, contents.len());
    /// drop(file);
    ///
    /// let mut file = fs::File::open(&path).unwrap();
    /// assert_eq!(file.read_to_end(&mut rbuf).unwrap(), contents.len());
    /// assert_eq!(&contents[..], &rbuf[..]);
    /// ```
    pub fn write_at<'a>(
        &self,
        buf: &'a [u8],
        offset: u64
    ) -> io::Result<WriteAt<'a>>
    {
        let fd = self.file.as_raw_fd();
        let source = TokioSource(mio_aio::WriteAt::write_at(fd, offset, buf, 0));
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
    /// use tokio::runtime;
    ///
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
    ///     .map(tokio_file::File::new)
    ///     .unwrap();
    /// let rt = runtime::Builder::new_current_thread()
    ///     .enable_io()
    ///     .build()
    ///     .unwrap();
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
    pub fn writev_at<'a>(&self, bufs: &[IoSlice<'a>], offset: u64)
        -> io::Result<WritevAt<'a>>
    {
        let fd = self.file.as_raw_fd();
        let source = TokioSource(mio_aio::WritevAt::writev_at(fd, offset, bufs, 0));
        Ok(TokioFileFut(Aio::new_for_aio(source)?))
    }
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

// LCOV_EXCL_START
#[cfg(test)]
mod t {
    use std::fs;
    use tempfile::TempDir;
    use tokio::runtime;
    use super::*;

    /// Pet grcov
    #[test]
    fn debug_futures() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo");
        let mut rbuf0 = [];
        let mut rbuf1 = [];
        let wbuf0 = [];
        let wbuf1 = [];
        let wbufs = [IoSlice::new(&wbuf0), IoSlice::new(&wbuf1)];

        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&path)
            .map(File::new)
            .unwrap();
        format!("{:?}", file);
        let rt = runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        rt.block_on(async {
            format!("{:?}", file.sync_all().unwrap());
            format!("{:?}", file.read_at(&mut rbuf0, 0).unwrap());
            let mut rbufs = [IoSliceMut::new(&mut rbuf0),
                             IoSliceMut::new(&mut rbuf1)];
            format!("{:?}", file.readv_at(&mut rbufs, 0).unwrap());
            format!("{:?}", file.write_at(&wbuf0, 0).unwrap());
            format!("{:?}", file.writev_at(&wbufs, 0).unwrap());
        });
    }
}
// LCOV_EXCL_STOP
