use std::{fs::File, io::ErrorKind};

use futures::future;
use sysctl::Sysctl;
use tempfile::TempDir;
use tokio_file::AioFileExt;

// A write_at call fails with EAGAIN.  This test must run in its own process
// since it intentionally uses all of the system's AIO resources.
#[tokio::test]
async fn write_at_eagain() {
    let ctl = sysctl::Ctl::new("vfs.aio.max_aio_queue_per_proc").unwrap();
    let limit = ctl.value().unwrap();
    let count = if let sysctl::CtlValue::Int(x) = limit {
        (2 * x) as usize
    } else {
        panic!("sysctl: {limit:?}");
    };

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("write_at_eagain.0");
    let file = File::create(path).unwrap();

    let wbuf = vec![0u8; 4096];

    let futs =
        (0..count).map(|i| file.write_at(&wbuf[..], 4096 * i as u64).unwrap());
    let results = future::join_all(futs).await;

    let mut n_ok = 0;
    let mut n_eagain = 0;
    for result in results {
        match result {
            Ok(aio_result) => {
                n_ok += 1;
                assert_eq!(aio_result, 4096);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => n_eagain += 1,
            Err(e) => panic!("unexpected result {e:?}"),
        }
    }
    // We should've been able to submit at least count / 2 operations.  But if
    // the test ran slowly and/or the storage system is fast, then we might've
    // been able to submit more.  If there wasn't a single EAGAIN, then the
    // testcase needs some work.
    assert!(n_ok >= count / 2);
    assert!(n_eagain > 1);
}
