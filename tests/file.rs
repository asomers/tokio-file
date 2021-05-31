use galvanic_test::*;
use nix::unistd::Uid;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;
use tokio_file::File;
use tokio::runtime;

macro_rules! t {
    ($e:expr) => (match $e {
        Ok(e) => e,
        Err(e) => panic!("{} failed with {:?}", stringify!($e), e),
    })
}

#[test]
fn metadata() {
    let wbuf = vec![0; 9000].into_boxed_slice();
    let dir = t!(TempDir::new());
    let path = dir.path().join("metadata");
    let mut f = t!(fs::File::create(&path));
    f.write_all(&wbuf).expect("write failed");
    let file = t!(File::open(&path));
    let metadata = file.metadata().unwrap();
    assert_eq!(9000, metadata.len());
}

#[test]
fn len() {
    let dir = t!(TempDir::new());
    let path = dir.path().join("len");
    let f = t!(fs::File::create(&path));
    f.set_len(9000).unwrap();
    let file = t!(File::open(&path));
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
        .open(&path)
        .map(tokio_file::File::new);
    assert!(r.is_err());
}

#[test]
fn read_at() {
    const WBUF: &[u8] = b"abcdef";
    const EXPECT: &[u8] = b"cdef";
    let mut rbuf = vec![0; 4];
    let off = 2;

    let dir = t!(TempDir::new());
    let path = dir.path().join("read_at");
    let mut f = t!(fs::File::create(&path));
    f.write_all(WBUF).expect("write failed");
    let file = t!(File::open(&path));
    let mut rt = runtime::Runtime::new().unwrap();
    let r = rt.block_on(async {
        file.read_at(&mut rbuf[..], off).expect("read_at failed early").await
    }).unwrap();
    assert_eq!(r.value.unwrap() as usize, EXPECT.len());

    assert_eq!(&rbuf[..], EXPECT);
}

#[test]
fn readv_at() {
    const WBUF: &[u8] = b"abcdefghijklmnopqrwtuvwxyz";
    const EXPECT0: &[u8] = b"cdef";
    const EXPECT1: &[u8] = b"ghijklmn";
    let mut rbuf0 = vec![0; 4];
    let mut rbuf1 = vec![0; 8];
    {
        let mut rbufs = [&mut rbuf0[..], &mut rbuf1[..]];
        let off = 2;

        let dir = t!(TempDir::new());
        let path = dir.path().join("readv_at");
        let mut f = t!(fs::File::create(&path));
        f.write_all(WBUF).expect("write failed");
        let file = t!(File::open(&path));
        let mut rt = runtime::Runtime::new().unwrap();
        let r = rt.block_on(async {
            file.readv_at(&mut rbufs[..], off).expect("readv_at failed early")
                .await
        }).unwrap();
        assert_eq!(12, r);
    }

    assert_eq!(&rbuf0[..], EXPECT0);
    assert_eq!(&rbuf1[..], EXPECT1);
}

#[test]
fn sync_all() {
    const WBUF: &[u8] = b"abcdef";

    let dir = t!(TempDir::new());
    let path = dir.path().join("sync_all");
    let mut f = t!(fs::File::create(&path));
    f.write_all(WBUF).expect("write failed");
    let file = t!(File::open(&path));
    let mut rt = runtime::Runtime::new().unwrap();
    let r = rt.block_on(async {
        file.sync_all().expect("sync_all failed early").await
    }).unwrap();
    assert!(r.value.is_none());
}

#[test]
fn write_at() {
    let wbuf = b"abcdef";
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new());
    let path = dir.path().join("write_at");
    let file = t!(File::open(&path));
    let mut rt = runtime::Runtime::new().unwrap();
    let r = rt.block_on(async {
        file.write_at(wbuf, 0).expect("write_at failed early").await
    }).unwrap();
    assert_eq!(r.value.unwrap() as usize, wbuf.len());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, wbuf.len());
    assert_eq!(&wbuf[..], &rbuf[..]);
}

#[test]
fn writev_at() {
    const EXPECT: &[u8] = b"abcdefghij";
    let wbuf0 = b"abcdef";
    let wbuf1 = b"ghij";
    let wbufs = [&wbuf0[..], &wbuf1[..]];
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new());
    let path = dir.path().join("writev_at");
    let file = t!(File::open(&path));
    let mut rt = runtime::Runtime::new().unwrap();
    let r = rt.block_on(async {
        file.writev_at(&wbufs[..], 0).expect("writev_at failed early").await
    }).unwrap();
    assert_eq!(r, wbuf0.len() + wbuf1.len());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, EXPECT.len());
    assert_eq!(rbuf, EXPECT);
}

#[test]
fn write_at_static() {
    const WBUF: &[u8] = b"abcdef";
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new());
    let path = dir.path().join("write_at");
    {
        let file = t!(File::open(&path));
        let mut rt = runtime::Runtime::new().unwrap();
        let r = rt.block_on(async {
            file.write_at(WBUF, 0).expect("write_at failed early").await
        }).unwrap();
        assert_eq!(r.value.unwrap() as usize, WBUF.len());
    }

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, WBUF.len());
    assert_eq!(rbuf, WBUF);
}

#[test]
fn writev_at_static() {
    const EXPECT: &[u8] = b"abcdefghi";
    const WBUF0: &[u8] = b"abcdef";
    const WBUF1: &[u8] = b"ghi";
    let wbufs = [WBUF0, WBUF1];
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new());
    let path = dir.path().join("writev_at_static");
    let file = t!(File::open(&path));
    let mut rt = runtime::Runtime::new().unwrap();
    let r = rt.block_on(async {
        file.writev_at(&wbufs[..], 0).expect("writev_at failed early").await
    }).unwrap();
    assert_eq!(r, WBUF0.len() + WBUF1.len());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, EXPECT.len());
    assert_eq!(rbuf, EXPECT);
}

// Tests that work with device files
test_suite! {
    name dev;

    use super::*;

    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    fixture md() -> Option<PathBuf> {
        members {
            devname: Option<PathBuf>
        }
        setup(&mut self ) {
            self.devname = if Uid::current().is_root() {
                let output = Command::new("mdconfig")
                    .args(&["-a", "-t",  "swap", "-s", "1m"])
                    .output()
                    .expect("failed to allocate md(4) device");
                // Strip the trailing "\n"
                let l = output.stdout.len() - 1;
                let mddev = OsStr::from_bytes(&output.stdout[0..l]);
                Some(Path::new("/dev").join(&mddev))
            } else {
                None
            };
            self.devname.clone()
        }

        tear_down(&self) {
            if let Some(ref path) = self.devname {
                Command::new("mdconfig")
                    .args(&["-d", "-u"])
                    .arg(&path)
                    .output()
                    .expect("failed to deallocate md(4) device");
            }
        }
    }

    test len(md){
        if let Some(path) = md.val {
            let file = t!(File::open(&path));
            let len = file.len().unwrap();
            assert_eq!(len, 1_048_576);
        } else {
            println!("This test requires root privileges");
        }
    }

    // readv with unaligned buffers
    test readv_at(md) {
        if let Some(path) = md.val {
            let mut orig = vec![0u8; 2048];
            orig[100..512].copy_from_slice(&[1u8; 412]);
            orig[512..1024].copy_from_slice(&[2u8; 512]);
            orig[1024..1124].copy_from_slice(&[3u8; 100]);
            orig[1124..1224].copy_from_slice(&[4u8; 100]);
            orig[1224..1536].copy_from_slice(&[5u8; 312]);
            orig[1536..2048].copy_from_slice(&[6u8; 512]);
            let mut rbufs = [
                vec![0u8; 100],
                vec![0u8; 412],
                vec![0u8; 512],
                vec![0u8; 100],
                vec![0u8; 100],
                vec![0u8; 312],
                vec![0u8; 512],
            ];
            let mut rt = runtime::Runtime::new().unwrap();

            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("open failed")
                .write_all(&orig)
                .expect("write failed");

            {
                let file = t!(File::open(&path));
                let mut rslices = rbufs.iter_mut()
                    .map(|b| b.as_mut())
                    .collect::<Vec<_>>();
                let r = rt.block_on(async {
                    file.readv_at(&mut rslices[..], 0)
                        .expect("readv_at failed early")
                        .await
                }).unwrap();
                assert_eq!(2048, r);
            }

            assert_eq!(&rbufs[0][..], &orig[0..100]);
            assert_eq!(&rbufs[1][..], &orig[100..512]);
            assert_eq!(&rbufs[2][..], &orig[512..1024]);
            assert_eq!(&rbufs[3][..], &orig[1024..1124]);
            assert_eq!(&rbufs[4][..], &orig[1124..1224]);
            assert_eq!(&rbufs[5][..], &orig[1224..1536]);
            assert_eq!(&rbufs[6][..], &orig[1536..2048]);
        } else {
            println!("This test requires root privileges");
        }
    }

    // writev with unaligned buffers
    test writev_at(md) {
        if let Some(path) = md.val {
            let wbufs = [
                &vec![0u8; 100][..],
                &vec![1u8; 412][..],
                &vec![2u8; 512][..],
                &vec![3u8; 100][..],
                &vec![4u8; 100][..],
                &vec![5u8; 312][..],
                &vec![6u8; 512][..],
            ];
            let mut rt = runtime::Runtime::new().unwrap();
            let file = t!(File::open(&path));

            let r = rt.block_on(async {
                file.writev_at(&wbufs[..], 0).expect("writev_at failed early")
                    .await
            }).unwrap();
            assert_eq!(r, 2048);

            drop(file);

            let mut f = fs::File::open(&path).unwrap();
            let mut rbuf = vec![0u8; 2048];
            t!(f.read_exact(&mut rbuf));
            assert_eq!(wbufs[0], &rbuf[0..100]);
            assert_eq!(wbufs[1], &rbuf[100..512]);
            assert_eq!(wbufs[2], &rbuf[512..1024]);
            assert_eq!(wbufs[3], &rbuf[1024..1124]);
            assert_eq!(wbufs[4], &rbuf[1124..1224]);
            assert_eq!(wbufs[5], &rbuf[1224..1536]);
            assert_eq!(wbufs[6], &rbuf[1536..2048]);
        } else {
            println!("This test requires root privileges");
        }
    }
}
