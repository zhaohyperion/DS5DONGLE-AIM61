# Opus firmware subset

This directory is derived from the official [xiph/opus 1.5.2](https://github.com/xiph/opus/tree/v1.5.2) source tree.

DS5DONGLE-AIM61 builds only the fixed-point CELT/SILK encoder and decoder sources listed in [`../opus.cmake`](../opus.cmake). To keep the firmware repository focused, the following upstream-only material is intentionally omitted:

- DNN/DRED training and evaluation data;
- upstream documentation, test vectors and fuzzers;
- Autotools, Meson and standalone upstream CMake build systems;
- ARM, MIPS, x86, Xtensa and floating-point optimized implementations.

The retained `COPYING`, `LICENSE_PLEASE_READ.txt`, `AUTHORS` and `README` files preserve the upstream copyright, license, patent and project information. Obtain the complete unmodified source tree from the xiph/opus `v1.5.2` tag.

The E907 fast paths in retained source files are downstream changes. Their history starts in the former Opus 1.2.1 M61 patch series preserved in Git commit `0968b719a38b5bfea951c8e87f3250ecac4e8fa8` and was ported with bit-exact tests to this 1.5.2 subset.
