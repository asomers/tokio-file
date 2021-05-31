//! A workalike of that old workhorse, dd.  It's basically the "Hello, World!"
//! of file I/O.
//!
//! You can try it out by running:
//!
//!     dd if=/dev/urandom of=/tmp/infile bs=4096 count=100
//!     cargo run --example dd -- -b 4096 -c 100 /tmp/infile /tmp/outfile
//!     cmp /tmp/infile /tmp/outfile

use divbuf::DivBufShared;
use futures::future::Future;
use futures::future::lazy;
use futures::{Stream, stream};
use getopts::Options;
use std::env;
use std::cell::Cell;
use std::mem;
use std::rc::Rc;
use std::str::FromStr;
use tokio::runtime::current_thread;
use tokio_file::File;

struct Dd {
    pub bs: usize,
    pub count: usize,
    pub infile: File,
    pub outfile: File,
    pub ofs: Cell<u64>,
}

impl Dd {
    pub fn new(infile: &str, outfile: &str, bs: usize, count: usize) -> Dd {
        let inf = File::open(infile);
        let outf = File::open(outfile);
        Dd {
            bs,
            count,
            infile: inf.unwrap(),
            outfile: outf.unwrap(),
            ofs: Cell::new(0)}
    }
}

fn usage(opts: Options) {
    let brief = "Usage: dd [options] <INFILE> <OUTFILE>";
    print!("{}", opts.usage(&brief));
}


fn main() {
    let mut opts = Options::new();
    opts.optopt("b", "blocksize", "Block size in bytes", "BS");
    opts.optopt("c", "count", "Number of blocks to copy", "COUNT");
    opts.optflag("h", "help", "print this help menu");
    let matches = match opts.parse(env::args().skip(1)) {
        Ok(m) => {m},
        Err(f) => { panic!("{}", f.to_string()) }
    };
    if matches.opt_present("h") {
        usage(opts);
        return;
    }
    let bs = match matches.opt_str("b") {
        Some(v) => usize::from_str(v.as_str()).unwrap(),
        None => 512
    };
    let count = match matches.opt_str("c") {
        Some(v) => usize::from_str(v.as_str()).unwrap(),
        None => 1
    };
    let infile = &matches.free[0];
    let outfile = &matches.free[1];

    let dd = Dd::new(infile.as_str(), outfile.as_str(), bs, count);
    let dbs = Rc::new(DivBufShared::from(vec![0; bs]));
    let mut rt = current_thread::Runtime::new().unwrap();
    rt.block_on(lazy(|| {
        // Note: this simple example will fail if infile isn't big enough.  A
        // robust program would use loop_fn instead of stream.for_each so it can
        // exit early.
        let stream = stream::iter_ok(0..dd.count);
        stream.for_each(|blocknum| {
            let rbuf = Box::new(dbs.try_mut().unwrap());
            let ofs = (dd.bs * blocknum) as u64;
            dd.infile.read_at(rbuf, ofs)
            .unwrap()
            .and_then(|r| {
                mem::drop(r);   // Release the mutable reference to dbs
                let wbuf = Box::new(dbs.try_const().unwrap());
                dd.outfile.write_at(wbuf, dd.ofs.get())
                .unwrap()
                .map(|r| {
                    dd.ofs.set(dd.ofs.get() + r.value.unwrap() as u64);
                })
            })
        })
    })).expect("Copying failed");
}
