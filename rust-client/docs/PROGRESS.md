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

- M3 codegen: `scripts/gen_protocol.py` turns `datasrc/network.py` (+ the system ex
  messages in `protocol_ex_msgs.h`) into `protocol/generated.rs`; `protocol/netobj.rs`
  ports `SecureUnpackObj`. Verified against C++: all 87 registered UUIDs, object sizes,
  1,520 object clamp/fail cases, 2,240 message accept/reject cases.

## In progress
- M3: snapshot port, connection state machine, client flow, local-server integration test.

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

## Local test server
`build-rs/DDNet-Server.exe` (CMake target `game-server`). Run it only with cwd set to a
scratch directory whose `storage.cfg` contains just `add_path $CURRENTDIR` and
`add_path $DATADIR`, so it never touches the real `%APPDATA%/DDNet`, and with
`sv_register 0`, `bindaddr 127.0.0.1`. Never connect to public servers until the user
says so.

## Regenerate protocol code
    python rust-client/scripts/gen_protocol.py   # needs rustfmt on PATH

## Known issues / blockers
- None.
