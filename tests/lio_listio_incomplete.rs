use futures::future;
use nix::unistd::{SysconfVar, sysconf};
use sysctl::CtlValue;
use tempfile::TempDir;
use tokio_file::File;
use tokio::runtime;

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
    let alm = sysconf(SysconfVar::AIO_LISTIO_MAX).expect("sysconf").unwrap();
    let maqpp = if let CtlValue::Int(x) = sysctl::value(
            "vfs.aio.max_aio_queue_per_proc").unwrap(){
        x
    } else {
        panic!("unknown sysctl");
    };
    // Find lio_listio sizes that satisfy the AIO_LISTIO_MAX constraint and also
    // result in a final lio_listio call that can only partially be queued
    let mut ops_per_listio = 0;
    let mut num_listios = 0;
    for i in (1..alm).rev() {
        let _ops_per_listio = f64::from(i as u32);
        let _num_listios = (f64::from(maqpp) / _ops_per_listio).ceil();
        let delayed = _ops_per_listio * _num_listios - f64::from(maqpp);
        if delayed > 0.01 {
            ops_per_listio = i as usize;
            num_listios = _num_listios as usize;
            break
        }
    }
    if num_listios == 0 {
        panic!("Can't find a configuration for max_aio_queue_per_proc={} AIO_LISTIO_MAX={}", maqpp, alm);
    }

    let dir = t!(TempDir::new());
    let path = dir.path().join("writev_at_eio");
    let file = t!(File::open(&path));
    let wbuf = vec![0u8; 4096];

    let mut rt = runtime::Runtime::new().unwrap();
    let wi = rt.block_on(async {
        let futs = (0..num_listios).map(|i| {
            let mut wbufs = Vec::with_capacity(ops_per_listio);
            for _ in 0..ops_per_listio {
                wbufs.push(&wbuf[..])
            }
            file.writev_at(&wbufs[..], 4096 * (i * ops_per_listio) as u64)
                .expect("writev_at failed early")
        });
        future::join_all(futs).await
    });

    for r in wi {
        assert_eq!(r.unwrap(), wbuf.len() * ops_per_listio);
    }
}
