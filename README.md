# tokio-aio

A library for integrating file I/O with [tokio].  File I/O can be seamlessly
mixed with all other Future types within the Tokio reactor.

[tokio]: https://github.com/tokio-rs/tokio-core

```toml
# Cargo.toml
[depdendencies]
mio-aio = "0.1"
mio = "0.7"
nix = "0.8"
tokio-core = "0.2"
```

# Usage

TODO

# Platforms

`tokio-file` version 0.1 works on FreeBSD, using the `mio-aio` crate..  It will
probably also work on DragonflyBSD and OSX.  It does not work on Linux.  The
`tokio-file` API can be supported on Linux, but it will need a completely
different backend.  Instead of using POSIX AIO as `mio-aio` does, Linux will
need a `mio-libaio` crate, that uses Linux's nonstandard libaio with a signalfd
for notifications.  That's the approach taken by [seastar].

[seastar]: http://www.seastar-project.org/

# License

`tokio-file` is primarily distributed under the terms of both the MIT license
and the Apache License (Version 2.0), with portions covered by various BSD-like
licenses.

See LICENSE-APACHE, and LICENSE-MIT for details.

