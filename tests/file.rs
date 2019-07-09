extern crate divbuf;
#[macro_use] extern crate galvanic_test;
extern crate nix;
extern crate futures;
extern crate tempdir;
extern crate tokio;
extern crate tokio_file;

use divbuf::DivBufShared;
use futures::future::lazy;
use nix::unistd::Uid;
use std::borrow::{Borrow, BorrowMut};
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use tempdir::TempDir;
use tokio_file::File;
use tokio::runtime::current_thread;

macro_rules! t {
    ($e:expr) => (match $e {
        Ok(e) => e,
        Err(e) => panic!("{} failed with {:?}", stringify!($e), e),
    })
}

#[test]
fn metadata() {
    let wbuf = vec![0; 9000].into_boxed_slice();
    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("read_at");
    let mut f = t!(fs::File::create(&path));
    f.write_all(&wbuf).expect("write failed");
    let file = t!(File::open(&path));
    let metadata = file.metadata().unwrap();
    assert_eq!(9000, metadata.len());
}

#[test]
fn len() {
    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("read_at");
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
    let dir = t!(TempDir::new("tokio-file"));
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
    let dbs = DivBufShared::from(vec![0; 4]);
    let rbuf = Box::new(dbs.try_mut().unwrap());
    let off = 2;

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("read_at_divbuf_mut");
    let mut f = t!(fs::File::create(&path));
    f.write_all(WBUF).expect("write failed");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let r = t!(rt.block_on(lazy(|| {
        file.read_at(rbuf, off).expect("read_at failed early")
    })));
    assert_eq!(r.value.unwrap() as usize, EXPECT.len());

    let mut buf_ref = r.into_buf_ref();
    let borrowed : &mut dyn BorrowMut<[u8]> = buf_ref.boxed_mut_slice()
                                                     .unwrap();
    assert_eq!(&borrowed.borrow_mut()[..], &EXPECT[..]);
}

#[test]
fn readv_at() {
    const WBUF: &[u8] = b"abcdefghijklmnopqrwtuvwxyz";
    const EXPECT0: &[u8] = b"cdef";
    const EXPECT1: &[u8] = b"ghijklmn";
    let dbs0 = DivBufShared::from(vec![0; 4]);
    let rbuf0 = Box::new(dbs0.try_mut().unwrap());
    let dbs1 = DivBufShared::from(vec![0; 8]);
    let rbuf1 = Box::new(dbs1.try_mut().unwrap());
    let rbufs : Vec<Box<dyn BorrowMut<[u8]>>> = vec![rbuf0, rbuf1];
    let off = 2;

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("readv_at");
    let mut f = t!(fs::File::create(&path));
    f.write_all(WBUF).expect("write failed");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let mut ri = t!(rt.block_on(lazy(|| {
        file.readv_at(rbufs, off).expect("readv_at failed early")
    })));

    let mut r0 = ri.next().unwrap();
    assert_eq!(r0.value.unwrap() as usize, EXPECT0.len());
    let b0 : &mut dyn BorrowMut<[u8]> = r0.buf.boxed_mut_slice().unwrap();
    assert_eq!(&b0.borrow_mut()[..], &EXPECT0[..]);

    let mut r1 = ri.next().unwrap();
    assert_eq!(r1.value.unwrap() as usize, EXPECT1.len());
    let b1 : &mut dyn BorrowMut<[u8]> = r1.buf.boxed_mut_slice().unwrap();
    assert_eq!(&b1.borrow_mut()[..], &EXPECT1[..]);

    assert!(ri.next().is_none());
}

#[test]
fn sync_all() {
    const WBUF: &[u8] = b"abcdef";

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("sync_all");
    let mut f = t!(fs::File::create(&path));
    f.write_all(WBUF).expect("write failed");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let r = t!(rt.block_on(lazy(|| {
        file.sync_all().expect("sync_all failed early")
    })));
    assert!(r.value.is_none());
}

#[test]
fn write_at() {
    let dbs = DivBufShared::from(&b"abcdef"[..]);
    let wbuf = Box::new(dbs.try().unwrap());
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let r = t!(rt.block_on(lazy(|| {
        file.write_at(wbuf.clone(), 0).expect("write_at failed early")
    })));
    assert_eq!(r.value.unwrap() as usize, wbuf.len());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, wbuf.len());
    assert_eq!(&wbuf[..], &rbuf[..]);
}

#[test]
fn writev_at() {
    const EXPECT: &[u8] = b"abcdefghij";
    let dbs0 = DivBufShared::from(&b"abcdef"[..]);
    let wbuf0 = Box::new(dbs0.try().unwrap());
    let dbs1 = DivBufShared::from(&b"ghij"[..]);
    let wbuf1 = Box::new(dbs1.try().unwrap());
    let wbufs : Vec<Box<dyn Borrow<[u8]>>> = vec![wbuf0, wbuf1];
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("writev_at");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let mut wi = t!(rt.block_on(lazy(|| {
        file.writev_at(wbufs, 0).expect("writev_at failed early")
    })));

    let w0 = wi.next().unwrap();
    assert_eq!(w0.value.unwrap() as usize, dbs0.len());
    let b0 : &dyn Borrow<[u8]> = w0.buf.boxed_slice().unwrap();
    assert_eq!(&dbs0.try().unwrap()[..], b0.borrow());

    let w1 = wi.next().unwrap();
    assert_eq!(w1.value.unwrap() as usize, dbs1.len());
    let b1 : &dyn Borrow<[u8]> = w1.buf.boxed_slice().unwrap();
    assert_eq!(&dbs1.try().unwrap()[..], b1.borrow());

    assert!(wi.next().is_none());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, EXPECT.len());
    assert_eq!(rbuf, EXPECT);
}

#[test]
fn write_at_static() {
    const WBUF: &[u8] = b"abcdef";
    let wbuf = Box::new(WBUF);
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at");
    {
        let file = t!(File::open(&path));
        let mut rt = current_thread::Runtime::new().unwrap();
        let r = t!(rt.block_on(lazy(|| {
            file.write_at(wbuf, 0).expect("write_at failed early")
        })));
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
    let wbuf0 = Box::new(WBUF0);
    let wbuf1 = Box::new(WBUF1);
    let wbufs : Vec<Box<dyn Borrow<[u8]>>> = vec![wbuf0, wbuf1];
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("writev_at_static");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let mut wi = t!(rt.block_on(lazy(|| {
        file.writev_at(wbufs, 0).expect("writev_at failed early")
    })));

    let w0 = wi.next().unwrap();
    assert_eq!(w0.value.unwrap() as usize, WBUF0.len());

    let w1 = wi.next().unwrap();
    assert_eq!(w1.value.unwrap() as usize, WBUF1.len());

    assert!(wi.next().is_none());

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
            let dbses = [
                DivBufShared::from(vec![0u8; 100]),
                DivBufShared::from(vec![0u8; 412]),
                DivBufShared::from(vec![0u8; 512]),
                DivBufShared::from(vec![0u8; 100]),
                DivBufShared::from(vec![0u8; 100]),
                DivBufShared::from(vec![0u8; 312]),
                DivBufShared::from(vec![0u8; 512]),
            ];
            let rbufs = dbses.iter().map(|dbs| {
                Box::new(dbs.try_mut().unwrap()) as Box<dyn BorrowMut<[u8]>>
            }).collect::<Vec<_>>();
            let mut rt = current_thread::Runtime::new().unwrap();
            let file = t!(File::open(&path));

            let mut ri = rt.block_on(lazy(|| {
                file.readv_at(rbufs, 0).expect("readv_at failed early")
            })).unwrap();

            for dbs in dbses.into_iter() {
                let rr = ri.next().unwrap();
                assert_eq!(rr.value.unwrap() as usize, dbs.len());
            }

            assert!(ri.next().is_none());
            drop(file);

            assert_eq!(&vec![0u8; 100][..], &orig[0..100]);
            assert_eq!(&vec![1u8; 412][..], &orig[100..512]);
            assert_eq!(&vec![2u8; 512][..], &orig[512..1024]);
            assert_eq!(&vec![3u8; 100][..], &orig[1024..1124]);
            assert_eq!(&vec![4u8; 100][..], &orig[1124..1224]);
            assert_eq!(&vec![5u8; 312][..], &orig[1224..1536]);
            assert_eq!(&vec![6u8; 512][..], &orig[1536..2048]);
        } else {
            println!("This test requires root privileges");
        }
    }

    // writev with unaligned buffers
    test writev_at(md) {
        if let Some(path) = md.val {
            let dbses = [
                DivBufShared::from(vec![0u8; 100]),
                DivBufShared::from(vec![1u8; 412]),
                DivBufShared::from(vec![2u8; 512]),
                DivBufShared::from(vec![3u8; 100]),
                DivBufShared::from(vec![4u8; 100]),
                DivBufShared::from(vec![5u8; 312]),
                DivBufShared::from(vec![6u8; 512]),
            ];
            let wbufs = dbses.iter().map(|dbs| {
                Box::new(dbs.try().unwrap()) as Box<dyn Borrow<[u8]>>
            }).collect::<Vec<_>>();
            let mut rt = current_thread::Runtime::new().unwrap();
            let file = t!(File::open(&path));

            let mut wi = rt.block_on(lazy(|| {
                file.writev_at(wbufs, 0).expect("writev_at failed early")
            })).unwrap();

            for dbs in dbses.into_iter() {
                let wbuf = dbs.try().unwrap();
                let wr = wi.next().unwrap();
                assert_eq!(wr.value.unwrap() as usize, wbuf.len());
                let b : &dyn Borrow<[u8]> = wr.buf.boxed_slice().unwrap();
                assert_eq!(&wbuf[..], b.borrow());
            }

            assert!(wi.next().is_none());
            drop(file);

            let mut f = fs::File::open(&path).unwrap();
            let mut rbuf = vec![0u8; 2048];
            t!(f.read_exact(&mut rbuf));
            assert_eq!(&vec![0u8; 100][..], &rbuf[0..100]);
            assert_eq!(&vec![1u8; 412][..], &rbuf[100..512]);
            assert_eq!(&vec![2u8; 512][..], &rbuf[512..1024]);
            assert_eq!(&vec![3u8; 100][..], &rbuf[1024..1124]);
            assert_eq!(&vec![4u8; 100][..], &rbuf[1124..1224]);
            assert_eq!(&vec![5u8; 312][..], &rbuf[1224..1536]);
            assert_eq!(&vec![6u8; 512][..], &rbuf[1536..2048]);
        } else {
            println!("This test requires root privileges");
        }
    }
}
