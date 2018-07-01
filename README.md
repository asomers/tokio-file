# tokio-file

A library for integrating file I/O with [tokio].  File I/O can be seamlessly
mixed with all other Future types within the Tokio reactor.

[tokio]: https://github.com/tokio-rs/tokio-core

```toml
# Cargo.toml
[depdendencies]
tokio = "0.1.5"
tokio-file = "0.3.0"
```

# Usage

See the `examples` directory in the repository.  In general, any program that's
already using `tokio` can add file I/O by using `tokio_file::File` and
running the resulting futures in the tokio reactor.

# Platforms

`tokio-file` version 0.3 works on FreeBSD, using the `mio-aio` crate..  It will
probably also work on DragonflyBSD and OSX.  It does not work on Linux.  The
`tokio-file` API can be supported on Linux, but it will need a completely
different backend.  Instead of using POSIX AIO as `mio-aio` does, Linux will
need a `mio-libaio` crate, that uses Linux's nonstandard libaio with an eventfd
for notifications.  That's the approach taken by [seastar].

[seastar]: http://www.seastar-project.org/

# License

`tokio-file` is primarily distributed under the terms of both the MIT license
and the Apache License (Version 2.0), with portions covered by various BSD-like
licenses.

See LICENSE-APACHE, and LICENSE-MIT for details.

