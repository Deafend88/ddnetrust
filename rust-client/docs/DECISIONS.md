# Decisions

Places where the Rust client deliberately differs from the C++ client, or where C++
behavior looks like a bug but is kept.

## 2026-11-21 — 0.7 packets are rejected on receive

C++: `CNetBase::UnpackPacket` (network.cpp:282). `CNetClient::Recv` passes a
`Sixup` flag by value, so a connected 0.6 client still parses any datagram whose
header marks it as 0.7 (connless with `buf[0] & 3 == 1`, or the `UNUSED` flag bit)
and feeds it to the connection. Real 0.6 servers never send these; in the golden
vectors they only come from random garbage (146 of 3,654 decode cases).

Rust: `protocol::packet::decode` returns `None` for them. 0.7 is out of scope
(PLAN §1), and dropping is the safe choice for unexpected input. Everything else
`decode` does matches C++ byte for byte.

## 2026-11-21 — `CUnpacker::GetString` returns a copy

C++ sanitizes the receive buffer in place and returns a pointer into it. Rust returns
a sanitized `String` and leaves the input untouched. No C++ caller reads the same
bytes again after `GetString`, so nothing observable changes.

## 2026-11-21 — C++ bug kept out: remote-controlled assert in `UnpackDelta`

C++: `CSnapshotDelta::UnpackDelta` (snapshot.cpp:513) bounds an item size only by the
remaining delta data (up to 128 KiB), then calls `CSnapshotBuilder::NewItemRaw`, whose
`dbg_assert` (snapshot.cpp:885) requires the size to fit a 64 KiB snapshot. A server can
therefore crash a C++ client with one crafted delta. This looks like a bug in C++.

Rust: `protocol::snapshot::unpack_delta` treats that case like the other "snapshot
full" case and returns `-302`, the code C++ would return if the assert were absent.
All non-crashing inputs behave identically (978 golden delta cases).

## 2026-11-21 — Validated ("alt") snapshot is a list, not a `CSnapshot`

C++: `CClient::UnpackAndValidateSnapshot` (client.cpp:2432) rebuilds the validated items
into a second `CSnapshot` and returns -4 if that builder runs out of space. Rust returns
a `Vec<Item>`. The builder can only overflow if validation grows the items past 64 KiB
(defaults filled in for short items); real snapshots are far below that, so the -4 path
is not reproduced.
