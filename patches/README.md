# Patches for smithay

This fork needs one small change in smithay: async page flips for tearing (`set-tearing`).
The patches come from [angel-smithay](https://github.com/MixaDoDs/angel-smithay) (MIT), authorship is kept in their headers.

`Cargo.toml` points smithay at `../crabikOS-smithay`. To prepare that folder:

```sh
git clone https://github.com/Smithay/smithay.git ../crabikOS-smithay
git -C ../crabikOS-smithay checkout "$(cat patches/smithay-base-rev)"
git -C ../crabikOS-smithay apply "$PWD/patches/smithay-async-flip.patch"
git -C ../crabikOS-smithay apply "$PWD/patches/smithay-async-flip-log.patch"
cargo build --release
```
