extern crate divbuf;
extern crate futures;
extern crate nix;
extern crate sysctl;
extern crate tempdir;
extern crate tokio;
extern crate tokio_file;

use divbuf::DivBufShared;
use futures::future::lazy;
use futures::{Future, future};
use tempdir::TempDir;
use tokio_file::{AioResult, File};
use tokio::runtime::current_thread;

macro_rules! t {
    ($e:expr) => (match $e {
        Ok(e) => e,
        Err(e) => panic!("{} failed with {:?}", stringify!($e), e),
    })
}

// A write_at call fails with EAGAIN.  This test must run in its own process
// since it intentionally uses all of the system's AIO resources.
#[test]
fn write_at_eagain() {
    let limit = sysctl::value("vfs.aio.max_aio_queue_per_proc").unwrap();
    let count = if let sysctl::CtlValue::Int(x) = limit {
        (2 * x) as usize
    } else {
        panic!("sysctl: {:?}", limit);
    };

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at_eagain.0");
    let file = t!(File::open(&path));

    let dbses: Vec<_> = (0..count).map(|_| {
        DivBufShared::from(vec![0u8; 4096])
    }).collect();
    let futs : Vec<_> = (0..count).map(|i| {
        let wbuf = Box::new(dbses[i].try().unwrap());
        file.write_at(wbuf, 4096 * i as u64).unwrap()
            //future::join_all annoyingly cancels all remaining futures after
            //the first error, so we have to pack both the real ok and real
            //error types into a single "fake ok" type.
            .map(|r| -> Result<AioResult, nix::Error> {
                Ok(r)
            })
            .or_else(|e| -> Result<Result<AioResult, nix::Error>, ()> {
                Ok(Err(e))
            })
    }).collect();

    let mut rt = current_thread::Runtime::new().unwrap();
    let wi = t!(rt.block_on(lazy(|| {
        future::join_all(futs)
    })));

    let mut n_ok = 0;
    let mut n_eagain = 0;
    for i in 0..count {
        match wi[i].as_ref() {
            Ok(aio_result) => {
                n_ok += 1;
                assert_eq!(aio_result.value.unwrap(), 4096);
            },
            Err(nix::Error::Sys(nix::errno::Errno::EAGAIN)) => n_eagain += 1,
            Err(e) => panic!("unexpected result {:?}", e)
        }
    }
    // We should've been able to submit at least count / 2 operations.  But if
    // the test ran slowly and/or the storage system is fast, then we might've
    // been able to submit more.  If there wasn't a single EAGAIN, then the
    // testcase needs some work.
    assert!(n_ok >= count / 2);
    assert!(n_eagain > 1);
}
