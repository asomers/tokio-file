//! A workalike of that old workhorse, dd.  It's basically the "Hello, World!"
//! of file I/O.
//!
//! You can try it out by running:
//!
//!     dd if=/dev/urandom of=/tmp/infile bs=4096 count=100
//!     cargo run --example dd -- -b 4096 -c 100 /tmp/infile /tmp/outfile
//!     cmp /tmp/infile /tmp/outfile

use futures::{StreamExt, stream};
use getopts::Options;
use std::{
    cell::Cell,
    env,
    rc::Rc,
    str::FromStr};
use tokio::runtime;
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

    let dd = Rc::new(Dd::new(infile.as_str(), outfile.as_str(), bs, count));
    let mut rt = runtime::Runtime::new().unwrap();
    // Note: this simple example will fail if infile isn't big enough.  A
    // robust program would use try_for_each instead of for_each so it can
    // exit early.
    let stream = stream::iter(0..dd.count);
    rt.block_on(async {
        stream.for_each(|blocknum| {
            let ddc = dd.clone();
            async move {
                let mut rbuf = vec![0; bs];
                let ofs: u64 = (ddc.bs * blocknum) as u64;
                ddc.infile.read_at(&mut rbuf[..], ofs)
                .unwrap()
                .await
                .unwrap();
                let r = ddc.outfile.write_at(&rbuf[..], ddc.ofs.get())
                .unwrap()
                .await
                .unwrap();
                ddc.ofs.set(ddc.ofs.get() + r.value.unwrap() as u64);
            }
        }).await
    })
}
