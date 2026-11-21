# Progress

## Done
- M1: recon and `docs/PLAN.md` (reviewed; decisions in §8).
- Git: `origin` = Deafend88/ddnetrust, `upstream` = ddnet/ddnet (push URL disabled),
  branch `rust-client`.
- M2 varint + packer: `protocol/varint.rs`, `protocol/packer.rs` (incl. UTF-8 and
  sanitize helpers from `base/str.cpp`); 7,300 golden vectors from `rs_golden` pass.
- M2 Huffman: `protocol/huffman.rs`; 5,610 compress/decompress vectors pass, incl.
  garbage input and the wrapped unsigned bit-counter quirk (hit 23 times).
- M2 headers: `protocol/packet.rs` (chunk header, packet encode/decode, connless,
  `IsSeqInBackroom`); 13,299 vectors pass. Packet vectors are captured from the real
  `CNetBase::SendPacket*` over loopback UDP. 0.7 packets are rejected on receive
  (`docs/DECISIONS.md`).
- **M2 complete**: 26,000+ C++ golden vectors, all byte-identical.

## In progress
- Nothing.

## Next
- M3: headless client against a locally built `DDNet-Server`: protocol code generation
  from `datasrc/network.py`, UUID messages, sans-IO connection (`net/conn.rs`),
  handshake, map download, snapshots, inputs, disconnect.

## How to regenerate golden vectors
From a shell with the Visual Studio 18 x64 environment (`vcvars64.bat`), repo root:

    cmake -G Ninja -S . -B build-rs -DCMAKE_BUILD_TYPE=Release -DCLIENT=OFF -DDOWNLOAD_GTEST=OFF
    cmake --build build-rs --target rs_golden
    build-rs\rs_golden.exe rust-client\tests\golden

Golden files live in `rust-client/tests/golden/` (not `tests/data/`: the root
`.gitignore` ignores every `data/` directory).

## Known issues / blockers
- None.
