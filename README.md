# ascend-nockchain-miner

Minimal Nockchain AI-PoW miner backend for Huawei Ascend 910B / CANN.

Only the mining wrapper is kept here. Nockchain's `ai-pow` and `ai-pow-miner` crates are pulled directly from the upstream `master` branch at build time, so this repository does not vendor the Nockchain monorepo. Upstream currently exposes AI-PoW through `crates/ai-pow` and `crates/ai-pow-miner`.

## Files

```text
src/main.rs                  miner CLI + SearchBackend + FFI
ascend/miner.asc             CANN host + Cube matmul + fold kernel
ascend/stub.c                CPU oracle/ABI test backend
ascend/CMakeLists.txt        AscendC build
build.rs                     Cargo native build glue
Makefile                     test/build/package
.github/workflows/prerelease.yml
Cargo.toml
rust-toolchain.toml
.gitignore
README.md
```

11 files total.

## Mining path

```text
Nockchain job
 -> canonical attempt preparation in upstream Rust
 -> prepared INT8 strips
 -> Ascend 910B Cube matmul
 -> fold to TileState[16]
 -> upstream jackpot hash / target check
 -> upstream certificate + submission
```

The v0 CANN kernel supports the rank-64 `h=8, w=8, k=1024` path.

## Commands

Hardware-free interface/oracle test:

```bash
make test
```

910B build:

```bash
make build
```

If CANN is elsewhere:

```bash
make build CANN_ROOT=/path/to/ascend-toolkit/latest
```

Build + release tarball:

```bash
make release
```

Run:

```bash
./target/release/ascend-nockchain-miner --help
```

## GitHub prerelease

Pushes to `main` run the CPU oracle test first, then dispatch the real CANN build to a self-hosted runner labeled:

```text
self-hosted, linux, ARM64, ascend910b
```

A successful build creates `prerelease-<run number>` and uploads the ARM64/910B tarball plus SHA256.
