
## Install

1. Download the CLI archive for your computer, `rusttv-{version}-<target>.tar.gz`, and unpack it. On macOS the binaries are not notarized: run `xattr -d com.apple.quarantine rusttv rusttv-mcp` once.
2. On the TV, turn on network or wireless debugging, then `rusttv device connect <tv-ip>`.
3. Download the APK for the TV's ABI (`rusttv device inspect` shows it) and `rusttv launcher install --apk rusttv-launcher-{version}-<abi>.apk`. Optional: `rusttv launcher takeover` makes RustTV the Home screen; `rusttv launcher restore` undoes it.

Supported devices are the ones with a profile in [`compat/devices/`](https://github.com/damienmiheev/rusttv/tree/v{version}/compat/devices). On other TVs RustTV refuses changes their profile doesn't record as tested unless you pass `--allow-untested`.

## Check the files

- `SHA256SUMS` covers every file and is signed with the update signing key: `minisign -Vm SHA256SUMS -P {minisign_key}`. `rusttv update` checks this itself.
- APKs are signed with RustTV's release key. Certificate SHA-256: `ec16f7040c826d218cfd147c1fa49c4b376253c7b89b3d5e97ac707c8fd54473` (`apksigner verify --print-certs`).

## Safety

RustTV never roots, flashes or uninstalls system apps, and runs no free-form shell commands. Every change is planned, shown as a dry run, approved by a person, recorded and checked, and most can be undone with one command. See [SECURITY.md](https://github.com/damienmiheev/rusttv/blob/v{version}/SECURITY.md).

RustTV is an independent project, not affiliated with or endorsed by the Rust Foundation, the Rust Project or Google.
