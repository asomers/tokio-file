use nix::unistd::Uid;
use std::ffi::OsStr;
use std::fs;
use std::io::{IoSlice, IoSliceMut, Read, Write};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

macro_rules! t {
    ($e:expr) => (match $e {
        Ok(e) => e,
        Err(e) => panic!("{} failed with {:?}", stringify!($e), e),
    })
}

mod aio_file_ext {
    use tokio::fs::File;
    use tokio_file::AioFileExt;

    use super::*;

    #[tokio::test]
    async fn read_at() {
        const WBUF: &[u8] = b"abcdef";
        const EXPECT: &[u8] = b"cdef";
        let mut rbuf = [0; 4];
        let off = 2;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("read_at");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(WBUF).expect("write failed");
        let file = File::open(&path).await.unwrap();
        let r = file.read_at(&mut rbuf[..], off)
            .expect("read_at failed early").await
            .unwrap();
        assert_eq!(r, EXPECT.len());

        assert_eq!(&rbuf[..], EXPECT);
    }

    #[tokio::test]
    async fn readv_at() {
        const WBUF: &[u8] = b"abcdefghijklmnopqrwtuvwxyz";
        const EXPECT0: &[u8] = b"cdef";
        const EXPECT1: &[u8] = b"ghijklmn";
        let mut rbuf0 = [0; 4];
        let mut rbuf1 = [0; 8];
        {
            let mut rbufs = [IoSliceMut::new(&mut rbuf0[..]),
                IoSliceMut::new(&mut rbuf1[..])];
            let off = 2;

            let dir = TempDir::new().unwrap();
            let path = dir.path().join("readv_at");
            let mut f = fs::File::create(&path).unwrap();
            f.write_all(WBUF).expect("write failed");
            let file = File::open(&path).await.unwrap();
            let r = file.readv_at(&mut rbufs[..], off)
                .expect("readv_at failed early")
                .await
                .unwrap();
            assert_eq!(12, r);
        }

        assert_eq!(&rbuf0[..], EXPECT0);
        assert_eq!(&rbuf1[..], EXPECT1);
    }

    #[tokio::test]
    async fn sync_all() {
        const WBUF: &[u8] = b"abcdef";

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sync_all");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(WBUF).expect("write failed");
        let file = File::open(&path).await.unwrap();
        <File as AioFileExt>::sync_all(&file)
            .expect("sync_all failed early")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn write_at() {
        let wbuf = b"abcdef";
        let mut rbuf = Vec::new();

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("write_at");
        let file = File::create(&path).await.unwrap();
        let r = file.write_at(wbuf, 0)
            .expect("write_at failed early")
            .await
            .unwrap();
        assert_eq!(r, wbuf.len());

        let mut f = fs::File::open(&path).unwrap();
        let len = f.read_to_end(&mut rbuf).unwrap();
        assert_eq!(len, wbuf.len());
        assert_eq!(&wbuf[..], &rbuf[..]);
    }

    #[tokio::test]
    async fn write_at_static() {
        const WBUF: &[u8] = b"abcdef";
        let mut rbuf = Vec::new();

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("write_at");
        {
            let file = File::create(&path).await.unwrap();
            let r = file.write_at(WBUF, 0)
                .expect("write_at failed early")
                .await
                .unwrap();
            assert_eq!(r, WBUF.len());
        }

        let mut f = fs::File::open(&path).unwrap();
        let len = f.read_to_end(&mut rbuf).unwrap();
        assert_eq!(len, WBUF.len());
        assert_eq!(rbuf, WBUF);
    }

    #[tokio::test]
    async fn writev_at() {
        const EXPECT: &[u8] = b"abcdefghij";
        let wbuf0 = b"abcdef";
        let wbuf1 = b"ghij";
        let wbufs = [IoSlice::new(&wbuf0[..]), IoSlice::new(&wbuf1[..])];
        let mut rbuf = Vec::new();

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("writev_at");
        let file = File::create(&path).await.unwrap();
        let r = file.writev_at(&wbufs[..], 0)
            .expect("writev_at failed early")
            .await
            .unwrap();
        assert_eq!(r, wbuf0.len() + wbuf1.len());

        let mut f = fs::File::open(&path).unwrap();
        let len = f.read_to_end(&mut rbuf).unwrap();
        assert_eq!(len, EXPECT.len());
        assert_eq!(rbuf, EXPECT);
    }

    #[tokio::test]
    async fn writev_at_static() {
        const EXPECT: &[u8] = b"abcdefghi";
        const WBUF0: &[u8] = b"abcdef";
        const WBUF1: &[u8] = b"ghi";
        let wbufs = [IoSlice::new(WBUF0), IoSlice::new(WBUF1)];
        let mut rbuf = Vec::new();

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("writev_at_static");
        let file = File::create(&path).await.unwrap();
        let r = file.writev_at(&wbufs[..], 0)
            .expect("writev_at failed early")
            .await
            .unwrap();
        assert_eq!(r, WBUF0.len() + WBUF1.len());

        let mut f = fs::File::open(&path).unwrap();
        let len = f.read_to_end(&mut rbuf).unwrap();
        assert_eq!(len, EXPECT.len());
        assert_eq!(rbuf, EXPECT);
    }

}

#[allow(deprecated)]
mod file {
    use super::*;
    use tokio_file::File;

    #[test]
    fn metadata() {
        let wbuf = vec![0; 9000].into_boxed_slice();
        let dir = t!(TempDir::new());
        let path = dir.path().join("metadata");
        let mut f = t!(fs::File::create(&path));
        f.write_all(&wbuf).expect("write failed");
        let file = t!(File::open(path));
        let metadata = file.metadata().unwrap();
        assert_eq!(9000, metadata.len());
    }

    #[test]
    fn len() {
        let dir = t!(TempDir::new());
        let path = dir.path().join("len");
        let f = t!(fs::File::create(&path));
        f.set_len(9000).unwrap();
        let file = t!(File::open(path));
        assert_eq!(9000, file.len().unwrap());
    }

    /// Demonstrate use of `tokio_file::File::new` with user-controlled options.
    /// It should fail because the file doesn't already exist and `O_CREAT` is not
    /// specified.
    #[test]
    fn new_nocreat() {
        let dir = t!(TempDir::new());
        let path = dir.path().join("nonexistent");
        let r = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open(path)
            .map(tokio_file::File::new);
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn read_at() {
        const WBUF: &[u8] = b"abcdef";
        const EXPECT: &[u8] = b"cdef";
        let mut rbuf = [0; 4];
        let off = 2;

        let dir = t!(TempDir::new());
        let path = dir.path().join("read_at");
        let mut f = t!(fs::File::create(&path));
        f.write_all(WBUF).expect("write failed");
        let file = t!(File::open(&path));
        let r = file.read_at(&mut rbuf[..], off)
            .expect("read_at failed early").await
            .unwrap();
        assert_eq!(r, EXPECT.len());

        assert_eq!(&rbuf[..], EXPECT);
    }

    #[tokio::test]
    async fn readv_at() {
        const WBUF: &[u8] = b"abcdefghijklmnopqrwtuvwxyz";
        const EXPECT0: &[u8] = b"cdef";
        const EXPECT1: &[u8] = b"ghijklmn";
        let mut rbuf0 = [0; 4];
        let mut rbuf1 = [0; 8];
        {
            let mut rbufs = [IoSliceMut::new(&mut rbuf0[..]),
                IoSliceMut::new(&mut rbuf1[..])];
            let off = 2;

            let dir = t!(TempDir::new());
            let path = dir.path().join("readv_at");
            let mut f = t!(fs::File::create(&path));
            f.write_all(WBUF).expect("write failed");
            let file = t!(File::open(&path));
            let r = file.readv_at(&mut rbufs[..], off)
                .expect("readv_at failed early")
                .await
                .unwrap();
            assert_eq!(12, r);
        }

        assert_eq!(&rbuf0[..], EXPECT0);
        assert_eq!(&rbuf1[..], EXPECT1);
    }

    #[tokio::test]
    async fn sync_all() {
        const WBUF: &[u8] = b"abcdef";

        let dir = t!(TempDir::new());
        let path = dir.path().join("sync_all");
        let mut f = t!(fs::File::create(&path));
        f.write_all(WBUF).expect("write failed");
        let file = t!(File::open(&path));
        file.sync_all()
            .expect("sync_all failed early")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn write_at() {
        let wbuf = b"abcdef";
        let mut rbuf = Vec::new();

        let dir = t!(TempDir::new());
        let path = dir.path().join("write_at");
        let file = t!(File::open(&path));
        let r = file.write_at(wbuf, 0)
            .expect("write_at failed early")
            .await
            .unwrap();
        assert_eq!(r, wbuf.len());

        let mut f = t!(fs::File::open(&path));
        let len = t!(f.read_to_end(&mut rbuf));
        assert_eq!(len, wbuf.len());
        assert_eq!(&wbuf[..], &rbuf[..]);
    }

    #[tokio::test]
    async fn writev_at() {
        const EXPECT: &[u8] = b"abcdefghij";
        let wbuf0 = b"abcdef";
        let wbuf1 = b"ghij";
        let wbufs = [IoSlice::new(&wbuf0[..]), IoSlice::new(&wbuf1[..])];
        let mut rbuf = Vec::new();

        let dir = t!(TempDir::new());
        let path = dir.path().join("writev_at");
        let file = t!(File::open(&path));
        let r = file.writev_at(&wbufs[..], 0)
            .expect("writev_at failed early")
            .await
            .unwrap();
        assert_eq!(r, wbuf0.len() + wbuf1.len());

        let mut f = t!(fs::File::open(&path));
        let len = t!(f.read_to_end(&mut rbuf));
        assert_eq!(len, EXPECT.len());
        assert_eq!(rbuf, EXPECT);
    }

    #[tokio::test]
    async fn write_at_static() {
        const WBUF: &[u8] = b"abcdef";
        let mut rbuf = Vec::new();

        let dir = t!(TempDir::new());
        let path = dir.path().join("write_at");
        {
            let file = t!(File::open(&path));
            let r = file.write_at(WBUF, 0)
                .expect("write_at failed early")
                .await
                .unwrap();
            assert_eq!(r, WBUF.len());
        }

        let mut f = t!(fs::File::open(&path));
        let len = t!(f.read_to_end(&mut rbuf));
        assert_eq!(len, WBUF.len());
        assert_eq!(rbuf, WBUF);
    }

    #[tokio::test]
    async fn writev_at_static() {
        const EXPECT: &[u8] = b"abcdefghi";
        const WBUF0: &[u8] = b"abcdef";
        const WBUF1: &[u8] = b"ghi";
        let wbufs = [IoSlice::new(WBUF0), IoSlice::new(WBUF1)];
        let mut rbuf = Vec::new();

        let dir = t!(TempDir::new());
        let path = dir.path().join("writev_at_static");
        let file = t!(File::open(&path));
        let r = file.writev_at(&wbufs[..], 0)
            .expect("writev_at failed early")
            .await
            .unwrap();
        assert_eq!(r, WBUF0.len() + WBUF1.len());

        let mut f = t!(fs::File::open(&path));
        let len = t!(f.read_to_end(&mut rbuf));
        assert_eq!(len, EXPECT.len());
        assert_eq!(rbuf, EXPECT);
    }

    // Tests that work with device files
    mod dev {
        use super::*;

        use rstest::{fixture, rstest};
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        struct Md(PathBuf);
        impl Drop for Md {
            fn drop(&mut self) {
                Command::new("mdconfig")
                    .args(["-d", "-u"])
                    .arg(&self.0)
                    .output()
                    .expect("failed to deallocate md(4) device");
            }
        }

        #[fixture]
        fn md() -> Option<Md> {
            if Uid::current().is_root() {
                let output = Command::new("mdconfig")
                    .args(["-a", "-t",  "swap", "-s", "1m"])
                    .output()
                    .expect("failed to allocate md(4) device");
                // Strip the trailing "\n"
                let l = output.stdout.len() - 1;
                let mddev = OsStr::from_bytes(&output.stdout[0..l]);
                Some(Md(Path::new("/dev").join(mddev)))
            } else {
                None
            }
        }

        #[rstest]
        fn len(md: Option<Md>){
            if let Some(md) = md {
                let file = t!(File::open(md.0.as_path()));
                let len = file.len().unwrap();
                assert_eq!(len, 1_048_576);
            } else {
                println!("This test requires root privileges");
            }
        }
    }
}
