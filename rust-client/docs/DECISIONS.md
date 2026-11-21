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
