// Writes golden vectors produced by the C++ engine code for the Rust client
// (rust-client/). Every value here comes from calling the real C++ functions;
// nothing is reimplemented. Output is plain text, one case per line, hex for bytes
// ("-" for empty).
//
// Usage: rs_golden <output_dir>

#include <base/logger.h>
#include <base/net.h>
#include <base/os.h>
#include <base/str.h>

#include <engine/shared/compression.h>
#include <engine/shared/packer.h>

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
