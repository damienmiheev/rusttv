# LLVM runtime libraries from the NDK

The launcher library statically links parts of the NDK's LLVM runtimes:
libc++ and libc++abi (`c++_static`, `c++abi`, linked for Skia by
`skia-bindings`) and libunwind (Rust's unwinder on Android). These are under the
Apache License 2.0 with LLVM Exceptions; older parts are also under the legacy
University of Illinois/NCSA licence, which asks for its notice in binary
distributions.

`LICENSE.TXT` is the LLVM Project section of the NDK's `NOTICE.toolchain`
(NDK 30.0.16248370, lines 1833–2110), copied unmodified. Recheck it when the
NDK is upgraded.
