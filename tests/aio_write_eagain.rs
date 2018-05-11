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
use tokio::executor::current_thread;
use tokio::reactor::Handle;

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
        (x + 1) as usize
    } else {
        panic!("sysctl: {:?}", limit);
    };

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at_eagain.0");
    let file = t!(File::open(&path, Handle::current()));

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

    let wi = t!(current_thread::block_on_all(lazy(|| {
        future::join_all(futs)
    })));

    for i in 0..count-1 {
        assert_eq!(wi[i].as_ref().unwrap().value.unwrap() as usize, 4096);
    }
    assert_eq!(wi[count - 1].as_ref().err().unwrap(),
               &nix::Error::Sys(nix::errno::Errno::EAGAIN));
}
