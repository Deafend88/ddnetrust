// Writes golden vectors produced by the C++ engine code for the Rust client
// (rust-client/). Every value here comes from calling the real C++ functions;
// nothing is reimplemented. Output is plain text, one case per line, hex for bytes
// ("-" for empty).
//
// Usage: rs_golden <output_dir>

#include <base/dbg.h>
#include <base/logger.h>
#include <base/mem.h>
#include <base/net.h>
#include <base/os.h>
#include <base/math.h>
#include <base/str.h>
#include <base/time.h>

#include <engine/shared/compression.h>
#include <engine/shared/huffman.h>
#include <engine/shared/network.h>
#include <engine/shared/packer.h>
#include <engine/shared/snapshot.h>
#include <engine/shared/uuid_manager.h>

#include <generated/protocol.h>

#include <chrono>
#include <cstdio>
#include <string>
#include <vector>

static constexpr const char *const TOOL_NAME = "rs_golden";

typedef std::vector<unsigned char> Bytes;

// Deterministic generator so the vectors are reproducible.
static unsigned s_Seed = 12345;
static unsigned Rand()
{
	s_Seed = s_Seed * 1103515245u + 12345u;
	return s_Seed >> 8;
}

static Bytes RandBytes(int Size)
{
	Bytes v(Size);
	for(auto &b : v)
		b = Rand() & 0xff;
	return v;
}

// Bytes skewed towards the distribution of real network chunks (small ints,
// zeros, ASCII), which exercises the short Huffman codes.
static Bytes RandSkewed(int Size)
{
	Bytes v(Size);
	for(auto &b : v)
	{
		unsigned r = Rand() % 10;
		b = r < 4 ? 0 : r < 7 ? Rand() % 16 : r < 9 ? 'a' + Rand() % 26 : Rand() & 0xff;
	}
	return v;
}

static std::string Hex(const unsigned char *p, int n)
{
	if(n <= 0)
		return "-";
	std::string s;
	char aBuf[3];
	for(int i = 0; i < n; i++)
	{
		str_format(aBuf, sizeof(aBuf), "%02x", p[i]);
		s += aBuf;
	}
	return s;
}
static std::string Hex(const Bytes &v) { return Hex(v.data(), (int)v.size()); }

static FILE *Open(const char *pDir, const char *pName)
{
	char aPath[512];
	str_format(aPath, sizeof(aPath), "%s/%s", pDir, pName);
	FILE *f = fopen(aPath, "wb");
	if(!f)
		log_error(TOOL_NAME, "cannot open %s", aPath);
	return f;
}

static void GenVarint(FILE *f)
{
	// pack <int> <hex>
	std::vector<int> vValues = {0, 1, -1, 63, -63, 64, -64, 65, -65, 127, -128, 8191, 8192, -8192, -8193,
		1048575, 1048576, -1048577, 134217727, 134217728, -134217729, 2147483647, (int)-2147483647 - 1};
	for(int k = 0; k < 31; k++)
	{
		vValues.push_back(1 << k);
		vValues.push_back((1 << k) - 1);
		vValues.push_back(-(1 << k));
	}
	for(int i = 0; i < 500; i++)
		vValues.push_back((int)(Rand() ^ (Rand() << 16)));
	for(int v : vValues)
	{
		unsigned char aBuf[CVariableInt::MAX_BYTES_PACKED];
		unsigned char *pEnd = CVariableInt::Pack(aBuf, v, sizeof(aBuf));
		fprintf(f, "pack %d %s\n", v, Hex(aBuf, (int)(pEnd - aBuf)).c_str());
	}

	// unpack <hexin> <value|ERR> <consumed>
	std::vector<Bytes> vInputs;
	for(int b = 0; b < 256; b++)
	{
		vInputs.push_back({(unsigned char)b});
		vInputs.push_back({(unsigned char)b, 0x7f});
	}
	for(int i = 0; i < 2000; i++)
	{
		Bytes v = RandBytes(1 + Rand() % 6);
		// bias towards long sequences with extend bits set
		for(size_t k = 0; k + 1 < v.size(); k++)
			if(Rand() % 4)
				v[k] |= 0x80;
		vInputs.push_back(v);
	}
	for(const Bytes &v : vInputs)
	{
		int Value = 0;
		const unsigned char *pEnd = CVariableInt::Unpack(v.data(), &Value, (int)v.size());
		if(pEnd)
			fprintf(f, "unpack %s %d %d\n", Hex(v).c_str(), Value, (int)(pEnd - v.data()));
		else
			fprintf(f, "unpack %s ERR 0\n", Hex(v).c_str());
	}
}

// A packer op list line: "pack <ops> <hexout> <error>" where ops are separated by
// ',' and are "i:<int>", "s:<limit>:<trunc>:<hex>" or "r:<hex>".
static void GenPacker(FILE *f)
{
	std::vector<Bytes> vStrings = {
		{},
		{'a'},
		{'h', 'e', 'l', 'l', 'o'},
		{0xc3, 0xa4}, // ä
		{0xe2, 0x82, 0xac}, // €
		{0xf0, 0x9f, 0x98, 0x80}, // emoji
		{0xc3}, // truncated
		{0xff, 'a'}, // invalid start
		{0xc3, 'a'}, // invalid continuation
		{0xe0, 0x80, 0x80}, // overlong
		{0xed, 0xa0, 0x80}, // surrogate
		{0xf4, 0x90, 0x80, 0x80}, // above U+10FFFF
		{0x80, 0x80},
		{'a', 0x01, 0x0a, 0x0d, 0x09, 0x1f, 'b'},
	};
	for(int i = 0; i < 200; i++)
	{
		Bytes v = RandBytes(Rand() % 12);
		for(auto &b : v)
			if(b == 0)
				b = 'z';
		vStrings.push_back(v);
	}
	int aLimits[] = {0, 1, 2, 3, 4, 5, 7, 16, -5};
	for(const Bytes &s : vStrings)
	{
		std::string Str(s.begin(), s.end());
		for(int Limit : aLimits)
		{
			for(int Trunc = 0; Trunc < 2; Trunc++)
			{
				CPacker p;
				p.Reset();
				p.AddString(Str.c_str(), Limit, Trunc);
				p.AddInt(7); // shows whether the packer is in the error state
				fprintf(f, "pack s:%d:%d:%s,i:7 %s %d\n", Limit, Trunc, Hex(s).c_str(), Hex(p.Data(), p.Size()).c_str(), p.Error());
			}
		}
	}

	// Filling the 2048-byte buffer.
	{
		CPacker p;
		p.Reset();
		std::string Ops;
		for(int i = 0; i < 420; i++)
		{
			p.AddInt(-2147483647 - 1);
			Ops += i ? ",i:-2147483648" : "i:-2147483648";
		}
		fprintf(f, "pack %s %s %d\n", Ops.c_str(), Hex(p.Data(), p.Size()).c_str(), p.Error());
	}
	{
		CPacker p;
		p.Reset();
		Bytes Raw = RandBytes(2040);
		p.AddRaw(Raw.data(), (int)Raw.size());
		std::string Long(30, 'x');
		p.AddString(Long.c_str(), 0, true);
		p.AddRaw(Raw.data(), 1);
		fprintf(f, "pack r:%s,s:0:1:%s,r:%s %s %d\n", Hex(Raw).c_str(), Hex((const unsigned char *)Long.c_str(), 30).c_str(),
			Hex(Raw.data(), 1).c_str(), Hex(p.Data(), p.Size()).c_str(), p.Error());
	}
	{
		CPacker p;
		p.Reset();
		Bytes Raw = RandBytes(2045);
		p.AddRaw(Raw.data(), (int)Raw.size());
		const char *pShort = "ab";
		p.AddString(pShort, 0, true); // needs 3 bytes incl. NUL: fits exactly
		p.AddInt(1);
		fprintf(f, "pack r:%s,s:0:1:6162,i:1 %s %d\n", Hex(Raw).c_str(), Hex(p.Data(), p.Size()).c_str(), p.Error());
	}

	// Unpacker: "unpack <hexin> <ops> <results>" where ops (',') are "i", "d<default>",
	// "u", "ud<default>", "s<flags>", "r<size>"; results (',') are ints, hex strings or
	// raw hex ("N" for a null raw pointer); last result is the error flag.
	struct SCase
	{
		Bytes m_In;
		std::string m_Ops;
	};
	std::vector<SCase> vCases = {
		{{}, "i"},
		{{}, "d5"},
		{{}, "s1"},
		{{}, "r0"},
		{{}, "r1"},
		{{}, "u"},
		{{}, "ud9"},
		{{0x01}, "i,d5"},
		{{0x01}, "i,i"},
		{{0x80}, "i"},
		{{'a', 'b', 0}, "s0,i"},
		{{'a', 'b'}, "s0"},
		{{' ', ' ', 'a', 0}, "s4"},
		{{' ', 0xc2, 0xa0, 'a', 0}, "s4"},
		{{0xe3, 0x80, 0x80, 'x', 0}, "s5"},
		{{'a', 0x01, 0x0a, 0x0d, 0x09, 0x7f, 0}, "s0"},
		{{'a', 0x01, 0x0a, 0x0d, 0x09, 0x7f, 0}, "s1"},
		{{'a', 0x01, 0x0a, 0x0d, 0x09, 0x7f, 0}, "s2"},
		{{0xff, 0}, "s0,i"},
		{{0xc3, 0xa4, 0}, "s1"},
		{{1, 2, 3, 4, 5}, "u,i"},
		{{1, 2, 3}, "u"},
		{{1, 2, 3, 4}, "r2,r2,r1"},
		{{1, 2, 3, 4}, "r-1"},
		{{0x40, 0x7f, 0}, "i,s0"},
	};
	for(int i = 0; i < 300; i++)
	{
		std::string Ops;
		int n = 1 + Rand() % 5;
		for(int k = 0; k < n; k++)
		{
			const char *apOps[] = {"i", "d3", "u", "ud-2", "s0", "s1", "s2", "s4", "s5", "r2"};
			Ops += k ? "," : "";
			Ops += apOps[Rand() % 10];
		}
		Bytes In = RandSkewed(Rand() % 16);
		vCases.push_back({In, Ops});
	}
	for(SCase &Case : vCases)
	{
		Bytes Buf = Case.m_In; // GetString sanitizes in place
		Buf.reserve(1); // data() of an empty vector may be nullptr; real buffers never are
		CUnpacker u;
		u.Reset(Buf.data(), (int)Buf.size());
		std::string Results;
		const char *pOps = Case.m_Ops.c_str();
		while(*pOps)
		{
			char aOp[16];
			int Len = 0;
			while(pOps[Len] && pOps[Len] != ',')
				Len++;
			str_truncate(aOp, sizeof(aOp), pOps, Len);
			pOps += Len + (pOps[Len] == ',');
			char aRes[4096];
			if(aOp[0] == 'i')
				str_format(aRes, sizeof(aRes), "%d", u.GetInt());
			else if(aOp[0] == 'd')
				str_format(aRes, sizeof(aRes), "%d", u.GetIntOrDefault(str_toint(aOp + 1)));
			else if(aOp[0] == 'u' && aOp[1] == 'd')
				str_format(aRes, sizeof(aRes), "%d", u.GetUncompressedIntOrDefault(str_toint(aOp + 2)));
			else if(aOp[0] == 'u')
				str_format(aRes, sizeof(aRes), "%d", u.GetUncompressedInt());
			else if(aOp[0] == 's')
			{
				const char *pStr = u.GetString(str_toint(aOp + 1));
				str_copy(aRes, Hex((const unsigned char *)pStr, str_length(pStr)).c_str());
			}
			else if(aOp[0] == 'r')
			{
				int Size = str_toint(aOp + 1);
				const unsigned char *pRaw = u.GetRaw(Size);
				str_copy(aRes, pRaw ? Hex(pRaw, Size).c_str() : "N");
			}
			Results += aRes;
			Results += ",";
		}
		Results += u.Error() ? "1" : "0";
		fprintf(f, "unpack %s %s %s\n", Hex(Case.m_In).c_str(), Case.m_Ops.c_str(), Results.c_str());
	}
}

// "c <outsize> <hexin> <hexout|ERR>" (Compress) and
// "d <outsize> <hexin> <hexout|ERR>" (Decompress).
static void GenHuffman(FILE *f)
{
	CHuffman Huffman;
	Huffman.Init();
	unsigned char aOut[4096];

	auto Compress = [&](const Bytes &In, int OutSize) {
		int n = Huffman.Compress(In.data(), (int)In.size(), aOut, OutSize);
		fprintf(f, "c %d %s %s\n", OutSize, Hex(In).c_str(), n < 0 ? "ERR" : Hex(aOut, n).c_str());
		return n < 0 ? Bytes() : Bytes(aOut, aOut + n);
	};
	auto Decompress = [&](const Bytes &In, int OutSize) {
		int n = Huffman.Decompress(In.data(), (int)In.size(), aOut, OutSize);
		fprintf(f, "d %d %s %s\n", OutSize, Hex(In).c_str(), n < 0 ? "ERR" : Hex(aOut, n).c_str());
	};

	std::vector<Bytes> vInputs = {{}};
	for(int b = 0; b < 256; b++)
		vInputs.push_back({(unsigned char)b});
	Bytes All(256);
	for(int b = 0; b < 256; b++)
		All[b] = b;
	vInputs.push_back(All);
	for(int i = 0; i < 300; i++)
		vInputs.push_back(RandSkewed(1 + Rand() % 1400));
	for(int i = 0; i < 100; i++)
		vInputs.push_back(RandBytes(1 + Rand() % 1400));

	std::vector<Bytes> vCompressed;
	for(const Bytes &In : vInputs)
	{
		Bytes c = Compress(In, 2048);
		vCompressed.push_back(c);
		// output buffer limits around the exact size
		if(!c.empty() && Rand() % 4 == 0)
		{
			Compress(In, (int)c.size());
			Compress(In, (int)c.size() - 1 > 0 ? (int)c.size() - 1 : 1);
			Compress(In, 1 + Rand() % 3);
		}
	}

	for(size_t i = 0; i < vCompressed.size(); i++)
	{
		const Bytes &c = vCompressed[i];
		Decompress(c, 2048);
		if(Rand() % 4 == 0)
		{
			Decompress(c, vInputs[i].empty() ? 1 : (int)vInputs[i].size());
			if(vInputs[i].size() > 1)
				Decompress(c, (int)vInputs[i].size() - 1);
		}
		if(c.size() > 1)
		{
			Bytes Cut(c.begin(), c.begin() + Rand() % c.size());
			Decompress(Cut, 2048);
			Bytes Flip = c;
			Flip[Rand() % Flip.size()] ^= 1 << (Rand() % 8);
			Decompress(Flip, 2048);
		}
	}

	// garbage
	for(int i = 0; i < 2000; i++)
		Decompress(RandBytes(Rand() % 40), 1 + (Rand() % 2 ? 2047 : Rand() % 8));
	for(int i = 0; i < 200; i++)
	{
		Bytes z(1 + Rand() % 8, 0);
		Decompress(z, 1 + Rand() % 64);
	}
}

// Chunk headers (split 4, the 0.6 layout):
//   "chpack <flags> <size> <seq> <hex>"
//   "chunpack <hex3> <flags> <size> <seq> <consumed>"
static void GenChunkHeader(FILE *f)
{
	int aSizes[] = {0, 1, 15, 16, 17, 63, 64, 255, 256, 1000, 1023};
	int aSeqs[] = {0, 1, 3, 4, 255, 256, 511, 512, 1023};
	for(int Flags = 0; Flags < 4; Flags++)
		for(int Size : aSizes)
			for(int Seq : aSeqs)
			{
				CNetChunkHeader h;
				h.m_Flags = Flags;
				h.m_Size = Size;
				h.m_Sequence = Seq;
				unsigned char aBuf[3];
				unsigned char *pEnd = h.Pack(aBuf);
				fprintf(f, "chpack %d %d %d %s\n", Flags, Size, Seq, Hex(aBuf, (int)(pEnd - aBuf)).c_str());
			}
	for(int i = 0; i < 5000; i++)
	{
		Bytes In = RandBytes(3);
		CNetChunkHeader h;
		unsigned char *pEnd = h.Unpack(In.data());
		fprintf(f, "chunpack %s %d %d %d %d\n", Hex(In).c_str(), h.m_Flags, h.m_Size, h.m_Sequence, (int)(pEnd - In.data()));
	}
}

// Packets, sent with the real CNetBase::SendPacket* over loopback and captured:
//   "penc <flags> <ack> <numchunks> <token> <datahex> <outhex>"
//   "cenc <extended> <extrahex> <datahex> <outhex>"
// Decoded with CNetBase::UnpackPacket the way CNetClient::Recv calls it:
//   "pdec <hexin> <ERR|OK> <sixup> <flags> <ack> <numchunks> <datahex> <extrahex>"
//   "backroom <seq> <ack> <0|1>"
static void GenPacket(FILE *f)
{
	CNetBase::Init();
	net_init();

	NETSOCKET RecvSocket = nullptr;
	NETADDR RecvAddr;
	for(int Port = 47800; Port < 47900 && !RecvSocket; Port++)
	{
		char aAddr[64];
		str_format(aAddr, sizeof(aAddr), "127.0.0.1:%d", Port);
		net_addr_from_str(&RecvAddr, aAddr);
		RecvSocket = net_udp_create(RecvAddr);
	}
	NETADDR BindAddr = NETADDR_ZEROED;
	BindAddr.type = NETTYPE_IPV4;
	NETSOCKET SendSocket = net_udp_create(BindAddr);
	dbg_assert(RecvSocket && SendSocket, "could not open sockets");

	auto Receive = [&]() {
		dbg_assert(net_socket_read_wait(RecvSocket, std::chrono::seconds(2)) > 0, "no packet received");
		NETADDR From;
		unsigned char *pData;
		int Size = net_udp_recv(RecvSocket, &From, &pData);
		dbg_assert(Size >= 0, "recv failed");
		return Bytes(pData, pData + Size);
	};

	std::vector<Bytes> vSent;
	int aTokens[] = {NET_SECURITY_TOKEN_UNSUPPORTED, NET_SECURITY_TOKEN_UNKNOWN, 0x12345678, (int)0xdeadbeef};
	for(int i = 0; i < 600; i++)
	{
		CNetPacketConstruct p = {};
		int Kind = Rand() % 3;
		p.m_Flags = Kind == 0 ? 0 : Kind == 1 ? NET_PACKETFLAG_RESEND : NET_PACKETFLAG_CONTROL;
		p.m_Ack = Rand() % NET_MAX_SEQUENCE;
		Bytes Data;
		if(Kind == 2)
		{
			Data = RandBytes(1 + Rand() % 8);
			Data[0] = Rand() % 5; // control message id
			p.m_NumChunks = 0;
		}
		else
		{
			int Size = Rand() % 4 == 0 ? Rand() % 8 : Rand() % (NET_MAX_PACKETSIZE - NET_PACKETHEADERSIZE - 4);
			Data = Rand() % 2 ? RandSkewed(Size) : RandBytes(Size);
			p.m_NumChunks = (Kind == 1 ? 0 : 1) + Rand() % 10;
		}
		p.m_DataSize = (int)Data.size();
		if(!Data.empty())
			mem_copy(p.m_aChunkData, Data.data(), Data.size());
		int Token = aTokens[Rand() % 4];
		int Flags = p.m_Flags, Ack = p.m_Ack, NumChunks = p.m_NumChunks;
		CNetBase::SendPacket(SendSocket, &RecvAddr, &p, Token);
		Bytes Out = Receive();
		fprintf(f, "penc %d %d %d %d %s %s\n", Flags, Ack, NumChunks, Token, Hex(Data).c_str(), Hex(Out).c_str());
		vSent.push_back(Out);
	}
	for(int i = 0; i < 100; i++)
	{
		bool Extended = Rand() % 2;
		Bytes Extra = RandBytes(NET_CONNLESS_EXTRA_SIZE);
		Bytes Data = RandBytes(Rand() % 64);
		CNetBase::SendPacketConnless(SendSocket, &RecvAddr, Data.data(), (int)Data.size(), Extended, Extra.data());
		Bytes Out = Receive();
		fprintf(f, "cenc %d %s %s %s\n", Extended, Hex(Extra).c_str(), Hex(Data).c_str(), Hex(Out).c_str());
		vSent.push_back(Out);
	}

	// decode: everything sent above, plus garbage
	std::vector<Bytes> vDecode = vSent;
	for(int i = 0; i < 3000; i++)
	{
		Bytes v = RandBytes(Rand() % 4 == 0 ? Rand() % 1500 : Rand() % 40);
		if(!v.empty() && Rand() % 2)
			v[0] &= 0xfc & ~(NET_PACKETFLAG_UNUSED << 2); // mostly 0.6, ack bits clear
		vDecode.push_back(v);
	}
	for(const Bytes &Sent : vSent)
	{
		if(Sent.size() > 4 && Rand() % 3 == 0)
		{
			Bytes Cut(Sent.begin(), Sent.begin() + Rand() % Sent.size());
			vDecode.push_back(Cut);
			Bytes Flip = Sent;
			Flip[3 + Rand() % (Flip.size() - 3)] ^= 1 << (Rand() % 8);
			vDecode.push_back(Flip);
		}
	}
	for(Bytes &In : vDecode)
	{
		Bytes Buf = In;
		Buf.reserve(1);
		CNetPacketConstruct p = {};
		bool Sixup = false;
		SECURITY_TOKEN Token, ResponseToken;
		int Ret = CNetBase::UnpackPacket(Buf.data(), (int)Buf.size(), &p, Sixup, true, &Token, &ResponseToken);
		if(Ret != 0)
			fprintf(f, "pdec %s ERR %d\n", Hex(In).c_str(), Sixup);
		else
			fprintf(f, "pdec %s OK %d %d %d %d %s %s\n", Hex(In).c_str(), Sixup, p.m_Flags, p.m_Ack, p.m_NumChunks,
				Hex(p.m_aChunkData, p.m_DataSize).c_str(),
				(p.m_Flags & NET_PACKETFLAG_EXTENDED) ? Hex(p.m_aExtraData, NET_CONNLESS_EXTRA_SIZE).c_str() : "-");
	}

	int aEdge[] = {0, 1, 511, 512, 513, 1022, 1023};
	for(int Seq : aEdge)
		for(int Ack : aEdge)
			fprintf(f, "backroom %d %d %d\n", Seq, Ack, CNetBase::IsSeqInBackroom(Seq, Ack));
	for(int i = 0; i < 3000; i++)
	{
		int Seq = Rand() % NET_MAX_SEQUENCE, Ack = Rand() % NET_MAX_SEQUENCE;
		fprintf(f, "backroom %d %d %d\n", Seq, Ack, CNetBase::IsSeqInBackroom(Seq, Ack));
	}

	net_udp_close(SendSocket);
	net_udp_close(RecvSocket);
}

// Protocol tables from datasrc/network.py as compiled into C++:
//   "uuid <name> <hex16>"                      every UUID in g_UuidManager
//   "objsize <name> <GetObjSize> <GetUnpackedObjSize>"
//   "obj <name> <hexin> <hexout|FAIL>"         CNetObjHandler::SecureUnpackObj
//   "msg <name> <hexin> <OK|failedon>"         CNetObjHandler::SecureUnpackMsg
static void GenProtocol(FILE *f)
{
	for(int i = 0; i < g_UuidManager.NumUuids(); i++)
	{
		const int Id = OFFSET_UUID + i;
		CUuid Uuid = g_UuidManager.GetUuid(Id);
		fprintf(f, "uuid %s %s\n", g_UuidManager.GetName(Id), Hex(Uuid.m_aData, sizeof(Uuid.m_aData)).c_str());
	}

	CNetObjHandler Handler;
	std::vector<int> vObjTypes, vMsgTypes;
	for(int t = 1; t < NUM_NETOBJTYPES; t++)
		vObjTypes.push_back(t);
	for(int t = __NETOBJTYPE_UUID_HELPER + 1; t < OFFSET_NETMSGTYPE_UUID; t++)
		vObjTypes.push_back(t);
	for(int t = 1; t < NUM_NETMSGTYPES; t++)
		vMsgTypes.push_back(t);
	for(int t = __NETMSGTYPE_UUID_HELPER + 1; t < OFFSET_MAPITEMTYPE_UUID; t++)
		vMsgTypes.push_back(t);

	for(int t = 0; t < NUM_NETOBJTYPES; t++)
		fprintf(f, "objsize %s %d %d\n", Handler.GetObjName(t), Handler.GetObjSize(t), Handler.GetUnpackedObjSize(t));

	auto RandInt = []() -> int {
		switch(Rand() % 6)
		{
		case 0: return Rand() % 4;
		case 1: return -(int)(Rand() % 4);
		case 2: return (int)(Rand() % 300) - 150;
		case 3: return (int)(Rand() ^ (Rand() << 16));
		case 4: return Rand() % 2 ? 0x7fffffff : (int)0x80000000;
		default: return Rand() % 1000000;
		}
	};

	for(int Type : vObjTypes)
	{
		const int NumInts = Handler.GetUnpackedObjSize(Type) / (int)sizeof(int);
		for(int Case = 0; Case < 40; Case++)
		{
			int n = NumInts + (int)(Rand() % 5) - 3;
			if(n < 0)
				n = 0;
			std::vector<int> vIn(n);
			for(int &v : vIn)
				v = RandInt();
			Bytes In(vIn.size() * sizeof(int));
			if(!In.empty())
				mem_copy(In.data(), vIn.data(), In.size());
			In.reserve(1);
			CUnpacker u;
			u.Reset(In.data(), (int)In.size());
			const void *pOut = Handler.SecureUnpackObj(Type, &u);
			fprintf(f, "obj %s %s %s\n", Handler.GetObjName(Type), Hex(In).c_str(),
				pOut ? Hex((const unsigned char *)pOut, Handler.GetUnpackedObjSize(Type)).c_str() : "FAIL");
		}
	}

	for(int Type : vMsgTypes)
	{
		for(int Case = 0; Case < 40; Case++)
		{
			CPacker p;
			p.Reset();
			int NumFields = Rand() % 9;
			for(int k = 0; k < NumFields; k++)
			{
				if(Rand() % 3)
					p.AddInt(Rand() % 2 ? RandInt() : (int)(Rand() % 3));
				else
				{
					Bytes s = RandSkewed(Rand() % 6);
					for(auto &b : s)
						if(b == 0)
							b = ' ';
					std::string Str(s.begin(), s.end());
					p.AddString(Str.c_str());
				}
			}
			Bytes In(p.Data(), p.Data() + p.Size());
			Bytes Buf = In;
			Buf.reserve(1);
			CUnpacker u;
			u.Reset(Buf.data(), (int)Buf.size());
			const void *pOut = Handler.SecureUnpackMsg(Type, &u);
			fprintf(f, "msg %s %s %s\n", Handler.GetMsgName(Type), Hex(In).c_str(), pOut ? "OK" : Handler.FailedMsgOn());
		}
	}
}

// Snapshots, built with CSnapshotBuilder; deltas made with CSnapshotDelta::CreateDelta and
// CVariableInt::Compress like the server, then decoded like CClient::ProcessServerPacket:
//   "snap <n> <hex>"                          snapshot n (full CSnapshot bytes)
//   "delta <from|-1> <hexin> <ERR code|hex> <crc> <valid>"   UnpackDelta result
//   "alt <n> <items>"                         CClient::UnpackAndValidateSnapshot of snapshot n:
//                                             ';'-separated "<type name>:<id>:<hex>" (EX items omitted)
static void GenSnapshot(FILE *f)
{
	CNetObjHandler Handler;
	CSnapshotDelta Delta;
	for(int i = 0; i < NUM_NETOBJTYPES; i++)
		Delta.SetStaticsize(i, Handler.GetObjSize(i));

	const int aExTypes[] = {NETOBJTYPE_DDNETCHARACTER, NETOBJTYPE_DDNETPLAYER, NETOBJTYPE_GAMEINFOEX, NETOBJTYPE_SWITCHSTATE};
	auto RandomValue = []() -> int {
		switch(Rand() % 4)
		{
		case 0: return Rand() % 10;
		case 1: return (int)(Rand() % 2000) - 1000;
		case 2: return (int)(Rand() ^ (Rand() << 16));
		default: return 0;
		}
	};
	struct SItem
	{
		int m_Type, m_Id;
		std::vector<int> m_vData;
	};
	auto Build = [&](const std::vector<SItem> &vItems, CSnapshotBuffer *pBuf) {
		CSnapshotBuilder b;
		b.Init();
		for(const SItem &It : vItems)
			b.NewItem(It.m_Type, It.m_Id, It.m_vData.data(), (int)It.m_vData.size() * sizeof(int));
		return b.Finish(pBuf);
	};
	auto RandomItem = [&]() {
		SItem It;
		int k = Rand() % 10;
		if(k < 6)
		{
			It.m_Type = 1 + Rand() % (NUM_NETOBJTYPES - 1);
			It.m_vData.resize(Handler.GetObjSize(It.m_Type) / sizeof(int));
		}
		else if(k < 8)
		{
			It.m_Type = aExTypes[Rand() % 4];
			It.m_vData.resize(Handler.GetUnpackedObjSize(It.m_Type) / sizeof(int) + Rand() % 3 - 1);
		}
		else
		{
			It.m_Type = NUM_NETOBJTYPES + Rand() % 60; // unknown or no static size
			It.m_vData.resize(Rand() % 6);
		}
		It.m_Id = Rand() % 24;
		for(int &v : It.m_vData)
			v = RandomValue();
		return It;
	};

	std::vector<std::vector<SItem>> vSnaps;
	std::vector<SItem> vCur;
	for(int n = 0; n < 60; n++)
	{
		// mutate: delete some, change some, add some (keeping keys unique)
		std::vector<SItem> vNext;
		for(const SItem &It : vCur)
		{
			if(Rand() % 8 == 0)
				continue;
			SItem Changed = It;
			if(Rand() % 3 == 0)
				for(int &v : Changed.m_vData)
					if(Rand() % 3 == 0)
						v = RandomValue();
			vNext.push_back(Changed);
		}
		int NumAdd = n == 0 ? 20 : Rand() % 6;
		for(int i = 0; i < NumAdd; i++)
		{
			SItem It = RandomItem();
			bool Dup = false;
			for(const SItem &o : vNext)
				Dup |= o.m_Type == It.m_Type && o.m_Id == It.m_Id;
			if(!Dup)
				vNext.push_back(It);
		}
		if(n % 7 == 6 && vNext.size() > 1)
			std::swap(vNext[0], vNext[vNext.size() - 1]); // reorder
		vSnaps.push_back(vNext);
		vCur = vNext;
	}

	std::vector<CSnapshotBuffer> vBufs(vSnaps.size());
	std::vector<int> vSizes;
	for(size_t n = 0; n < vSnaps.size(); n++)
	{
		vSizes.push_back(Build(vSnaps[n], &vBufs[n]));
		fprintf(f, "snap %d %s\n", (int)n, Hex(vBufs[n].m_aData, vSizes[n]).c_str());
	}

	auto Decode = [&](int From, const Bytes &Compressed) {
		const CSnapshot *pFrom = From < 0 ? CSnapshot::EmptySnapshot() : vBufs[From].AsSnapshot();
		static CSnapshotDeltaBuffer s_DeltaBuf;
		const void *pDeltaData = Delta.EmptyDelta();
		int DeltaSize = sizeof(int) * 3;
		if(!Compressed.empty())
		{
			DeltaSize = CVariableInt::Decompress(Compressed.data(), (int)Compressed.size(), s_DeltaBuf.m_aData, sizeof(s_DeltaBuf.m_aData));
			if(DeltaSize < 0)
			{
				fprintf(f, "delta %d %s ERR decompress\n", From, Hex(Compressed).c_str());
				return;
			}
			pDeltaData = s_DeltaBuf.m_aData;
		}
		static CSnapshotBuffer s_Out;
		int Size = Delta.UnpackDelta(pFrom, &s_Out, pDeltaData, DeltaSize);
		if(Size < 0)
			fprintf(f, "delta %d %s ERR %d\n", From, Hex(Compressed).c_str(), Size);
		else
			fprintf(f, "delta %d %s %s %u %d\n", From, Hex(Compressed).c_str(), Hex(s_Out.m_aData, Size).c_str(),
				s_Out.AsSnapshot()->Crc(), s_Out.AsSnapshot()->IsValid(Size));
	};

	std::vector<Bytes> vValid;
	for(int n = 0; n < (int)vSnaps.size(); n++)
	{
		for(int From : {n - 1, -1, n - 3})
		{
			if(From < -1)
				continue;
			const CSnapshot *pFrom = From < 0 ? CSnapshot::EmptySnapshot() : vBufs[From].AsSnapshot();
			static CSnapshotDeltaBuffer s_Delta;
			int DeltaSize = Delta.CreateDelta(pFrom, vBufs[n].AsSnapshot(), &s_Delta);
			Bytes Compressed;
			if(DeltaSize > 0)
			{
				Compressed.resize(2 * CSnapshot::MAX_SIZE);
				int n2 = CVariableInt::Compress(s_Delta.m_aData, DeltaSize, Compressed.data(), (int)Compressed.size());
				Compressed.resize(n2);
			}
			Decode(From, Compressed);
			if(!Compressed.empty())
				vValid.push_back(Compressed);
		}
	}

	// malformed deltas: mutated valid ones and random varint streams
	for(int i = 0; i < 800; i++)
	{
		Bytes In;
		if(i % 2 == 0 && !vValid.empty())
		{
			In = vValid[Rand() % vValid.size()];
			int NumFlips = 1 + Rand() % 3;
			for(int k = 0; k < NumFlips; k++)
				In[Rand() % In.size()] ^= 1 << (Rand() % 8);
			if(Rand() % 4 == 0)
				In.resize(Rand() % In.size() + 1);
		}
		else
		{
			CPacker p;
			p.Reset();
			int NumInts = 1 + Rand() % 30;
			for(int k = 0; k < NumInts; k++)
				p.AddInt(k < 3 ? (int)(Rand() % 4) - 1 : Rand() % 3 == 0 ? RandomValue() : (int)(Rand() % 40));
			In.assign(p.Data(), p.Data() + p.Size());
		}
		Decode(Rand() % 2 ? -1 : (int)(Rand() % vSnaps.size()), In);
	}

	// CClient::UnpackAndValidateSnapshot
	for(size_t n = 0; n < vSnaps.size(); n++)
	{
		const CSnapshot *pFrom = vBufs[n].AsSnapshot();
		CSnapshotBuilder Builder;
		Builder.Init();
		for(int Index = 0; Index < pFrom->NumItems(); Index++)
		{
			const CSnapshotItem *pFromItem = pFrom->GetItem(Index);
			const int ItemType = pFrom->GetItemType(Index);
			if(ItemType <= 0)
				continue;
			CUnpacker Unpacker;
			Unpacker.Reset(pFromItem->Data(), pFrom->GetItemSize(Index));
			void *pSecuredData = Handler.SecureUnpackObj(ItemType, &Unpacker);
			if(!pSecuredData)
				continue;
			Builder.NewItem(ItemType, pFromItem->Id(), pSecuredData, Handler.GetUnpackedObjSize(ItemType));
		}
		static CSnapshotBuffer s_Alt;
		int AltSize = Builder.Finish(&s_Alt);
		const CSnapshot *pAlt = s_Alt.AsSnapshot();
		dbg_assert(AltSize > 0, "alt snapshot failed");
		std::string Items;
		for(int Index = 0; Index < pAlt->NumItems(); Index++)
		{
			const int Type = pAlt->GetItemType(Index);
			if(Type == 0)
				continue;
			char aBuf[64];
			str_format(aBuf, sizeof(aBuf), "%s%s:%d:", Items.empty() ? "" : ";", Handler.GetObjName(Type), pAlt->GetItem(Index)->Id());
			Items += aBuf;
			Items += Hex((const unsigned char *)pAlt->GetItem(Index)->Data(), pAlt->GetItemSize(Index));
		}
		fprintf(f, "alt %d %s\n", (int)n, Items.empty() ? "-" : Items.c_str());
	}
}

// CNetConnection driven through random scenarios (client side, no timers):
//   "scenario <n>" then op lines, each followed by every datagram the connection sent:
//   "connect" / "feed <hexin> <ret>" / "queue <flags> <hex> <ret>" / "flush <ret>" /
//   "disconnect <hexreason>", "sent <hex>", "chunk <flags> <seq> <hex>" (from
//   CPacketChunkUnpacker), and after each op "state <state> <ack> <hexerror>".
// At most one resend request per scenario keeps C++'s per-second resend limit
// independent of wall-clock time.
static void GenConn(FILE *f)
{
	CNetBase::Init();
	net_init();
	NETSOCKET ClientSock = nullptr, ServerSock = nullptr;
	NETADDR ClientAddr, ServerAddr;
	for(int Port = 47900; Port < 48000 && !ServerSock; Port += 2)
	{
		char aAddr[64];
		str_format(aAddr, sizeof(aAddr), "127.0.0.1:%d", Port);
		net_addr_from_str(&ServerAddr, aAddr);
		str_format(aAddr, sizeof(aAddr), "127.0.0.1:%d", Port + 1);
		net_addr_from_str(&ClientAddr, aAddr);
		ClientSock = net_udp_create(ClientAddr);
		ServerSock = ClientSock ? net_udp_create(ServerAddr) : nullptr;
	}
	dbg_assert(ClientSock && ServerSock, "could not open sockets");

	auto DrainSent = [&]() {
		while(net_socket_read_wait(ServerSock, std::chrono::milliseconds(20)) > 0)
		{
			NETADDR From;
			unsigned char *pData;
			int Size = net_udp_recv(ServerSock, &From, &pData);
			if(Size < 0)
				break;
			fprintf(f, "sent %s\n", Hex(pData, Size).c_str());
		}
	};

	for(int Scenario = 0; Scenario < 150; Scenario++)
	{
		fprintf(f, "scenario %d\n", Scenario);
		CNetConnection Conn;
		Conn.Init(ClientSock, false);
		CPacketChunkUnpacker Unpacker;
		auto State = [&]() {
			const char *pErr = Conn.ErrorString();
			fprintf(f, "state %d %d %s\n", (int)Conn.State(), Conn.AckSequence(), Hex((const unsigned char *)pErr, str_length(pErr)).c_str());
		};

		fprintf(f, "connect\n");
		Conn.Connect(&ServerAddr, 1);
		DrainSent();
		State();

		// server -> client packets, encoded like CNetBase::SendPacket without compression
		const int ServerToken = Rand() % 4 == 0 ? NET_SECURITY_TOKEN_UNSUPPORTED : (int)(Rand() ^ (Rand() << 16));
		int ServerSeq = 0;
		auto Feed = [&](Bytes Packet) {
			CNetPacketConstruct p;
			bool Sixup = false;
			SECURITY_TOKEN Token, ResponseToken;
			Bytes Buf = Packet;
			int Ret = CNetBase::UnpackPacket(Buf.data(), (int)Buf.size(), &p, Sixup, true, &Token, &ResponseToken);
			int FeedRet = Ret == 0 && Conn.State() != CNetConnection::EState::OFFLINE && Conn.State() != CNetConnection::EState::ERROR ?
					      Conn.Feed(&p, &ServerAddr, Token, ResponseToken) :
					      -1;
			fprintf(f, "feed %s %d\n", Hex(Packet).c_str(), FeedRet);
			if(FeedRet == 1 && !(p.m_Flags & NET_PACKETFLAG_CONTROL) && p.m_DataSize > 0 && p.m_NumChunks > 0)
			{
				Unpacker.FeedPacket(ServerAddr, p, &Conn, 0);
				CNetChunk Chunk;
				while(Unpacker.UnpackNextChunk(&Chunk))
					fprintf(f, "chunk %d %s\n", Chunk.m_Flags, Hex((const unsigned char *)Chunk.m_pData, Chunk.m_DataSize).c_str());
			}
			DrainSent();
			State();
		};
		auto Encode = [&](int Flags, int Ack, int NumChunks, const Bytes &Data, bool BadToken) {
			Bytes Out = {(unsigned char)(((Flags << 2) & 0xfc) | ((Ack >> 8) & 0x3)), (unsigned char)(Ack & 0xff), (unsigned char)NumChunks};
			Out.insert(Out.end(), Data.begin(), Data.end());
			if(ServerToken != NET_SECURITY_TOKEN_UNSUPPORTED)
			{
				unsigned char aTok[4];
				WriteSecurityToken(aTok, BadToken ? ServerToken ^ 1 : ServerToken);
				Out.insert(Out.end(), aTok, aTok + 4);
			}
			return Out;
		};

		// CONNECTACCEPT, with or without the TKEN magic
		{
			Bytes Data = {NET_CTRLMSG_CONNECTACCEPT};
			if(ServerToken != NET_SECURITY_TOKEN_UNSUPPORTED)
			{
				Data.insert(Data.end(), SECURITY_TOKEN_MAGIC, SECURITY_TOKEN_MAGIC + 4);
				unsigned char aTok[4];
				WriteSecurityToken(aTok, ServerToken);
				Data.insert(Data.end(), aTok, aTok + 4);
			}
			Bytes Out = {(unsigned char)(NET_PACKETFLAG_CONTROL << 2), 0, 0};
			Out.insert(Out.end(), Data.begin(), Data.end());
			Feed(Out);
		}

		bool ResendUsed = false;
		int NumOps = 5 + Rand() % 30;
		for(int Op = 0; Op < NumOps && Conn.State() == CNetConnection::EState::ONLINE; Op++)
		{
			int Kind = Rand() % 10;
			if(Kind < 3)
			{
				int Flags = Rand() % 3 ? NET_CHUNKFLAG_VITAL : 0;
				Bytes Data = RandSkewed(Rand() % 3 == 0 ? Rand() % 400 : Rand() % 30);
				int Ret = Conn.QueueChunk(Flags, (int)Data.size(), Data.data());
				fprintf(f, "queue %d %s %d\n", Flags, Hex(Data).c_str(), Ret);
				DrainSent();
				State();
			}
			else if(Kind < 5)
			{
				int Ret = Conn.Flush();
				fprintf(f, "flush %d\n", Ret);
				DrainSent();
				State();
			}
			else if(Kind < 9)
			{
				// data packet with 1..4 chunks: in order, duplicate, future or non-vital
				Bytes Data;
				int NumChunks = 1 + Rand() % 4;
				for(int c = 0; c < NumChunks; c++)
				{
					CNetChunkHeader h;
					int r = Rand() % 10;
					h.m_Flags = r < 2 ? 0 : NET_CHUNKFLAG_VITAL;
					if(h.m_Flags)
					{
						if(r < 7)
							h.m_Sequence = ServerSeq = (ServerSeq + 1) % NET_MAX_SEQUENCE;
						else if(r < 9)
							h.m_Sequence = ServerSeq; // duplicate
						else
							h.m_Sequence = (ServerSeq + 2 + Rand() % 5) % NET_MAX_SEQUENCE; // future
					}
					Bytes Payload = RandSkewed(Rand() % 20);
					h.m_Size = (int)Payload.size();
					unsigned char aHdr[3];
					unsigned char *pEnd = h.Pack(aHdr);
					Data.insert(Data.end(), aHdr, pEnd);
					Data.insert(Data.end(), Payload.begin(), Payload.end());
				}
				int Flags = 0;
				if(!ResendUsed && Rand() % 8 == 0)
				{
					Flags |= NET_PACKETFLAG_RESEND;
					ResendUsed = true;
				}
				int Ack = Rand() % 6 == 0 ? (int)(Rand() % NET_MAX_SEQUENCE) : Conn.SeqSequence() - (int)(Rand() % 2);
				if(Ack < 0)
					Ack += NET_MAX_SEQUENCE;
				Feed(Encode(Flags, Ack, NumChunks, Data, Rand() % 12 == 0));
			}
			else
			{
				Bytes Ctrl = {NET_CTRLMSG_KEEPALIVE};
				if(Rand() % 4 == 0)
				{
					Ctrl = {NET_CTRLMSG_CLOSE};
					Bytes Reason = RandSkewed(Rand() % 12);
					Ctrl.insert(Ctrl.end(), Reason.begin(), Reason.end());
					if(Rand() % 2)
						Ctrl.push_back(0);
				}
				Feed(Encode(NET_PACKETFLAG_CONTROL, Conn.SeqSequence(), 0, Ctrl, false));
			}
		}

		const char *pReason = Rand() % 2 ? "bye" : nullptr;
		fprintf(f, "disconnect %s\n", pReason ? Hex((const unsigned char *)pReason, 3).c_str() : "N");
		Conn.Disconnect(pReason);
		DrainSent();
		State();
	}
	net_udp_close(ClientSock);
	net_udp_close(ServerSock);
}

// str_time for every format at the unit boundaries and random values, and str_time_float
// for times in milliseconds / 1000 as the finish and record messages use it. Lines:
// "t <format> <centisecs> <text>" and "f <format> <float bits> <text>".
static void GenStrTime(FILE *f)
{
	const int64_t aEdges[] = {-1, 0, 1, 99, 100, 101, 5999, 6000, 6001, 359999, 360000, 360001,
		8639999, 8640000, 8640001, 999999999, 2147483647, 214748364700, 9223372036854775807};
	char aBuf[64];
	for(int Format = 0; Format <= (int)ETimeFormat::SECS_CENTISECS; Format++)
	{
		for(int64_t Centisecs : aEdges)
		{
			str_time(Centisecs, (ETimeFormat)Format, aBuf, sizeof(aBuf));
			fprintf(f, "t %d %lld %s\n", Format, (long long)Centisecs, aBuf);
		}
		for(int i = 0; i < 300; i++)
		{
			const int64_t Centisecs = (int64_t)(Rand() % 4 == 0 ? Rand() % 1000000000u : Rand() % 1000000u);
			str_time(Centisecs, (ETimeFormat)Format, aBuf, sizeof(aBuf));
			fprintf(f, "t %d %lld %s\n", Format, (long long)Centisecs, aBuf);
		}
		for(int i = 0; i < 300; i++)
		{
			// a network int in milliseconds, as Sv_RaceFinish m_Time / 1000.0f (incl.
			// negative diffs and the extremes)
			int Ms = (int)(Rand() % 10 == 0 ? (Rand() << 8) ^ Rand() : Rand() % 4000000u);
			if(i < 4)
				Ms = i == 0 ? INT32_MIN : i == 1 ? INT32_MAX : i == 2 ? -1 : 5;
			const float Secs = (i % 2 ? absolute(Ms) : Ms) / 1000.0f;
			unsigned Bits;
			mem_copy(&Bits, &Secs, sizeof(Bits));
			str_time_float(Secs, (ETimeFormat)Format, aBuf, sizeof(aBuf));
			fprintf(f, "f %d %08x %s\n", Format, Bits, aBuf);
		}
	}
}

int main(int argc, const char **argv)
{
	CCmdlineFix CmdlineFix(&argc, &argv);
	log_set_global_logger_default();

	if(argc != 2)
	{
		log_error(TOOL_NAME, "Usage: %s <output_dir>", argc > 0 ? argv[0] : TOOL_NAME);
		return -1;
	}
	const char *pDir = argv[1];

	struct SGen
	{
		const char *m_pName;
		void (*m_pfnGen)(FILE *);
	} aGens[] = {
		{"varint.txt", GenVarint},
		{"packer.txt", GenPacker},
		{"huffman.txt", GenHuffman},
		{"chunk_header.txt", GenChunkHeader},
		{"packet.txt", GenPacket},
		{"protocol.txt", GenProtocol},
		{"snapshot.txt", GenSnapshot},
		{"conn.txt", GenConn},
		{"str_time.txt", GenStrTime},
	};
	for(const SGen &Gen : aGens)
	{
		FILE *f = Open(pDir, Gen.m_pName);
		if(!f)
			return -1;
		Gen.m_pfnGen(f);
		fclose(f);
		log_info(TOOL_NAME, "wrote %s", Gen.m_pName);
	}
	return 0;
}
