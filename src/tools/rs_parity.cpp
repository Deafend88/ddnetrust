// Parity data for the Rust client (rust-client/): runs the real C++ map loading and
// physics code and writes what it computed, for rust-client/ddnet/tests/parity.rs.
//
// Usage: rs_parity map <map file> <output file> <scratch dir>
//        rs_parity collision <map file> <output file> [hooktele] [verbose]
//
// "map" loads the map with CMap + CLayers and writes every item, every data blob and
// the physics layers, then loads mutated copies of the file (truncated, bit flips,
// overwritten ints) and writes whether each one loads and what its physics layers are.
//
// "collision" drives boxes through the map with CCollision::MoveBox for many ticks and
// queries the collision functions the game core and prediction use at every tick.
// "hooktele" first turns all tele-in tiles (10, 26, 31, 63) into hook tele-ins (15),
// since none of the bundled maps has hook teleporters.

#include <base/dbg.h>
#include <base/fs.h>
#include <base/io.h>
#include <base/logger.h>
#include <base/math.h>
#include <base/mem.h>
#include <base/os.h>
#include <base/str.h>
#include <base/vmath.h>

#include <engine/map.h>
#include <engine/shared/uuid_manager.h>
#include <engine/storage.h>

#include <game/collision.h>
#include <game/layers.h>
#include <game/mapitems.h>

#include <zlib.h>

#include <cstdio>
#include <string>
#include <vector>

static constexpr const char *const TOOL_NAME = "rs_parity";

typedef std::vector<unsigned char> Bytes;

static unsigned s_Seed = 4242;
static unsigned Rand()
{
	s_Seed = s_Seed * 1103515245u + 12345u;
	return s_Seed >> 8;
}

static unsigned Crc(const void *pData, size_t Size)
{
	return crc32(0, (const Bytef *)pData, (uInt)Size);
}

// "layer <name> -" | "layer <name> <w> <h> ERR" | "layer <name> <w> <h> <size> <crc>"
static void DumpLayers(FILE *f, IMap *pMap)
{
	CLayers Layers;
	Layers.Init(pMap, false, false);
	struct SLayer
	{
		const char *m_pName;
		const CMapItemLayerTilemap *m_pTilemap;
		int m_DataIndex;
	} aLayers[] = {
		{"game", Layers.GameLayer(), Layers.GameLayer() ? Layers.GameLayer()->m_Data : -1},
		{"tele", Layers.TeleLayer(), Layers.TeleLayer() ? Layers.TeleLayer()->m_Tele : -1},
		{"speedup", Layers.SpeedupLayer(), Layers.SpeedupLayer() ? Layers.SpeedupLayer()->m_Speedup : -1},
		{"front", Layers.FrontLayer(), Layers.FrontLayer() ? Layers.FrontLayer()->m_Front : -1},
		{"switch", Layers.SwitchLayer(), Layers.SwitchLayer() ? Layers.SwitchLayer()->m_Switch : -1},
		{"tune", Layers.TuneLayer(), Layers.TuneLayer() ? Layers.TuneLayer()->m_Tune : -1},
	};
	for(const SLayer &Layer : aLayers)
	{
		if(!Layer.m_pTilemap)
		{
			fprintf(f, "layer %s -\n", Layer.m_pName);
			continue;
		}
		const void *pData = pMap->GetData(Layer.m_DataIndex);
		if(!pData)
		{
			fprintf(f, "layer %s %d %d ERR\n", Layer.m_pName, Layer.m_pTilemap->m_Width, Layer.m_pTilemap->m_Height);
			continue;
		}
		const int Size = pMap->GetDataSize(Layer.m_DataIndex);
		fprintf(f, "layer %s %d %d %d %08x\n", Layer.m_pName, Layer.m_pTilemap->m_Width, Layer.m_pTilemap->m_Height, Size, Crc(pData, Size));
	}
}

static int MapMode(const char *pMapFile, const char *pOutFile, const char *pScratchDir)
{
	std::unique_ptr<IStorage> pStorage = CreateLocalStorage();
	FILE *f = fopen(pOutFile, "wb");
	if(!pStorage || !f)
	{
		log_error(TOOL_NAME, "cannot create storage or open '%s'", pOutFile);
		return -1;
	}

	// full dump of the unmodified map
	std::unique_ptr<IMap> pMap = CreateMap();
	if(!pMap->Load(pStorage.get(), pMapFile, IStorage::TYPE_ABSOLUTE))
	{
		log_error(TOOL_NAME, "cannot load '%s'", pMapFile);
		return -1;
	}
	fprintf(f, "file %d %08x\n", pMap->Size(), pMap->Crc());
	for(int Index = 0; Index < pMap->NumItems(); Index++)
	{
		int Type, Id;
		CUuid Uuid;
		const void *pItem = pMap->GetItem(Index, &Type, &Id, &Uuid);
		const int Size = pMap->GetItemSize(Index);
		char aUuid[UUID_MAXSTRSIZE];
		FormatUuid(Uuid, aUuid, sizeof(aUuid));
		fprintf(f, "item %d %d %s %d %d %08x\n", Index, Uuid == UUID_ZEROED ? Type : -1, Uuid == UUID_ZEROED ? "-" : aUuid, Id, Size, Crc(pItem, Size));
	}
	for(int Index = 0; Index < pMap->NumData(); Index++)
	{
		const void *pData = pMap->GetData(Index); // size is final after loading
		if(!pData)
			fprintf(f, "data %d ERR\n", Index);
		else
			fprintf(f, "data %d %d %08x\n", Index, pMap->GetDataSize(Index), Crc(pData, pMap->GetDataSize(Index)));
	}
	DumpLayers(f, pMap.get());
	pMap->Unload();

	// mutated copies: "mut <kind> <offset> <value> fail" or followed by layer lines
	IOHANDLE File = io_open(pMapFile, IOFLAG_READ);
	Bytes Original;
	{
		unsigned char aBuf[4096];
		unsigned n;
		while((n = io_read(File, aBuf, sizeof(aBuf))) > 0)
			Original.insert(Original.end(), aBuf, aBuf + n);
		io_close(File);
	}
	char aScratch[IO_MAX_PATH_LENGTH];
	str_format(aScratch, sizeof(aScratch), "%s/rs_parity_mutated.map", pScratchDir);
	const int Size = (int)Original.size();
	for(int i = 0; i < 150; i++)
	{
		Bytes Mutated = Original;
		char Kind;
		int Offset, Value;
		const int Region = Rand() % 4;
		// header (36 bytes), then the item area is roughly the first quarter, then data
		const int Lo = Region == 0 ? 0 : Region == 1 ? 36 : Region == 2 ? Size / 8 : Size / 2;
		const int Hi = Region == 0 ? 36 : Region == 1 ? Size / 4 : Size;
		Offset = Lo + Rand() % (Hi - Lo);
		switch(Rand() % 3)
		{
		case 0:
			Kind = 't';
			Value = 0;
			Mutated.resize(Offset);
			break;
		case 1:
			Kind = 'x';
			Value = 1 + Rand() % 255;
			Mutated[Offset] ^= Value;
			break;
		default:
			Kind = 'w';
			Offset &= ~3;
			Value = Rand() % 3 == 0 ? -1 : Rand() % 2 ? (int)(Rand() % 200) - 50 : (int)(Rand() ^ (Rand() << 16));
			if(Offset + 4 > Size)
				Offset = Size - 4;
			for(int k = 0; k < 4; k++)
				Mutated[Offset + k] = (Value >> (8 * k)) & 0xff;
			break;
		}
		IOHANDLE Out = io_open(aScratch, IOFLAG_WRITE);
		io_write(Out, Mutated.data(), Mutated.size());
		io_close(Out);
		fprintf(f, "mut %c %d %d", Kind, Offset, Value);
		if(!pMap->Load(pStorage.get(), aScratch, IStorage::TYPE_ABSOLUTE))
		{
			fprintf(f, " fail\n");
			continue;
		}
		fprintf(f, " ok\n");
		DumpLayers(f, pMap.get());
		pMap->Unload();
	}
	fclose(f);
	(void)fs_remove(aScratch);
	return 0;
}

static bool LoadMapAndLayers(IStorage *pStorage, const char *pMapFile, IMap *pMap, CLayers *pLayers)
{
	if(!pMap->Load(pStorage, pMapFile, IStorage::TYPE_ABSOLUTE))
	{
		log_error(TOOL_NAME, "cannot load '%s'", pMapFile);
		return false;
	}
	pLayers->Init(pMap, false, true); // like CGameClient::OnConnected
	return true;
}

static unsigned FloatBits(float f)
{
	unsigned u;
	mem_copy(&u, &f, sizeof(u));
	return u;
}

// Query results of one tick, as ints (floats by their bits), CRC'd per tick.
struct SQueries
{
	std::vector<int> m_vValues;
	void Int(int v) { m_vValues.push_back(v); }
	void Float(float f) { m_vValues.push_back((int)FloatBits(f)); }
	void Vec(vec2 v)
	{
		Float(v.x);
		Float(v.y);
	}
	unsigned Crc() const { return ::Crc(m_vValues.data(), m_vValues.size() * sizeof(int)); }
};

static bool SwitchActiveCb(unsigned char Number, void *pUser)
{
	return Number % 2 == 0;
}

// "static <highest switch number> <crc of switch types>" then per body
// "body <start x bits> <start y bits> <elasticity x bits> <elasticity y bits>" and per tick
// "<ax> <jump> <boost> <bx> <by> <angle> <len> <pos x> <pos y> <vel x> <vel y> <crc>" (hex
// float bits), with the query values appended in verbose mode.
static void MakeHookTeleporters(IMap *pMap, CLayers *pLayers)
{
	if(!pLayers->TeleLayer())
		return;
	CTeleTile *pTele = static_cast<CTeleTile *>(pMap->GetData(pLayers->TeleLayer()->m_Tele));
	const int Count = pLayers->GameLayer()->m_Width * pLayers->GameLayer()->m_Height;
	for(int i = 0; pTele && i < Count; i++)
		if(pTele[i].m_Type == TILE_TELEINEVIL || pTele[i].m_Type == TILE_TELEIN || pTele[i].m_Type == TILE_TELECHECKIN || pTele[i].m_Type == TILE_TELECHECKINEVIL)
			pTele[i].m_Type = TILE_TELEINHOOK;
}

static int CollisionMode(const char *pMapFile, const char *pOutFile, bool HookTele, bool Verbose)
{
	std::unique_ptr<IStorage> pStorage = CreateLocalStorage();
	std::unique_ptr<IMap> pMap = CreateMap();
	CLayers Layers;
	FILE *f = fopen(pOutFile, "wb");
	if(!pStorage || !f || !LoadMapAndLayers(pStorage.get(), pMapFile, pMap.get(), &Layers))
		return -1;
	if(HookTele)
		MakeHookTeleporters(pMap.get(), &Layers);
	CCollision Collision;
	Collision.Init(&Layers);

	const int Width = Collision.GetWidth(), Height = Collision.GetHeight();
	std::vector<int> vSwitchTypes;
	if(Collision.SwitchLayer())
		for(int i = 0; i < Width * Height; i++)
			vSwitchTypes.push_back(Collision.GetSwitchType(i));
	fprintf(f, "static %d %08x\n", Collision.m_HighestSwitchNumber, Crc(vSwitchTypes.data(), vSwitchTypes.size() * sizeof(int)));

	const float aElasticities[] = {0.0f, 0.0f, 0.5f, 1.0f, -0.3f, 2.0f};
	for(int Body = 0; Body < 24; Body++)
	{
		int Index = 0;
		for(int Try = 0; Try < 100; Try++)
		{
			Index = Rand() % (Width * Height);
			const int Tile = Collision.GetTileIndex(Index);
			if(Tile != TILE_SOLID && Tile != TILE_NOHOOK)
				break;
		}
		vec2 Pos = vec2((Index % Width) * 32.0f + 16.0f + (int)(Rand() % 9) - 4, (Index / Width) * 32.0f + 16.0f + (int)(Rand() % 9) - 4);
		vec2 Vel = vec2(0.0f, 0.0f);
		const vec2 Elasticity = vec2(aElasticities[Rand() % 6], aElasticities[Rand() % 6]);
		fprintf(f, "body %08x %08x %08x %08x\n", FloatBits(Pos.x), FloatBits(Pos.y), FloatBits(Elasticity.x), FloatBits(Elasticity.y));

		for(int Tick = 0; Tick < 300; Tick++)
		{
			const int Ax = (int)(Rand() % 3) - 1;
			const int Jump = Rand() % 10 == 0;
			const int Boost = Rand() % 40 == 0;
			const int Bx = Boost ? (int)(Rand() % 121) - 60 : 0;
			const int By = Boost ? (int)(Rand() % 121) - 60 : 0;
			const int AngleDeg = Rand() % 360;
			const int Len = Rand() % 600;

			Vel.y += 0.5f;
			Vel.x += Ax * 1.5f;
			if(Jump)
				Vel.y = -12.0f;
			if(Boost)
				Vel = vec2(Bx, By);
			const vec2 PrevPos = Pos;
			bool Grounded = false;
			Collision.MoveBox(&Pos, &Vel, vec2(28.0f, 28.0f), Elasticity, &Grounded);

			SQueries Q;
			Q.Int(Grounded);
			Q.Int(Collision.IsOnGround(Pos, 28.0f));
			Q.Int(Collision.TestBox(Pos, vec2(28.0f, 28.0f)));
			Q.Int(Collision.CheckPoint(Pos));
			const int Restrictions = Collision.GetMoveRestrictions(nullptr, nullptr, Pos, 18.0f, -1);
			Q.Int(Restrictions);
			Q.Int(Collision.GetMoveRestrictions(SwitchActiveCb, nullptr, Pos, 18.0f, -1));
			Q.Vec(ClampVel(Restrictions, Vel));
			const int PureIndex = Collision.GetPureMapIndex(Pos);
			Q.Int(PureIndex);
			Q.Int(Collision.GetTileIndex(PureIndex));
			Q.Int(Collision.GetFrontTileIndex(PureIndex));
			Q.Int(Collision.GetTileFlags(PureIndex));
			Q.Int(Collision.GetFrontTileFlags(PureIndex));
			Q.Int(Collision.IsTeleCheckpoint(PureIndex));
			Q.Int(Collision.IsSpeedup(PureIndex));
			if(Collision.IsSpeedup(PureIndex))
			{
				vec2 Dir;
				int Force, MaxSpeed, Type;
				Collision.GetSpeedup(PureIndex, &Dir, &Force, &MaxSpeed, &Type);
				Q.Vec(Dir);
				Q.Int(Force);
				Q.Int(MaxSpeed);
				Q.Int(Type);
			}
			Q.Int(Collision.IsTune(PureIndex));
			Q.Int(Collision.GetSwitchType(PureIndex));
			Q.Int(Collision.GetSwitchNumber(PureIndex));
			Q.Int(Collision.GetSwitchDelay(PureIndex));
			const int MapIndex = Collision.GetMapIndex(Pos);
			Q.Int(MapIndex);
			Q.Int(Collision.GetTileIndex(MapIndex));
			Q.Int(Collision.IsTeleCheckpoint(MapIndex));
			const std::vector<int> vIndices = Collision.GetMapIndices(PrevPos, Pos, 0);
			Q.Int((int)vIndices.size());
			for(int i : vIndices)
				Q.Int(i);
			Q.Int(Collision.GetCollisionAt(Pos.x, Pos.y));
			Q.Int(Collision.GetFrontCollisionAt(Pos.x, Pos.y));
			const vec2 End = Pos + direction(AngleDeg * (pi / 180.0f)) * (float)Len;
			vec2 Coll, Before;
			int TeleNr = 0;
			Q.Int(Collision.IntersectLineTeleHook(Pos, End, &Coll, &Before, &TeleNr));
			Q.Int(TeleNr);
			Q.Vec(Coll);
			Q.Vec(Before);
			const std::vector<vec2> &vOuts = Collision.TeleOuts(Tick % 8);
			Q.Int((int)vOuts.size());
			if(!vOuts.empty())
				Q.Vec(vOuts[0]);

			fprintf(f, "%d %d %d %d %d %d %d %08x %08x %08x %08x %08x", Ax, Jump, Boost, Bx, By, AngleDeg, Len,
				FloatBits(Pos.x), FloatBits(Pos.y), FloatBits(Vel.x), FloatBits(Vel.y), Q.Crc());
			if(Verbose)
				for(int v : Q.m_vValues)
					fprintf(f, " %x", (unsigned)v);
			fprintf(f, "\n");
		}
	}
	fclose(f);
	return 0;
}

int main(int argc, const char **argv)
{
	CCmdlineFix CmdlineFix(&argc, &argv);
	log_set_global_logger_default();
	if(argc == 5 && str_comp(argv[1], "map") == 0)
		return MapMode(argv[2], argv[3], argv[4]);
	if(argc >= 4 && str_comp(argv[1], "collision") == 0)
	{
		bool HookTele = false, Verbose = false;
		for(int i = 4; i < argc; i++)
		{
			HookTele |= str_comp(argv[i], "hooktele") == 0;
			Verbose |= str_comp(argv[i], "verbose") == 0;
		}
		return CollisionMode(argv[2], argv[3], HookTele, Verbose);
	}
	log_error(TOOL_NAME, "Usage: %s map <map file> <output file> <scratch dir> | collision <map file> <output file> [hooktele] [verbose]", argc > 0 ? argv[0] : TOOL_NAME);
	return -1;
}
