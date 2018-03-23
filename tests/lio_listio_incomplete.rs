extern crate divbuf;
extern crate futures;
extern crate sysctl;
extern crate tempdir;
extern crate tokio;
extern crate tokio_file;

use divbuf::DivBufShared;
use futures::future::lazy;
use futures::Future;
use std::borrow::Borrow;
use tempdir::TempDir;
use tokio_file::File;
use tokio::executor::current_thread;
use tokio::reactor::Handle;

macro_rules! t {
    ($e:expr) => (match $e {
        Ok(e) => e,
        Err(e) => panic!("{} failed with {:?}", stringify!($e), e),
    })
}

// A writev_at call fails because lio_listio(2) returns EIO.  That means that
// some of the AioCbs may have been initiated, but not all.
// This test must run in its own process since it intentionally uses all of the
// system's AIO resources.
#[test]
fn writev_at_eio() {
    let limit = sysctl::value("vfs.aio.max_aio_queue_per_proc").unwrap();
    let count = if let sysctl::CtlValue::Int(x) = limit {
        // This should cause the second lio_listio call to return incomplete
        (x * 3 / 4) as usize
    } else {
        panic!("sysctl: {:?}", limit);
    };

    let dir = t!(TempDir::new("tokio-file"));
    let path0 = dir.path().join("writev_at_eio.0");
    let file0 = t!(File::open(&path0, Handle::current()));
    let path1 = dir.path().join("writev_at_eio.1");
    let file1 = t!(File::open(&path1, Handle::current()));

    let mut wbufs0: Vec<Box<Borrow<[u8]>>> = Vec::with_capacity(count);
    let dbses0: Vec<_> = (0..count).map(|_| DivBufShared::from(vec![0u8; 4096])).collect();
    let mut wbufs1: Vec<Box<Borrow<[u8]>>> = Vec::with_capacity(count);
    let dbses1: Vec<_> = (0..count).map(|_| DivBufShared::from(vec![0u8; 4096])).collect();
    for i in 0..count {
        let wbuf0 = dbses0[i].try().unwrap();
        wbufs0.push(Box::new(wbuf0));
        let wbuf1 = dbses1[i].try().unwrap();
        wbufs1.push(Box::new(wbuf1));
    }

    let mut wi = t!(current_thread::block_on_all(lazy(|| {
        let fut0 = file0.writev_at(wbufs0, 0).ok().expect("writev_at failed early");
        let fut1 = file1.writev_at(wbufs1, 0).ok().expect("writev_at failed early");
        fut0.join(fut1)
    })));

    for _ in 0..count {
        let result = wi.0.next().unwrap();
        assert_eq!(result.value.unwrap() as usize, 4096);
    }
    assert!(wi.0.next().is_none());
    for _ in 0..count {
        let result = wi.1.next().unwrap();
        assert_eq!(result.value.unwrap() as usize, 4096);
    }
    assert!(wi.1.next().is_none());
}


