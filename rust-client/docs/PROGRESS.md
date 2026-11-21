# Progress

## Done
- M1: recon of network, packer, huffman, snapshot, datafile, layers, gamecore,
  collision, client prediction; `docs/PLAN.md` written and reviewed (decisions in §8).
- Git set up: `origin` = Deafend88/ddnetrust, `upstream` = ddnet/ddnet, branch `rust-client`.

## In progress
- Nothing.

## Next
- M2: workspace skeleton, `rs_golden` C++ tool, varint/packer/huffman/packet/chunk
  ports with golden-vector tests.

## Known issues / blockers
- C++ build needs a Visual Studio 18 developer environment (CMake not on PATH).
- context7 must be authorized (`/mcp`) before writing code against crates.
