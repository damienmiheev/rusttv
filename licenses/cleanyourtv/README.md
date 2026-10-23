# cleanyourtv package data

Most entries of the package catalog (`compat/packages/*.toml`) are derived from
the per-model package lists (`examples/*/debloat.txt`) and the `CATALOGUE` in
`tools/privacy.py` of cleanyourtv (https://github.com/Gathin23/cleanyourtv,
commit f9f0c53): which packages to consider, and the wording of many `what` and
`side_effects` notes. Each derived entry cites it in `sources`. The catalog is
compiled into `rusttv`, so the MIT licence notice is shipped with it.

`LICENSE` is the MIT licence from that repository, copied unmodified (without
its note on the Outfit font, which RustTV doesn't use).
