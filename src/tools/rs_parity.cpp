// Parity data for the Rust client (rust-client/): runs the real C++ map loading and
// physics code and writes what it computed, for rust-client/ddnet/tests/parity.rs.
//
// Usage: rs_parity map <map file> <output file> <scratch dir>
//        rs_parity collision <map file> <output file> [hooktele] [verbose]
//        rs_parity core <map file> <output file> [hooktele] [verbose]
//        rs_parity world <map file> <output file> [hooktele] [verbose]
//        rs_parity doors <map file> <output file>
//        rs_parity mapimages <map file> <output file>
//        rs_parity envelopes <map file> <output file>
//        rs_parity envelopes-synthetic <output file>
//        rs_parity screen <output file>
//        rs_parity png <output file> <dir>...
//        rs_parity anims <output file>
//        rs_parity colors <output file>
//        rs_parity interp <output file>
//        rs_parity items <map file> <output file>
//        rs_parity skins <output file> <skins dir>
//        rs_parity camera <output file>
//        rs_parity movement <output file>
//
// "map" loads the map with CMap + CLayers and writes every item, every data blob and
// the physics layers, then loads mutated copies of the file (truncated, bit flips,
// overwritten ints) and writes whether each one loads and what its physics layers are.
//
// "collision" drives boxes through the map with CCollision::MoveBox for many ticks and
// queries the collision functions the game core and prediction use at every tick.
// "hooktele" first turns all tele-in tiles (10, 26, 31, 63) into hook tele-ins (15),
// since none of the bundled maps has hook teleporters.
//
// "core" ticks four interacting CCharacterCores with held random inputs and writes
// their complete state every tick.
//
// "world" ticks four characters through the client prediction (CGameWorld with
// CCharacter, as in CGameClient::OnPredict), spawned on DDNet tiles (freeze, switches,
// speedups, tune zones, ...), and writes their state and the switch states every tick.
//
// "doors" feeds the door lasers the server creates for the map (with every door tile also
// copied to the front layer, half of them with another length), more doors on the same
// tiles and odd lasers a server could send, into CGameWorld snapshot by snapshot (NetObjBegin/NetObjAdd/NetObjEnd as in
// CGameClient::UpdatePrediction), with items leaving, returning and moving, and writes
// the collision door grid after each one.
//
// "png" loads every PNG below the given directories with the C++ PNG loader and
// RGBA conversion. "mapimages" runs the map image decisions of CMapImages on the map
// and on copies with changed image/layer item ints. "envelopes" evaluates the map's
// envelopes like CEnvelopeState; "envelopes-synthetic" random ones of every curve type.
// "screen" maps random cameras to world rectangles with IGraphics::MapScreenToWorld.
// "anims" blends CAnimState like CPlayers::RenderPlayer, "colors" converts tee colours,
// "skins" prepares skin images like CSkins::LoadSkinData. "interp" computes the render
// fractions between ticks, interpolated positions and other players' aim angles.
// "items" extracts projectiles, pickups and lasers from snapshot objects like CItems.

#include <base/color.h>
#include <base/dbg.h>
#include <base/fs.h>
#include <base/io.h>
#include <base/logger.h>
#include <base/math.h>
#include <base/mem.h>
#include <base/os.h>
#include <base/str.h>
#include <base/time.h>
#include <base/vmath.h>

#include <engine/gfx/image_loader.h>
#include <engine/graphics.h>
#include <engine/gfx/image_manipulation.h>
#include <engine/image.h>
#include <engine/map.h>
#include <engine/shared/config.h>
#include <engine/shared/uuid_manager.h>
#include <engine/storage.h>

#include <game/client/animstate.h>
#include <game/client/laser_data.h>
#include <game/client/pickup_data.h>
#include <game/client/projectile_data.h>
#include <game/client/prediction/entities/character.h>
#include <game/client/prediction/entities/dragger.h>
#include <game/client/prediction/entities/laser.h>
#include <game/client/prediction/entities/pickup.h>
#include <game/client/prediction/entities/plasma.h>
#include <game/client/prediction/entities/projectile.h>
#include <game/client/prediction/gameworld.h>
#include <game/collision.h>
#include <game/map/render_map.h>
#include <game/gamecore.h>
#include <game/teamscore.h>
#include <game/layers.h>
#include <game/mapitems.h>

#include <generated/client_data.h>

#include <zlib.h>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <functional>
#include <limits>
#include <map>
#include <string>
#include <tuple>
#include <vector>

static constexpr const char *const TOOL_NAME = "rs_parity";

int CameraMode(const char *pOutFile); // rs_parity_camera.cpp
int MovementMode(const char *pOutFile); // rs_parity_hud.cpp
int MsgsMode(IMap *pMap, CCollision *pCollision, const char *pOutFile); // rs_parity_msgs.cpp
int AntiPingMode(CCollision *pCollision, const char *pOutFile); // rs_parity_antiping.cpp
int ChatMode(const char *pOutFile); // rs_parity_chat.cpp
int ScoreboardMode(const char *pOutFile); // rs_parity_scoreboard.cpp
int WvMode(const char *pOutFile, int NumDirs, const char *const *ppDirs); // rs_parity_sound.cpp

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

// No bundled map has mover tiles (CPickup::Move): every 7th empty game tile becomes one,
// with the rotations MoverSpeed knows and one it does not.
static void MakeMovers(IMap *pMap, CLayers *pLayers)
{
	CTile *pTiles = static_cast<CTile *>(pMap->GetData(pLayers->GameLayer()->m_Data));
	const int Count = pLayers->GameLayer()->m_Width * pLayers->GameLayer()->m_Height;
	const int aFlags[] = {ROTATION_0, ROTATION_90, ROTATION_180, ROTATION_270, TILEFLAG_XFLIP};
	int Made = 0;
	for(int i = 0; pTiles && i < Count; i += 7)
		if(pTiles[i].m_Index == 0)
		{
			pTiles[i].m_Index = Made % 2 ? TILE_CP_F : TILE_CP;
			pTiles[i].m_Flags = aFlags[Made % 5];
			Made++;
		}
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
	{
		MakeHookTeleporters(pMap.get(), &Layers);
		MakeMovers(pMap.get(), &Layers);
	}
	CCollision Collision;
	Collision.Init(&Layers);

	const int Width = Collision.GetWidth(), Height = Collision.GetHeight();
	std::vector<int> vSwitchTypes;
	if(Collision.SwitchLayer())
		for(int i = 0; i < Width * Height; i++)
			vSwitchTypes.push_back(Collision.GetSwitchType(i));
	fprintf(f, "static %d %08x\n", Collision.m_HighestSwitchNumber, Crc(vSwitchTypes.data(), vSwitchTypes.size() * sizeof(int)));
	// the mover tiles (CPickup::Move) of the whole map
	SQueries Movers;
	for(int i = 0; i < Width * Height; i++)
	{
		vec2 Speed(0.0f, 0.0f);
		const int Mover = Collision.MoverSpeed((i % Width) * 32 + 5, (i / Width) * 32 + 30, &Speed);
		if(Mover)
		{
			Movers.Int(i);
			Movers.Int(Mover);
			Movers.Vec(Speed);
		}
	}
	fprintf(f, "movers %d %08x\n", (int)Movers.m_vValues.size() / 4, Movers.Crc());

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
			// weapons (M8.2)
			Q.Int(Collision.IntersectLine(Pos, End, &Coll, &Before));
			Q.Vec(Coll);
			Q.Vec(Before);
			Q.Int(Collision.IntersectLineTeleWeapon(Pos, End, &Coll, &Before));
			Q.Vec(Coll);
			Q.Vec(Before);
			vec2 PointPos = Pos, PointVel = (End - Pos) * 0.05f;
			int Bounces = -1;
			Collision.MovePoint(&PointPos, &PointVel, Elasticity.x, &Bounces);
			Q.Vec(PointPos);
			Q.Vec(PointVel);
			Q.Int(Bounces);
			PointPos = Pos;
			PointVel = Vel * 4.0f;
			Collision.MovePoint(&PointPos, &PointVel, 1.0f, nullptr);
			Q.Vec(PointPos);
			Q.Vec(PointVel);
			vec2 Speed(0.0f, 0.0f);
			Q.Int(Collision.MoverSpeed(round_to_int(Pos.x), round_to_int(Pos.y), &Speed));
			Q.Vec(Speed);

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

// Everything CCharacterCore keeps between ticks, as ints (floats by their bits).
static void CoreState(SQueries &Q, const CCharacterCore &Core)
{
	CNetObj_CharacterCore Net = {};
	Core.Write(&Net);
	for(int i = 1; i < (int)(sizeof(Net) / sizeof(int)); i++) // without m_Tick
		Q.Int(((const int *)&Net)[i]);
	Q.Vec(Core.m_Pos);
	Q.Vec(Core.m_Vel);
	Q.Vec(Core.m_HookPos);
	Q.Vec(Core.m_HookDir);
	Q.Vec(Core.m_HookTeleBase);
	Q.Int(Core.m_HookTick);
	Q.Int(Core.m_HookState);
	Q.Int(Core.HookedPlayer());
	Q.Int(Core.m_Jumped);
	Q.Int(Core.m_JumpedTotal);
	Q.Int(Core.m_Jumps);
	Q.Int(Core.m_Direction);
	Q.Int(Core.m_Angle);
	Q.Int(Core.m_TriggeredEvents);
	Q.Int(Core.m_Colliding);
	Q.Int(Core.m_LeftWall);
	Q.Int(Core.m_NewHook);
	Q.Int(Core.m_Reset);
	Q.Int((int)Core.m_AttachedPlayers.size());
	for(int Id : Core.m_AttachedPlayers)
		Q.Int(Id);
}

// "run <run> <start tile index>" then per tick, per character,
// "<dir> <jump> <hook> <target x> <target y>" and after all characters
// "| <pos x> <pos y> <crc of the core state>" per character (hex float bits).
static constexpr int CORE_CHARS = 4;
static int CoreMode(const char *pMapFile, const char *pOutFile, bool HookTele, bool Verbose)
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

	for(int Run = 0; Run < 2; Run++)
	{
		CWorldCore World;
		World.InitSwitchers(Collision.m_HighestSwitchNumber); // like CGameClient::OnConnected
		CTeamsCore Teams;
		CTuningParams Tuning;
		if(Run == 1)
		{
			Tuning.m_Gravity = 0.3f;
			Tuning.m_HookLength = 500.0f;
			Tuning.m_HookFireSpeed = 120.0f;
			Tuning.m_HookDragAccel = 5.0f;
			Tuning.m_HookDragSpeed = 20.0f;
			Tuning.m_GroundControlSpeed = 13.0f;
			Tuning.m_AirFriction = 0.9f;
			Tuning.m_VelrampStart = 300;
			Tuning.m_HookDuration = 2.5f;
			Tuning.m_GroundElasticityX = 0.5f;
			Tuning.m_GroundElasticityY = 0.3f;
			Teams.Team(2, 1);
			Teams.SetSolo(3, true);
		}
		CCharacterCore aCores[CORE_CHARS]{};
		int Index = 0;
		for(int Try = 0; Try < 100; Try++)
		{
			Index = Rand() % (Width * Height);
			if(Collision.GetTileIndex(Index) == 0)
				break;
		}
		fprintf(f, "run %d %d\n", Run, Index);
		const int aJumps[CORE_CHARS] = {2, 3, 1, 0};
		for(int i = 0; i < CORE_CHARS; i++)
		{
			CCharacterCore &Core = aCores[i];
			Core.Init(&World, &Collision, &Teams);
			Core.Reset();
			Core.m_Id = i;
			Core.m_Tuning = Tuning;
			Core.m_Jumps = aJumps[i];
			Core.m_Pos = vec2((Index % Width) * 32.0f + 16.0f + i * 30 - 45, (Index / Width) * 32.0f + 16.0f);
			World.m_apCharacters[i] = &Core;
		}

		CNetObj_PlayerInput aInputs[CORE_CHARS] = {};
		int aHold[CORE_CHARS][3] = {}; // ticks left for direction, hook, jump
		for(int Tick = 0; Tick < 600; Tick++)
		{
			for(int i = 0; i < CORE_CHARS; i++)
			{
				CNetObj_PlayerInput &In = aInputs[i];
				if(--aHold[i][0] <= 0)
				{
					In.m_Direction = (int)(Rand() % 3) - 1;
					aHold[i][0] = 1 + Rand() % 40;
				}
				if(--aHold[i][1] <= 0)
				{
					In.m_Hook = Rand() % 2;
					aHold[i][1] = 1 + Rand() % 60;
					if(Rand() % 3 == 0)
					{
						// aim at another character
						const vec2 Diff = aCores[(i + 1 + Rand() % (CORE_CHARS - 1)) % CORE_CHARS].m_Pos - aCores[i].m_Pos;
						In.m_TargetX = (int)Diff.x;
						In.m_TargetY = (int)Diff.y;
					}
					else
					{
						const float Angle = (Rand() % 360) * (pi / 180.0f);
						In.m_TargetX = (int)(std::cos(Angle) * (50 + Rand() % 350));
						In.m_TargetY = (int)(std::sin(Angle) * (50 + Rand() % 350));
					}
				}
				if(--aHold[i][2] <= 0)
				{
					In.m_Jump = Rand() % 4 == 0;
					aHold[i][2] = 1 + Rand() % 15;
				}
				fprintf(f, "%d %d %d %d %d ", In.m_Direction, In.m_Jump, In.m_Hook, In.m_TargetX, In.m_TargetY);
				aCores[i].m_Input = In;
			}
			// like CGameWorld::Tick with entities ticked in id order: Tick, then Move + Quantize
			for(auto &Core : aCores)
				Core.Tick(true);
			for(auto &Core : aCores)
			{
				Core.Move();
				Core.Quantize();
			}
			fprintf(f, "|");
			for(auto &Core : aCores)
			{
				SQueries Q;
				CoreState(Q, Core);
				fprintf(f, " %08x %08x %08x", FloatBits(Core.m_Pos.x), FloatBits(Core.m_Pos.y), Q.Crc());
				if(Verbose)
				{
					fprintf(f, " [");
					for(int v : Q.m_vValues)
						fprintf(f, " %x", (unsigned)v);
					fprintf(f, " ]");
				}
			}
			fprintf(f, "\n");
		}
	}
	fclose(f);
	return 0;
}

// Spawn spots are grouped by what is on them; 0 for no spot (solid or nothing special).
static long long WorldSpotKind(const CCollision &Collision, int Index)
{
	const int Tile = Collision.GetTileIndex(Index);
	if(Tile == TILE_SOLID || Tile == TILE_NOHOOK)
		return 0;
	vec2 Direction;
	int Force = 0, MaxSpeed = 0, Type = 0;
	if(Collision.IsSpeedup(Index))
		Collision.GetSpeedup(Index, &Direction, &Force, &MaxSpeed, &Type);
	return Tile | Collision.GetFrontTileIndex(Index) << 8 | Collision.GetSwitchType(Index) << 16 | (long long)Type << 24 |
	       (long long)(Type != 0 && MaxSpeed == 0) << 32 | (long long)(Collision.IsTune(Index) != 0) << 33 |
	       (long long)(Collision.IsTeleCheckpoint(Index) != 0) << 34;
}

// No bundled map has speedups of the new type (TILE_SPEED_BOOST), so the world mode
// turns every second speedup tile into one. Also done by parity.rs.
static void MakeNewSpeedups(IMap *pMap, CLayers *pLayers)
{
	if(!pLayers->SpeedupLayer())
		return;
	CSpeedupTile *pSpeedup = static_cast<CSpeedupTile *>(pMap->GetData(pLayers->SpeedupLayer()->m_Speedup));
	const int Count = pLayers->GameLayer()->m_Width * pLayers->GameLayer()->m_Height;
	for(int i = 1; pSpeedup && i < Count; i += 2)
		pSpeedup[i].m_Type = TILE_SPEED_BOOST;
}

// Tuning of tune zone `Zone` (zone 0 keeps the defaults). Also in parity.rs.
static void ZoneTuning(CTuningParams *pTuning, int Zone)
{
	if(Zone == 0)
		return;
	pTuning->m_Gravity = 0.5f + (Zone % 5) * 0.1f;
	pTuning->m_GroundControlSpeed = 10.0f + Zone % 4;
	pTuning->m_AirJumpImpulse = 12.0f - Zone % 3;
	pTuning->m_JetpackStrength = 400.0f + Zone * 10;
	pTuning->m_VelrampCurvature = 1.4f + (Zone % 3) * 0.3f;
	pTuning->m_PlayerCollision = Zone % 4 != 1;
	pTuning->m_PlayerHooking = Zone % 4 != 2;
}

static void WorldState(SQueries &Q, CCharacter *pChar)
{
	const CCharacterCore &Core = *pChar->Core();
	CoreState(Q, Core);
	Q.Int(Core.m_ActiveWeapon);
	for(const auto &Weapon : Core.m_aWeapons)
	{
		Q.Int(Weapon.m_Got);
		Q.Int(Weapon.m_Ammo);
	}
	Q.Vec(Core.m_Ninja.m_ActivationDir);
	Q.Int(Core.m_Ninja.m_ActivationTick);
	Q.Int(Core.m_Ninja.m_CurrentMoveTime);
	Q.Int(Core.m_Ninja.m_OldVelAmount);
	const bool aFlags[] = {Core.m_Solo, Core.m_Jetpack, Core.m_CollisionDisabled, Core.m_EndlessHook, Core.m_EndlessJump,
		Core.m_HammerHitDisabled, Core.m_GrenadeHitDisabled, Core.m_LaserHitDisabled, Core.m_ShotgunHitDisabled,
		Core.m_HookHitDisabled, Core.m_Super, Core.m_Invincible, Core.m_HasTelegunGun, Core.m_HasTelegunGrenade,
		Core.m_HasTelegunLaser, Core.m_IsInFreeze, Core.m_DeepFrozen, Core.m_LiveFrozen};
	for(bool Flag : aFlags)
		Q.Int(Flag);
	Q.Int(Core.m_FreezeStart);
	Q.Int(Core.m_FreezeEnd);
	for(int i = 0; i < CTuningParams::Num(); i++)
		Q.Int(((const CTuneParam *)&Core.m_Tuning)[i].Get());
	Q.Int(pChar->m_FreezeTime);
	Q.Int(pChar->m_FrozenLastTick);
	Q.Int(pChar->m_TeleCheckpoint);
	Q.Int(pChar->m_TileIndex);
	Q.Int(pChar->m_TileFIndex);
	Q.Int(pChar->m_LastRefillJumps);
	Q.Vec(pChar->m_PrevPos);
	Q.Vec(pChar->m_PrevPrevPos);
	Q.Vec(pChar->GetPos());
	Q.Int(pChar->GetLastWeapon());
	Q.Int(pChar->IsInterfering());
	Q.Int(pChar->GetPureTuneZone());
	Q.Int(pChar->GetOverriddenTuneZone());
	Q.Int(pChar->Team());
	Q.Int(pChar->TeamsCore()->GetSolo(pChar->GetCid()));
}

// For rs_parity_msgs.cpp.
unsigned CharacterStateCrc(CCharacter *pChar)
{
	SQueries Q;
	WorldState(Q, pChar);
	return Q.Crc();
}

// "run <run> <spawn index> <number of kinds of spawn spots>" then per tick, per character in id
// order, "<dir> <jump> <hook> <target x> <target y> <fire> <next> <prev> <wanted> <flags>"
// and after all characters "| <pos x> <pos y> <crc of the state>" per character (hex float
// bits) and "| <crc of the switch states>".
static constexpr int WORLD_RUNS = 24;
static constexpr int WORLD_TICKS = 100;
static constexpr int WORLD_START_TICK = 10000;
static int WorldMode(const char *pMapFile, const char *pOutFile, bool HookTele, bool Verbose)
{
	std::unique_ptr<IStorage> pStorage = CreateLocalStorage();
	std::unique_ptr<IMap> pMap = CreateMap();
	CLayers Layers;
	FILE *f = fopen(pOutFile, "wb");
	if(!pStorage || !f || !LoadMapAndLayers(pStorage.get(), pMapFile, pMap.get(), &Layers))
		return -1;
	if(HookTele)
		MakeHookTeleporters(pMap.get(), &Layers);
	MakeNewSpeedups(pMap.get(), &Layers);
	CCollision Collision;
	Collision.Init(&Layers);
	const int Width = Collision.GetWidth(), Height = Collision.GetHeight();
	std::map<long long, std::vector<int>> SpotKinds;
	for(int i = 0; i < Width * Height; i++)
		if(const long long Kind = WorldSpotKind(Collision, i))
			SpotKinds[Kind].push_back(i);
	// only kinds with something the prediction reacts to, if the map has any
	const auto IsEffectTile = [](int Tile) {
		const int aTiles[] = {TILE_FREEZE, TILE_UNFREEZE, TILE_DFREEZE, TILE_DUNFREEZE, TILE_LFREEZE, TILE_LUNFREEZE,
			TILE_EHOOK_ENABLE, TILE_EHOOK_DISABLE, TILE_HIT_ENABLE, TILE_HIT_DISABLE, TILE_NPC_ENABLE, TILE_NPC_DISABLE,
			TILE_NPH_ENABLE, TILE_NPH_DISABLE, TILE_UNLIMITED_JUMPS_ENABLE, TILE_UNLIMITED_JUMPS_DISABLE, TILE_WALLJUMP,
			TILE_JETPACK_ENABLE, TILE_JETPACK_DISABLE, TILE_SOLO_ENABLE, TILE_SOLO_DISABLE, TILE_REFILL_JUMPS,
			TILE_TELE_GUN_ENABLE, TILE_TELE_GUN_DISABLE, TILE_TELE_GRENADE_ENABLE, TILE_TELE_GRENADE_DISABLE,
			TILE_TELE_LASER_ENABLE, TILE_TELE_LASER_DISABLE};
		return std::find(std::begin(aTiles), std::end(aTiles), Tile) != std::end(aTiles);
	};
	std::vector<const std::vector<int> *> vpKinds;
	for(const auto &[Kind, vSpots] : SpotKinds)
		if(IsEffectTile(Kind & 0xff) || IsEffectTile((Kind >> 8) & 0xff) || (Kind >> 16) != 0)
			vpKinds.push_back(&vSpots);
	if(vpKinds.empty())
		for(const auto &[Kind, vSpots] : SpotKinds)
			vpKinds.push_back(&vSpots);

	// the tool's g_Config is zeroed, set what the prediction reads to the defaults
	g_Config.m_SvFreezeDelay = 3;
	g_Config.m_ClAntiPingPlayers = 1;
	g_Config.m_SvDeepfly = 1;
	g_Config.m_SvOldTeleportHook = 0;

	static CTuningParams s_aTuning[TuneZone::NUM];
	for(int z = 0; z < TuneZone::NUM; z++)
	{
		s_aTuning[z] = CTuningParams();
		ZoneTuning(&s_aTuning[z], z);
	}

	const int aIds[CORE_CHARS] = {3, 0, 6, 1}; // inserted in this order, so ticked 1, 6, 0, 3
	for(int Run = 0; Run < WORLD_RUNS; Run++)
	{
		int Index = 0;
		if(vpKinds.empty())
		{
			for(int Try = 0; Try < 100; Try++)
			{
				Index = Rand() % (Width * Height);
				if(Collision.GetTileIndex(Index) == 0)
					break;
			}
		}
		else
		{
			// go through the kinds of spots, so rare tiles get their turn
			const size_t Kinds = vpKinds.size();
			const std::vector<int> &vSpots = *vpKinds[Kinds > WORLD_RUNS ? Run * Kinds / WORLD_RUNS : Run % Kinds];
			Index = vSpots[Rand() % vSpots.size()];
		}
		fprintf(f, "run %d %d %d\n", Run, Index, (int)vpKinds.size());

		CGameWorld World;
		World.Init(&Collision, s_aTuning, nullptr);
		World.m_Core.InitSwitchers(Collision.m_HighestSwitchNumber);
		World.m_GameTick = WORLD_START_TICK;
		auto &Config = World.m_WorldConfig;
		const int Mode = Run % 4;
		Config.m_IsDDRace = true;
		Config.m_IsVanilla = false;
		Config.m_IsFNG = false;
		Config.m_InfiniteAmmo = true;
		Config.m_PredictTiles = Mode != 3;
		Config.m_PredictFreeze = Mode != 2;
		Config.m_PredictWeapons = false;
		Config.m_PredictDDRace = Mode != 3;
		Config.m_UseTuneZones = Mode != 2;
		Config.m_BugDDRaceInput = Mode == 3;
		Config.m_NoWeakHookAndBounce = Mode % 2 == 1;
		if(Mode == 1)
			World.m_Teams.Team(aIds[2], 1);

		for(int i = 0; i < CORE_CHARS; i++)
		{
			CNetObj_Character Obj = {};
			Obj.m_X = (Index % Width) * 32 + 16 + i * 30 - 45;
			Obj.m_Y = (Index / Width) * 32 + 16;
			Obj.m_HookX = Obj.m_X;
			Obj.m_HookY = Obj.m_Y;
			Obj.m_HookedPlayer = -1;
			Obj.m_Angle = i * 100;
			Obj.m_Weapon = WEAPON_GUN;
			Obj.m_AmmoCount = 10;
			Obj.m_AttackTick = WORLD_START_TICK - 3;
			CNetObj_DDNetCharacter Ext = {};
			Ext.m_Flags = CHARACTERFLAG_WEAPON_HAMMER | CHARACTERFLAG_WEAPON_GUN;
			Ext.m_Jumps = 2 + i % 2;
			Ext.m_JumpedTotal = -1;
			Ext.m_NinjaActivationTick = -1;
			Ext.m_FreezeStart = -1;
			Ext.m_StrongWeakId = i;
			Ext.m_TuneZoneOverride = TuneZone::OVERRIDE_NONE;
			if(Mode == 0 && i == 1)
				Ext.m_FreezeEnd = WORLD_START_TICK + 60;
			if(Mode == 0 && i == 2)
				Ext.m_Flags |= CHARACTERFLAG_ENDLESS_HOOK | CHARACTERFLAG_ENDLESS_JUMP;
			if(Mode == 1 && i == 3)
				Ext.m_Flags |= CHARACTERFLAG_SUPER;
			if(Mode == 2 && i == 0)
				Ext.m_Flags |= CHARACTERFLAG_JETPACK;
			if(Mode == 2 && i == 1)
			{
				Ext.m_Flags |= CHARACTERFLAG_WEAPON_NINJA;
				Ext.m_NinjaActivationTick = WORLD_START_TICK - 700;
				Obj.m_Weapon = WEAPON_NINJA;
			}
			if(Mode == 2 && i == 2)
			{
				Ext.m_Flags |= CHARACTERFLAG_COLLISION_DISABLED | CHARACTERFLAG_WEAPON_SHOTGUN;
				Ext.m_TuneZoneOverride = 3;
				Ext.m_TargetX = 100;
				Ext.m_TargetY = -50;
			}
			if(Mode == 3 && i == 3)
				Ext.m_Flags |= CHARACTERFLAG_SOLO;
			if(Mode == 3 && i == 1)
				Ext.m_FreezeEnd = -1;
			CCharacter *pChar = new CCharacter(&World, aIds[i], &Obj, &Ext);
			World.InsertEntity(pChar);
			if(i == 0)
				pChar->Read(&Obj, &Ext, true); // the local player
		}

		CNetObj_PlayerInput aInputs[MAX_CLIENTS] = {};
		int aHold[CORE_CHARS][4] = {}; // ticks left for direction, hook, jump, fire
		for(int Tick = 1; Tick <= WORLD_TICKS; Tick++)
		{
			for(int i = 0; i < CORE_CHARS; i++)
			{
				CNetObj_PlayerInput &In = aInputs[aIds[i]];
				if(--aHold[i][0] <= 0)
				{
					In.m_Direction = (int)(Rand() % 3) - 1;
					aHold[i][0] = 1 + Rand() % 40;
				}
				if(--aHold[i][1] <= 0)
				{
					In.m_Hook = Rand() % 2;
					aHold[i][1] = 1 + Rand() % 60;
					const float Angle = (Rand() % 360) * (pi / 180.0f);
					const int Length = Rand() % 350;
					In.m_TargetX = (int)(std::cos(Angle) * Length);
					In.m_TargetY = (int)(std::sin(Angle) * Length);
				}
				if(--aHold[i][2] <= 0)
				{
					In.m_Jump = Rand() % 4 == 0;
					aHold[i][2] = 1 + Rand() % 15;
				}
				if(--aHold[i][3] <= 0)
				{
					if((In.m_Fire & 1) != (int)(Rand() % 2))
						In.m_Fire = (In.m_Fire + 1) & INPUT_STATE_MASK;
					aHold[i][3] = 1 + Rand() % 20;
				}
				const int Extra = Rand() % 100;
				In.m_NextWeapon += Extra == 0 ? 2 : 0;
				In.m_PrevWeapon += Extra == 1 ? 2 : 0;
				In.m_WantedWeapon = Extra < 6 ? Extra % 4 : 0;
				In.m_PlayerFlags = Extra >= 95 ? PLAYERFLAG_SPEC_CAM : Extra >= 90 ? PLAYERFLAG_CHATTING : PLAYERFLAG_PLAYING;
			}
			for(int Id = 0; Id < MAX_CLIENTS; Id++)
			{
				if(CCharacter *pChar = World.GetCharacterById(Id))
				{
					const CNetObj_PlayerInput &In = aInputs[Id];
					fprintf(f, "%d %d %d %d %d %d %d %d %d %d ", In.m_Direction, In.m_Jump, In.m_Hook, In.m_TargetX, In.m_TargetY,
						In.m_Fire, In.m_NextWeapon, In.m_PrevWeapon, In.m_WantedWeapon, In.m_PlayerFlags);
					pChar->OnDirectInput(&In);
				}
			}
			World.m_GameTick = WORLD_START_TICK + Tick;
			for(int Id = 0; Id < MAX_CLIENTS; Id++)
				if(CCharacter *pChar = World.GetCharacterById(Id))
					pChar->OnPredictedInput(&aInputs[Id]);
			World.Tick();

			fprintf(f, "|");
			for(int Id = 0; Id < MAX_CLIENTS; Id++)
			{
				CCharacter *pChar = World.GetCharacterById(Id);
				if(!pChar)
					continue;
				SQueries Q;
				WorldState(Q, pChar);
				fprintf(f, " %08x %08x %08x", FloatBits(pChar->GetPos().x), FloatBits(pChar->GetPos().y), Q.Crc());
				if(Verbose)
				{
					fprintf(f, " [");
					for(int v : Q.m_vValues)
						fprintf(f, " %x", (unsigned)v);
					fprintf(f, " ]");
				}
			}
			SQueries Switches;
			for(const auto &Switcher : World.Switchers())
				for(int t = 0; t < NUM_DDRACE_TEAMS; t++)
				{
					Switches.Int(Switcher.m_aStatus[t]);
					Switches.Int(Switcher.m_aEndTick[t]);
					Switches.Int(Switcher.m_aType[t]);
					Switches.Int(Switcher.m_aLastUpdateTick[t]);
				}
			fprintf(f, " | %08x\n", Switches.Crc());
		}
	}
	fclose(f);
	return 0;
}

// Access to private members for the weapons dump: explicit instantiations may name them.
template<typename Tag, typename Tag::Type Ptr>
struct SRobPrivate
{
	friend typename Tag::Type Get(Tag) { return Ptr; }
};
#define RS_PRIVATE(Name, Class, MemberType, Member) \
	struct Name \
	{ \
		using Type = MemberType Class::*; \
		friend Type Get(Name); \
	}; \
	template struct SRobPrivate<Name, &Class::Member>;
RS_PRIVATE(SCharReload, CCharacter, int, m_ReloadTimer)
RS_PRIVATE(SCharQueued, CCharacter, int, m_QueuedWeapon)
RS_PRIVATE(SCharPain, CCharacter, int, m_PainSoundTimer)
RS_PRIVATE(SCharNumHit, CCharacter, int, m_NumObjectsHit)
RS_PRIVATE(SCharMoveRestrictions, CCharacter, int, m_MoveRestrictions)
RS_PRIVATE(SProjDir, CProjectile, vec2, m_Direction)
RS_PRIVATE(SProjLife, CProjectile, int, m_LifeSpan)
RS_PRIVATE(SProjOwner, CProjectile, int, m_Owner)
RS_PRIVATE(SProjType, CProjectile, int, m_Type)
RS_PRIVATE(SProjStart, CProjectile, int, m_StartTick)
RS_PRIVATE(SProjExplosive, CProjectile, bool, m_Explosive)
RS_PRIVATE(SProjBouncing, CProjectile, int, m_Bouncing)
RS_PRIVATE(SProjFreeze, CProjectile, bool, m_Freeze)
RS_PRIVATE(SProjTune, CProjectile, int, m_TuneZone)
RS_PRIVATE(SLaserFrom, CLaser, vec2, m_From)
RS_PRIVATE(SLaserDir, CLaser, vec2, m_Dir)
RS_PRIVATE(SLaserEnergy, CLaser, float, m_Energy)
RS_PRIVATE(SLaserBounces, CLaser, int, m_Bounces)
RS_PRIVATE(SLaserEval, CLaser, int, m_EvalTick)
RS_PRIVATE(SLaserOwner, CLaser, int, m_Owner)
RS_PRIVATE(SLaserZero, CLaser, bool, m_ZeroEnergyBounceInLastTick)
RS_PRIVATE(SLaserPrev, CLaser, vec2, m_PrevPos)
RS_PRIVATE(SLaserType, CLaser, int, m_Type)
RS_PRIVATE(SLaserTune, CLaser, int, m_TuneZone)
RS_PRIVATE(SPickupCore, CPickup, vec2, m_Core)
RS_PRIVATE(SDraggerCore, CDragger, vec2, m_Core)
RS_PRIVATE(SDraggerWalls, CDragger, bool, m_IgnoreWalls)
RS_PRIVATE(SDraggerTarget, CDragger, int, m_TargetId)
RS_PRIVATE(SPlasmaCore, CPlasma, vec2, m_Core)
RS_PRIVATE(SPlasmaFreeze, CPlasma, bool, m_Freeze)
RS_PRIVATE(SPlasmaExplosive, CPlasma, bool, m_Explosive)
RS_PRIVATE(SPlasmaFor, CPlasma, int, m_ForClientId)
RS_PRIVATE(SPlasmaEval, CPlasma, int, m_EvalTick)
RS_PRIVATE(SPlasmaLife, CPlasma, int, m_LifeTime)

// WorldState plus the weapon state of a character.
static void WeaponCharState(SQueries &Q, CCharacter *pChar)
{
	WorldState(Q, pChar);
	Q.Int(pChar->*Get(SCharReload()));
	Q.Int(pChar->GetAttackTick());
	Q.Int(pChar->*Get(SCharQueued()));
	Q.Int(pChar->*Get(SCharPain()));
	Q.Int(pChar->*Get(SCharNumHit()));
	Q.Int(pChar->*Get(SCharMoveRestrictions()));
	Q.Int(pChar->m_GameTeam);
	Q.Int(pChar->m_LastSnapWeapon);
}

// Every non-character entity, list by list in ENTTYPE order.
static void WeaponEntityState(SQueries &Q, CGameWorld &World)
{
	for(CEntity *pEnt = World.FindFirst(CGameWorld::ENTTYPE_PROJECTILE); pEnt; pEnt = pEnt->TypeNext())
	{
		CProjectile *p = static_cast<CProjectile *>(pEnt);
		Q.Int(p->GetId());
		Q.Vec(p->m_Pos);
		Q.Vec(p->*Get(SProjDir()));
		Q.Int(p->*Get(SProjLife()));
		Q.Int(p->*Get(SProjOwner()));
		Q.Int(p->*Get(SProjType()));
		Q.Int(p->*Get(SProjStart()));
		Q.Int(p->*Get(SProjExplosive()));
		Q.Int(p->m_Layer);
		Q.Int(p->m_Number);
		Q.Int(p->*Get(SProjBouncing()));
		Q.Int(p->*Get(SProjFreeze()));
		Q.Int(p->*Get(SProjTune()));
	}
	Q.Int(-1);
	for(CEntity *pEnt = World.FindFirst(CGameWorld::ENTTYPE_LASER); pEnt; pEnt = pEnt->TypeNext())
	{
		CLaser *p = static_cast<CLaser *>(pEnt);
		Q.Int(p->GetId());
		Q.Vec(p->m_Pos);
		Q.Vec(p->*Get(SLaserFrom()));
		Q.Vec(p->*Get(SLaserDir()));
		Q.Vec(p->*Get(SLaserPrev()));
		Q.Float(p->*Get(SLaserEnergy()));
		Q.Int(p->*Get(SLaserBounces()));
		Q.Int(p->*Get(SLaserEval()));
		Q.Int(p->*Get(SLaserOwner()));
		Q.Int(p->*Get(SLaserType()));
		Q.Int(p->*Get(SLaserZero()));
		Q.Int(p->*Get(SLaserTune()));
	}
	Q.Int(-1);
	for(CEntity *pEnt = World.FindFirst(CGameWorld::ENTTYPE_DRAGGER); pEnt; pEnt = pEnt->TypeNext())
	{
		CDragger *p = static_cast<CDragger *>(pEnt);
		Q.Int(p->GetId());
		Q.Vec(p->m_Pos);
		Q.Vec(p->*Get(SDraggerCore()));
		Q.Float(p->GetStrength());
		Q.Int(p->*Get(SDraggerWalls()));
		Q.Int(p->*Get(SDraggerTarget()));
		Q.Int(p->m_Number);
		Q.Int(p->m_Layer);
	}
	Q.Int(-1);
	for(CEntity *pEnt = World.FindFirst(CGameWorld::ENTTYPE_PLASMA); pEnt; pEnt = pEnt->TypeNext())
	{
		CPlasma *p = static_cast<CPlasma *>(pEnt);
		Q.Int(p->GetId());
		Q.Vec(p->m_Pos);
		Q.Vec(p->*Get(SPlasmaCore()));
		Q.Int(p->*Get(SPlasmaFreeze()));
		Q.Int(p->*Get(SPlasmaExplosive()));
		Q.Int(p->*Get(SPlasmaFor()));
		Q.Int(p->*Get(SPlasmaEval()));
		Q.Int(p->*Get(SPlasmaLife()));
		Q.Int(p->m_Number);
		Q.Int(p->m_Layer);
	}
	Q.Int(-1);
	for(CEntity *pEnt = World.FindFirst(CGameWorld::ENTTYPE_PICKUP); pEnt; pEnt = pEnt->TypeNext())
	{
		CPickup *p = static_cast<CPickup *>(pEnt);
		Q.Int(p->GetId());
		Q.Vec(p->m_Pos);
		Q.Int(p->Type());
		Q.Int(p->Subtype());
		Q.Int(p->Flags());
		Q.Int(p->m_Number);
		Q.Int(p->m_Layer);
		Q.Vec(p->*Get(SPickupCore()));
		Q.Int(p->InDDNetTile());
	}
}

static int EntityCount(CGameWorld &World, int Type)
{
	int n = 0;
	for(CEntity *pEnt = World.FindFirst(Type); pEnt; pEnt = pEnt->TypeNext())
		n++;
	return n;
}

// ZoneTuning plus weapon parameters per zone.
static void WeaponZoneTuning(CTuningParams *pTuning, int Zone)
{
	ZoneTuning(pTuning, Zone);
	if(Zone % 3 == 1)
	{
		pTuning->m_LaserReach = 400.0f;
		pTuning->m_GrenadeCurvature = 4.0f;
		pTuning->m_ExplosionStrength = 3.0f;
		pTuning->m_LaserBounceNum = 1;
	}
	else if(Zone % 3 == 2)
	{
		pTuning->m_GunSpeed = 1400.0f;
		pTuning->m_ShotgunStrength = 25.0f;
		pTuning->m_HammerStrength = 3.0f;
		pTuning->m_GunFireDelay = 200;
	}
}

static constexpr int WEAPON_RUNS = 30;
static constexpr int WEAPON_TICKS = 150;
static constexpr int WEAPON_CHARS = 4;

// Weapons and entities in the prediction world (M8.3): characters with every weapon
// firing at each other, plus pickups, draggers, turret projectiles and plasma, in six
// world configurations (DDRace, DDRace with no weak hook/old laser/teams, vanilla, FNG,
// solo with limited ammo, changed global tuning without zones).
static int WeaponsMode(const char *pMapFile, const char *pOutFile, bool Verbose)
{
	std::unique_ptr<IStorage> pStorage = CreateLocalStorage();
	std::unique_ptr<IMap> pMap = CreateMap();
	CLayers Layers;
	FILE *f = fopen(pOutFile, "wb");
	if(!pStorage || !f || !LoadMapAndLayers(pStorage.get(), pMapFile, pMap.get(), &Layers))
		return -1;
	MakeMovers(pMap.get(), &Layers);
	CCollision Collision;
	Collision.Init(&Layers);
	const int Width = Collision.GetWidth(), Height = Collision.GetHeight();

	g_Config.m_SvFreezeDelay = 3;
	g_Config.m_ClAntiPingPlayers = 1;
	g_Config.m_SvDeepfly = 1;
	g_Config.m_SvHit = 1;
	g_Config.m_SvDraggerRange = 700;
	g_Config.m_SvOldTeleportHook = 0;

	static CTuningParams s_aTuning[TuneZone::NUM];
	const int aIds[WEAPON_CHARS] = {3, 0, 6, 1};
	const int aPickupTypes[] = {POWERUP_FREEZE, POWERUP_ARMOR, POWERUP_WEAPON, POWERUP_NINJA, POWERUP_ARMOR_SHOTGUN,
		POWERUP_ARMOR_GRENADE, POWERUP_ARMOR_NINJA, POWERUP_ARMOR_LASER, POWERUP_HEALTH};
	const auto Free = [&](int x, int y) {
		const int t = Collision.GetTileIndex(std::clamp(y, 0, Height - 1) * Width + std::clamp(x, 0, Width - 1));
		return t != TILE_SOLID && t != TILE_NOHOOK;
	};
	for(int Run = 0; Run < WEAPON_RUNS; Run++)
	{
		int Index = 0;
		for(int Try = 0; Try < 300; Try++)
		{
			Index = Rand() % (Width * Height);
			const int x = Index % Width, y = Index / Width;
			if(Free(x - 2, y) && Free(x - 1, y) && Free(x, y) && Free(x + 1, y) && Free(x + 2, y))
				break;
		}
		const int Mode = Run % 6;
		fprintf(f, "run %d %d %d\n", Run, Index, Mode);

		for(int z = 0; z < TuneZone::NUM; z++)
		{
			s_aTuning[z] = CTuningParams();
			WeaponZoneTuning(&s_aTuning[z], z);
		}
		if(Mode == 5)
		{
			CTuningParams &T = s_aTuning[0];
			T.m_LaserBounceNum = 2;
			T.m_LaserBounceCost = 50;
			T.m_LaserBounceDelay = 300;
			T.m_ShotgunStrength = 20.0f;
			T.m_ExplosionStrength = 9.0f;
			T.m_HammerStrength = 2.0f;
			T.m_HammerHitFireDelay = 200;
			T.m_GrenadeLifetime = 1.5f;
			T.m_GunLifetime = 0.5f;
			T.m_ShotgunSpeeddiff = 0.6f;
		}
		fprintf(f, "tune0");
		for(int i = 0; i < CTuningParams::Num(); i++)
			fprintf(f, " %d", ((const CTuneParam *)&s_aTuning[0])[i].Get());
		fprintf(f, "\n");

		CGameWorld World;
		World.Init(&Collision, s_aTuning, nullptr);
		World.m_Core.InitSwitchers(Collision.m_HighestSwitchNumber);
		World.m_GameTick = WORLD_START_TICK;
		auto &Config = World.m_WorldConfig;
		Config.m_IsDDRace = Mode != 2 && Mode != 3;
		Config.m_IsVanilla = Mode == 2;
		Config.m_IsFNG = Mode == 3;
		Config.m_InfiniteAmmo = Mode != 4;
		Config.m_PredictTiles = Mode != 2;
		Config.m_PredictFreeze = Mode != 2;
		Config.m_PredictWeapons = true;
		Config.m_PredictDDRace = Config.m_IsDDRace;
		Config.m_IsSolo = Mode == 4;
		Config.m_UseTuneZones = Mode != 5;
		Config.m_BugDDRaceInput = false;
		Config.m_NoWeakHookAndBounce = Mode == 1;
		Config.m_PredictEvents = false;
		Config.m_OldLaser = Mode == 1;
		if(Mode == 1)
			World.m_Teams.Team(aIds[2], 1);

		const vec2 Spot = vec2((Index % Width) * 32 + 16, (Index / Width) * 32 + 16);
		for(int i = 0; i < WEAPON_CHARS; i++)
		{
			CNetObj_Character Obj = {};
			Obj.m_X = (int)Spot.x + i * 24 - 36;
			Obj.m_Y = (int)Spot.y;
			Obj.m_HookX = Obj.m_X;
			Obj.m_HookY = Obj.m_Y;
			Obj.m_HookedPlayer = -1;
			Obj.m_Angle = i * 100;
			Obj.m_Weapon = (i + Run) % 5;
			Obj.m_AmmoCount = Mode == 4 ? i : 10;
			Obj.m_AttackTick = WORLD_START_TICK - 30;
			CNetObj_DDNetCharacter Ext = {};
			Ext.m_Flags = CHARACTERFLAG_WEAPON_HAMMER | CHARACTERFLAG_WEAPON_GUN | CHARACTERFLAG_WEAPON_SHOTGUN |
				      CHARACTERFLAG_WEAPON_GRENADE | CHARACTERFLAG_WEAPON_LASER;
			Ext.m_Jumps = 2;
			Ext.m_JumpedTotal = -1;
			Ext.m_NinjaActivationTick = -1;
			Ext.m_FreezeStart = -1;
			Ext.m_StrongWeakId = i;
			Ext.m_TuneZoneOverride = TuneZone::OVERRIDE_NONE;
			if(Mode == 0 && i == 3)
			{
				Ext.m_Flags |= CHARACTERFLAG_WEAPON_NINJA;
				Ext.m_NinjaActivationTick = WORLD_START_TICK - 10;
				Obj.m_Weapon = WEAPON_NINJA;
			}
			if(Mode == 0 && i == 2)
				Ext.m_Flags |= CHARACTERFLAG_JETPACK;
			if(Mode == 1 && i == 3)
				Ext.m_Flags |= CHARACTERFLAG_SUPER;
			if(Mode == 4 && i == 3)
				Ext.m_Flags |= CHARACTERFLAG_HAMMER_HIT_DISABLED | CHARACTERFLAG_SHOTGUN_HIT_DISABLED |
					       CHARACTERFLAG_GRENADE_HIT_DISABLED | CHARACTERFLAG_LASER_HIT_DISABLED;
			if(Mode == 3 && i == 1)
				Ext.m_FreezeEnd = WORLD_START_TICK + 40;
			if(Mode == 5 && i == 2)
				Ext.m_FreezeEnd = -1;
			CCharacter *pChar = new CCharacter(&World, aIds[i], &Obj, &Ext);
			World.InsertEntity(pChar);
			pChar->m_GameTeam = i % 2;
			if(i == 0)
				pChar->Read(&Obj, &Ext, true); // the local player
			fprintf(f, "char %d %d", aIds[i], i % 2);
			for(int k = 0; k < (int)(sizeof(Obj) / sizeof(int)); k++)
				fprintf(f, " %d", ((const int *)&Obj)[k]);
			fprintf(f, " |");
			for(int k = 0; k < (int)(sizeof(Ext) / sizeof(int)); k++)
				fprintf(f, " %d", ((const int *)&Ext)[k]);
			fprintf(f, "\n");
		}

		// entities from snapshots: pickups (appended), draggers and turret projectiles
		for(int k = 0; k < 8; k++)
		{
			CPickupData Data;
			Data.m_Pos = Spot + vec2((int)(Rand() % 161) - 80, (int)(Rand() % 81) - 60);
			Data.m_Type = aPickupTypes[(Run + k) % 9];
			Data.m_Subtype = (Run + k) % 6;
			Data.m_SwitchNumber = k % 3 == 0 ? 1 : 0;
			Data.m_Flags = 0;
			fprintf(f, "pickup %d %08x %08x %d %d %d\n", 100 + k, FloatBits(Data.m_Pos.x), FloatBits(Data.m_Pos.y), Data.m_Type, Data.m_Subtype, Data.m_SwitchNumber);
			World.InsertEntity(new CPickup(&World, 100 + k, &Data), true);
		}
		for(int k = 0; k < 2; k++)
		{
			CLaserData Data;
			Data.m_From = Spot + vec2((int)(Rand() % 401) - 200, (int)(Rand() % 201) - 150);
			Data.m_To = Data.m_From;
			Data.m_StartTick = WORLD_START_TICK;
			Data.m_ExtraInfo = true;
			Data.m_Owner = -1;
			Data.m_Type = LASERTYPE_DRAGGER;
			Data.m_SwitchNumber = k;
			Data.m_Subtype = (Run + k * 3) % 8 - 1;
			Data.m_TuneZone = 0;
			Data.m_Predict = true;
			fprintf(f, "dragger %d %08x %08x %d %d\n", 120 + k, FloatBits(Data.m_From.x), FloatBits(Data.m_From.y), Data.m_Subtype, Data.m_SwitchNumber);
			World.InsertEntity(new CDragger(&World, 120 + k, &Data));
		}
		for(int k = 0; k < 3; k++)
		{
			const int aTypes[] = {WEAPON_GUN, WEAPON_GRENADE, WEAPON_SHOTGUN};
			CProjectileData Data;
			Data.m_StartPos = Spot + vec2((int)(Rand() % 301) - 150, (int)(Rand() % 161) - 120);
			Data.m_StartVel = direction((Rand() % 360) * (pi / 180.0f)) * (k == 2 ? 0.7f : 1.0f);
			if(k == 0) // the owner-less turret shot aims at the tees
				Data.m_StartVel = normalize(Spot - Data.m_StartPos);
			Data.m_Type = aTypes[(Run + k) % 3];
			Data.m_StartTick = WORLD_START_TICK - (int)(Rand() % 5);
			Data.m_ExtraInfo = k != 1;
			Data.m_Owner = k == 0 ? -1 : aIds[k];
			Data.m_Explosive = (Run / 2) % 2 == 0 || k == 0;
			Data.m_Bouncing = Run % 3;
			Data.m_Freeze = k == 2 && Run % 2 == 1;
			Data.m_TuneZone = 0;
			Data.m_SwitchNumber = Run % 4 == 1 ? 1 : 0;
			fprintf(f, "proj %d %08x %08x %08x %08x %d %d %d %d %d %d %d %d\n", 130 + k, FloatBits(Data.m_StartPos.x),
				FloatBits(Data.m_StartPos.y), FloatBits(Data.m_StartVel.x), FloatBits(Data.m_StartVel.y), Data.m_Type,
				Data.m_StartTick, Data.m_ExtraInfo, Data.m_Owner, Data.m_Explosive, Data.m_Bouncing, Data.m_Freeze,
				Data.m_SwitchNumber);
			World.InsertEntity(new CProjectile(&World, 130 + k, &Data));
		}

		CNetObj_PlayerInput aInputs[MAX_CLIENTS] = {};
		int aHold[WEAPON_CHARS][4] = {};
		int aTotals[5] = {};
		for(int Tick = 1; Tick <= WEAPON_TICKS; Tick++)
		{
			if(Rand() % 25 == 0)
			{
				CLaserData Data;
				Data.m_From = Spot + vec2((int)(Rand() % 401) - 200, (int)(Rand() % 201) - 150);
				Data.m_To = Data.m_From;
				Data.m_StartTick = World.GameTick();
				Data.m_ExtraInfo = true;
				Data.m_Owner = Rand() % 5 == 0 ? 99 : aIds[Rand() % WEAPON_CHARS];
				Data.m_Type = LASERTYPE_PLASMA;
				Data.m_SwitchNumber = 0;
				Data.m_Subtype = (int)(Rand() % 5) - 1;
				Data.m_TuneZone = 0;
				Data.m_Predict = true;
				fprintf(f, "plasma %d %08x %08x %d %d\n", 140 + Tick, FloatBits(Data.m_From.x), FloatBits(Data.m_From.y), Data.m_Owner, Data.m_Subtype);
				World.InsertEntity(new CPlasma(&World, 140 + Tick, &Data));
			}
			for(int i = 0; i < WEAPON_CHARS; i++)
			{
				CNetObj_PlayerInput &In = aInputs[aIds[i]];
				if(--aHold[i][0] <= 0)
				{
					In.m_Direction = (int)(Rand() % 3) - 1;
					aHold[i][0] = 1 + Rand() % 30;
				}
				if(--aHold[i][1] <= 0)
				{
					In.m_Hook = Rand() % 4 == 0;
					aHold[i][1] = 1 + Rand() % 40;
					// aim mostly at the others
					const float Angle = (Rand() % 360) * (pi / 180.0f);
					const int Length = Rand() % 300;
					In.m_TargetX = Rand() % 2 ? (int)(Rand() % 201) - 100 : (int)(std::cos(Angle) * Length);
					In.m_TargetY = (int)(std::sin(Angle) * Length);
				}
				if(--aHold[i][2] <= 0)
				{
					In.m_Jump = Rand() % 5 == 0;
					aHold[i][2] = 1 + Rand() % 15;
				}
				if(--aHold[i][3] <= 0)
				{
					if((In.m_Fire & 1) != (int)(Rand() % 2))
						In.m_Fire = (In.m_Fire + 1) & INPUT_STATE_MASK;
					aHold[i][3] = 1 + Rand() % 8;
				}
				const int Extra = Rand() % 100;
				In.m_NextWeapon += Extra == 0 ? 2 : 0;
				In.m_PrevWeapon += Extra == 1 ? 2 : 0;
				In.m_WantedWeapon = Extra >= 2 && Extra < 8 ? Extra - 1 : 0;
				In.m_PlayerFlags = Extra >= 98 ? PLAYERFLAG_CHATTING : PLAYERFLAG_PLAYING;
			}
			for(int Id = 0; Id < MAX_CLIENTS; Id++)
			{
				if(CCharacter *pChar = World.GetCharacterById(Id))
				{
					const CNetObj_PlayerInput &In = aInputs[Id];
					fprintf(f, "%d %d %d %d %d %d %d %d %d %d ", In.m_Direction, In.m_Jump, In.m_Hook, In.m_TargetX, In.m_TargetY,
						In.m_Fire, In.m_NextWeapon, In.m_PrevWeapon, In.m_WantedWeapon, In.m_PlayerFlags);
					pChar->OnDirectInput(&In);
				}
			}
			World.m_GameTick = WORLD_START_TICK + Tick;
			for(int Id = 0; Id < MAX_CLIENTS; Id++)
				if(CCharacter *pChar = World.GetCharacterById(Id))
					pChar->OnPredictedInput(&aInputs[Id]);
			World.Tick();

			fprintf(f, "|");
			for(int Id = 0; Id < MAX_CLIENTS; Id++)
			{
				CCharacter *pChar = World.GetCharacterById(Id);
				if(!pChar)
					continue;
				SQueries Q;
				WeaponCharState(Q, pChar);
				fprintf(f, " %08x %08x %08x", FloatBits(pChar->GetPos().x), FloatBits(pChar->GetPos().y), Q.Crc());
			}
			SQueries E;
			WeaponEntityState(E, World);
			const int aCounts[5] = {EntityCount(World, CGameWorld::ENTTYPE_PROJECTILE), EntityCount(World, CGameWorld::ENTTYPE_LASER),
				EntityCount(World, CGameWorld::ENTTYPE_DRAGGER), EntityCount(World, CGameWorld::ENTTYPE_PLASMA),
				EntityCount(World, CGameWorld::ENTTYPE_PICKUP)};
			for(int k = 0; k < 5; k++)
				aTotals[k] += aCounts[k];
			fprintf(f, " | %d %d %d %d %d %08x", aCounts[0], aCounts[1], aCounts[2], aCounts[3], aCounts[4], E.Crc());
			if(Verbose)
			{
				fprintf(f, " [");
				for(int v : E.m_vValues)
					fprintf(f, " %x", (unsigned)v);
				fprintf(f, " ]");
			}
			fprintf(f, "\n");
		}
		fprintf(f, "totals %d %d %d %d %d\n", aTotals[0], aTotals[1], aTotals[2], aTotals[3], aTotals[4]);
	}
	fclose(f);
	return 0;
}

// A snapshot entity item for the netents mode.
struct SNetItem
{
	int m_Type;
	int m_Id;
	std::vector<int> m_vData;
	bool m_HasEx = false;
	CNetObj_EntityEx m_Ex = {};
};

// The C++ and Rust ids of extended object types differ, so the data names them.
static const char *NetObjTypeName(int Type)
{
	switch(Type)
	{
	case NETOBJTYPE_PROJECTILE: return "projectile";
	case NETOBJTYPE_DDRACEPROJECTILE: return "ddraceprojectile";
	case NETOBJTYPE_DDNETPROJECTILE: return "ddnetprojectile";
	case NETOBJTYPE_PICKUP: return "pickup";
	case NETOBJTYPE_DDNETPICKUP: return "ddnetpickup";
	case NETOBJTYPE_LASER: return "laser";
	case NETOBJTYPE_DDNETLASER: return "ddnetlaser";
	}
	return "?";
}

static constexpr int NETENT_RUNS = 4;
static constexpr int NETENT_SNAPS = 60;

// Where FindMatch found an item: list (ENTTYPE) and index in it, or -1.
static void PrintMatch(FILE *f, CGameWorld &World, CEntity *pFound)
{
	if(!pFound)
	{
		fprintf(f, " -");
		return;
	}
	for(int Type = 0; Type < CGameWorld::NUM_ENTTYPES; Type++)
	{
		int Index = 0;
		for(CEntity *pEnt = World.FindFirst(Type); pEnt; pEnt = pEnt->TypeNext(), Index++)
			if(pEnt == pFound)
			{
				fprintf(f, " %d:%d", Type, Index);
				return;
			}
	}
	fprintf(f, " ?");
}

static void DumpNetWorld(FILE *f, const char *pTag, CGameWorld &World)
{
	fprintf(f, "%s", pTag);
	for(int Id = 0; Id < MAX_CLIENTS; Id++)
		if(CCharacter *pChar = World.GetCharacterById(Id))
		{
			SQueries Q;
			WeaponCharState(Q, pChar);
			fprintf(f, " %d:%08x", Id, Q.Crc());
		}
	SQueries E;
	WeaponEntityState(E, World);
	fprintf(f, " | %d %d %d %d %d %d %08x\n", EntityCount(World, CGameWorld::ENTTYPE_PROJECTILE), EntityCount(World, CGameWorld::ENTTYPE_LASER),
		EntityCount(World, CGameWorld::ENTTYPE_DOOR), EntityCount(World, CGameWorld::ENTTYPE_DRAGGER),
		EntityCount(World, CGameWorld::ENTTYPE_PLASMA), EntityCount(World, CGameWorld::ENTTYPE_PICKUP), E.Crc());
}

// For rs_parity_antiping.cpp: CoreState, and every character and entity of a world.
void CoreStateInts(std::vector<int> &vValues, const CCharacterCore &Core)
{
	SQueries Q;
	CoreState(Q, Core);
	vValues.insert(vValues.end(), Q.m_vValues.begin(), Q.m_vValues.end());
}

void CharStateInts(std::vector<int> &vValues, CCharacter *pChar)
{
	SQueries Q;
	WeaponCharState(Q, pChar);
	vValues.insert(vValues.end(), Q.m_vValues.begin(), Q.m_vValues.end());
}

void NetWorldInts(std::vector<int> &vValues, CGameWorld &World)
{
	for(int Id = 0; Id < MAX_CLIENTS; Id++)
		if(CCharacter *pChar = World.GetCharacterById(Id))
		{
			SQueries Q;
			WeaponCharState(Q, pChar);
			vValues.push_back(Id);
			vValues.push_back((int)Q.Crc());
		}
	for(int Type : {CGameWorld::ENTTYPE_PROJECTILE, CGameWorld::ENTTYPE_LASER, CGameWorld::ENTTYPE_DOOR, CGameWorld::ENTTYPE_DRAGGER,
		    CGameWorld::ENTTYPE_PLASMA, CGameWorld::ENTTYPE_PICKUP})
		vValues.push_back(EntityCount(World, Type));
	SQueries E;
	WeaponEntityState(E, World);
	vValues.push_back((int)E.Crc());
}

// Snapshot entities into the prediction world (M8.4): characters fire for real, and each
// snapshot sends server versions of their projectiles and lasers (matching, moved,
// stopped early), server-only projectiles (all three objects, owner guessing), pickups,
// draggers, plasma, doors and odd objects that come, change and go. After each
// NetObjEnd: every character and entity, FindMatch for every item, and a CopyWorld that
// ticks once.
static int NetEntsMode(const char *pMapFile, const char *pOutFile)
{
	std::unique_ptr<IStorage> pStorage = CreateLocalStorage();
	std::unique_ptr<IMap> pMap = CreateMap();
	CLayers Layers;
	FILE *f = fopen(pOutFile, "wb");
	if(!pStorage || !f || !LoadMapAndLayers(pStorage.get(), pMapFile, pMap.get(), &Layers))
		return -1;
	MakeMovers(pMap.get(), &Layers);
	CCollision Collision;
	Collision.Init(&Layers);
	const int Width = Collision.GetWidth(), Height = Collision.GetHeight();

	g_Config.m_SvFreezeDelay = 3;
	g_Config.m_ClAntiPingPlayers = 1;
	g_Config.m_SvDeepfly = 1;
	g_Config.m_SvHit = 1;
	g_Config.m_SvDraggerRange = 700;
	g_Config.m_SvOldTeleportHook = 0;

	static CTuningParams s_aTuning[TuneZone::NUM];
	for(int z = 0; z < TuneZone::NUM; z++)
	{
		s_aTuning[z] = CTuningParams();
		WeaponZoneTuning(&s_aTuning[z], z);
	}
	const int aIds[WEAPON_CHARS] = {3, 0, 6, 1};
	const int aSorted[WEAPON_CHARS] = {0, 1, 3, 6};
	const auto Free = [&](int x, int y) {
		const int t = Collision.GetTileIndex(std::clamp(y, 0, Height - 1) * Width + std::clamp(x, 0, Width - 1));
		return t != TILE_SOLID && t != TILE_NOHOOK;
	};
	for(int Run = 0; Run < NETENT_RUNS; Run++)
	{
		int Index = 0;
		for(int Try = 0; Try < 300; Try++)
		{
			Index = Rand() % (Width * Height);
			const int x = Index % Width, y = Index / Width;
			if(Free(x - 2, y) && Free(x - 1, y) && Free(x, y) && Free(x + 1, y) && Free(x + 2, y))
				break;
		}
		const int Mode = Run % 4;
		fprintf(f, "run %d %d %d\n", Run, Index, Mode);

		CGameWorld World;
		World.Init(&Collision, s_aTuning, nullptr);
		World.m_Core.InitSwitchers(Collision.m_HighestSwitchNumber);
		World.m_GameTick = WORLD_START_TICK;
		auto &Config = World.m_WorldConfig;
		Config.m_IsDDRace = Mode < 2;
		Config.m_IsVanilla = Mode == 2;
		Config.m_IsFNG = Mode == 3;
		Config.m_InfiniteAmmo = true;
		Config.m_PredictTiles = Mode != 2;
		Config.m_PredictFreeze = 1;
		Config.m_PredictWeapons = true;
		Config.m_PredictDDRace = Config.m_IsDDRace;
		Config.m_IsSolo = false;
		Config.m_UseTuneZones = Mode != 1;
		Config.m_BugDDRaceInput = false;
		Config.m_NoWeakHookAndBounce = false;
		Config.m_PredictEvents = false;
		Config.m_OldLaser = false;
		CTeamsCore Teams;
		if(Mode == 1)
			Teams.Team(aIds[2], 1);
		World.m_Teams = Teams;
		const int Local = aIds[0];

		const vec2 Spot = vec2((Index % Width) * 32 + 16, (Index / Width) * 32 + 16);
		CNetObj_DDNetCharacter aExt[MAX_CLIENTS] = {};
		CNetObj_Character aLastObj[MAX_CLIENTS] = {};
		for(int i = 0; i < WEAPON_CHARS; i++)
		{
			CNetObj_Character Obj = {};
			Obj.m_X = (int)Spot.x + i * 24 - 36;
			Obj.m_Y = (int)Spot.y;
			Obj.m_HookX = Obj.m_X;
			Obj.m_HookY = Obj.m_Y;
			Obj.m_HookedPlayer = -1;
			Obj.m_Weapon = 1 + (i + Run) % 4;
			Obj.m_AmmoCount = 10;
			Obj.m_AttackTick = WORLD_START_TICK - 30;
			CNetObj_DDNetCharacter &Ext = aExt[aIds[i]];
			Ext.m_Flags = CHARACTERFLAG_WEAPON_HAMMER | CHARACTERFLAG_WEAPON_GUN | CHARACTERFLAG_WEAPON_SHOTGUN |
				      CHARACTERFLAG_WEAPON_GRENADE | CHARACTERFLAG_WEAPON_LASER;
			Ext.m_Jumps = 2;
			Ext.m_JumpedTotal = -1;
			Ext.m_NinjaActivationTick = -1;
			Ext.m_FreezeStart = -1;
			Ext.m_StrongWeakId = i;
			Ext.m_TuneZoneOverride = TuneZone::OVERRIDE_NONE;
			CCharacter *pChar = new CCharacter(&World, aIds[i], &Obj, &Ext);
			World.InsertEntity(pChar);
			pChar->m_GameTeam = i % 2;
			if(i == 0)
				pChar->Read(&Obj, &Ext, true);
			aLastObj[aIds[i]] = Obj;
			fprintf(f, "char %d %d", aIds[i], i % 2);
			for(int k = 0; k < (int)(sizeof(Obj) / sizeof(int)); k++)
				fprintf(f, " %d", ((const int *)&Obj)[k]);
			fprintf(f, " |");
			for(int k = 0; k < (int)(sizeof(Ext) / sizeof(int)); k++)
				fprintf(f, " %d", ((const int *)&Ext)[k]);
			fprintf(f, "\n");
		}

		CNetObj_PlayerInput aInputs[MAX_CLIENTS] = {};
		int aHold[WEAPON_CHARS][4] = {};
		const auto TickWorld = [&](CGameWorld &W, bool Print) {
			if(Print)
			{
				for(int i = 0; i < WEAPON_CHARS; i++)
				{
					CNetObj_PlayerInput &In = aInputs[aIds[i]];
					if(--aHold[i][0] <= 0)
					{
						In.m_Direction = (int)(Rand() % 3) - 1;
						aHold[i][0] = 1 + Rand() % 30;
					}
					if(--aHold[i][1] <= 0)
					{
						In.m_Hook = Rand() % 5 == 0;
						aHold[i][1] = 1 + Rand() % 40;
						const float Angle = (Rand() % 360) * (pi / 180.0f);
						const int Length = 10 + Rand() % 300;
						In.m_TargetX = (int)(std::cos(Angle) * Length);
						In.m_TargetY = (int)(std::sin(Angle) * Length);
					}
					if(--aHold[i][2] <= 0)
					{
						In.m_Jump = Rand() % 5 == 0;
						aHold[i][2] = 1 + Rand() % 15;
					}
					if(--aHold[i][3] <= 0)
					{
						if((In.m_Fire & 1) != (int)(Rand() % 2))
							In.m_Fire = (In.m_Fire + 1) & INPUT_STATE_MASK;
						aHold[i][3] = 1 + Rand() % 6;
					}
					const int Extra = Rand() % 100;
					In.m_WantedWeapon = Extra < 6 ? 2 + Extra % 4 : 0;
				}
				fprintf(f, "t");
				for(int Id : aSorted)
				{
					const CNetObj_PlayerInput &In = aInputs[Id];
					fprintf(f, " %d %d %d %d %d %d %d", In.m_Direction, In.m_Jump, In.m_Hook, In.m_TargetX, In.m_TargetY, In.m_Fire, In.m_WantedWeapon);
				}
				fprintf(f, "\n");
			}
			for(int Id : aSorted)
				if(CCharacter *pChar = W.GetCharacterById(Id))
					pChar->OnDirectInput(&aInputs[Id]);
			W.m_GameTick++;
			for(int Id : aSorted)
				if(CCharacter *pChar = W.GetCharacterById(Id))
					pChar->OnPredictedInput(&aInputs[Id]);
			W.Tick();
		};

		// server entities that stay over several snapshots, by id
		std::map<int, SNetItem> Server;
		int NextId = 1000;
		const auto Near = [&](int Range) { return Spot + vec2((int)(Rand() % (2 * Range + 1)) - Range, (int)(Rand() % (Range + 1)) - Range * 3 / 4); };
		for(int Snap = 0; Snap < NETENT_SNAPS; Snap++)
		{
			const int Ticks = 1 + Rand() % 3;
			for(int t = 0; t < Ticks; t++)
				TickWorld(World, true);
			const int GameTick = World.GameTick();

			std::vector<SNetItem> vItems;
			// the server's versions of locally fired projectiles and lasers
			for(CEntity *pEnt = World.FindFirst(CGameWorld::ENTTYPE_PROJECTILE); pEnt; pEnt = pEnt->TypeNext())
			{
				CProjectile *p = static_cast<CProjectile *>(pEnt);
				if(p->GetId() != -1 || Rand() % 10 < 3)
					continue;
				const vec2 Dir = p->*Get(SProjDir());
				const int aOffsets[] = {0, 0, 0, 2, 3, 40}; // near the 2 px match limit, or too far
				const int Off = aOffsets[Rand() % 6];
				vItems.push_back({NETOBJTYPE_PROJECTILE, NextId++, {(int)p->m_Pos.x + Off, (int)p->m_Pos.y, (int)(Dir.x * 100.0f), (int)(Dir.y * 100.0f), p->*Get(SProjType()), p->*Get(SProjStart())}});
			}
			for(CEntity *pEnt = World.FindFirst(CGameWorld::ENTTYPE_LASER); pEnt; pEnt = pEnt->TypeNext())
			{
				CLaser *p = static_cast<CLaser *>(pEnt);
				if(p->GetId() != -1 || Rand() % 10 < 3)
					continue;
				const vec2 From = p->*Get(SLaserFrom());
				vec2 To = p->m_Pos;
				const int Cut = Rand() % 4; // stopped earlier than predicted, around the 2 px limit
				if(Cut == 1)
					To = From + (To - From) * 0.5f;
				else if(Cut >= 2 && length(To - From) > 4.0f)
					To = To - normalize(To - From) * (Cut == 2 ? 1.5f : 3.0f);
				vItems.push_back({NETOBJTYPE_DDNETLASER, NextId++, {(int)To.x, (int)To.y, (int)From.x, (int)From.y, p->*Get(SLaserEval()), p->*Get(SLaserOwner()), p->*Get(SLaserType()) == WEAPON_SHOTGUN ? LASERTYPE_SHOTGUN : LASERTYPE_RIFLE, -1, -1, 0}});
			}
			// new server entities
			const int New = Rand() % 4;
			for(int k = 0; k < New; k++)
			{
				SNetItem It;
				It.m_Id = NextId++;
				const vec2 Pos = Near(150);
				const int Owner = Rand() % 4 == 0 ? -1 : (Rand() % 6 == 0 ? 99 : aIds[Rand() % WEAPON_CHARS]);
				const int Kind = Rand() % 11;
				const int Type = (int)(Rand() % 3) + 1; // gun, shotgun, grenade
				const int Start = GameTick - (int)(Rand() % 7);
				if(Kind == 0)
				{
					It.m_Type = NETOBJTYPE_PROJECTILE;
					const vec2 Dir = direction((Rand() % 360) * (pi / 180.0f)) * (Rand() % 8 == 0 ? 1.5f : 1.0f);
					It.m_vData = {(int)Pos.x, (int)Pos.y, (int)(Dir.x * 100.0f), (int)(Dir.y * 100.0f), Type, Start};
				}
				else if(Kind == 1)
				{
					It.m_Type = NETOBJTYPE_DDRACEPROJECTILE;
					const int Data = (Owner & 255) | (Rand() % 4 == 0 ? LEGACYPROJECTILEFLAG_NO_OWNER : 0) | ((int)(Rand() % 4) << 10) |
							 (Rand() % 2 ? LEGACYPROJECTILEFLAG_EXPLOSIVE : 0) | (Rand() % 3 == 0 ? LEGACYPROJECTILEFLAG_FREEZE : 0);
					It.m_vData = {(int)(Pos.x * 100), (int)(Pos.y * 100), (int)(Rand() % 6283185), Data, Type, Start};
				}
				else if(Kind == 2)
				{
					It.m_Type = NETOBJTYPE_DDNETPROJECTILE;
					const vec2 Dir = direction((Rand() % 360) * (pi / 180.0f));
					const int Flags = (Rand() % 2 ? PROJECTILEFLAG_EXPLOSIVE : 0) | (Rand() % 3 == 0 ? PROJECTILEFLAG_FREEZE : 0) |
							  (Rand() % 3 == 0 ? PROJECTILEFLAG_BOUNCE_HORIZONTAL : 0) | (Rand() % 4 == 0 ? PROJECTILEFLAG_NORMALIZE_VEL : 0);
					const int DdOwner = Rand() % 2 ? -1 : Owner;
					It.m_vData = {(int)(Pos.x * 100), (int)(Pos.y * 100), DdOwner < 0 ? (int)(Dir.x * 1e6f) : (int)(Rand() % 21) - 10,
						DdOwner < 0 ? (int)(Dir.y * 1e6f) : (int)(Rand() % 21) - 10, Type, Start, DdOwner, (int)(Rand() % 3), 0, Flags};
				}
				else if(Kind <= 4)
				{
					It.m_Type = Kind == 3 ? NETOBJTYPE_PICKUP : NETOBJTYPE_DDNETPICKUP;
					const int PType = Rand() % 9, Sub = Rand() % 6;
					if(Kind == 3)
						It.m_vData = {(int)Pos.x, (int)Pos.y, PType, Sub};
					else
						It.m_vData = {(int)Pos.x, (int)Pos.y, PType, Sub, (int)(Rand() % 3), Rand() % 5 == 0 ? PICKUPFLAG_NO_PREDICT : 0};
				}
				else
				{
					It.m_Type = Kind == 5 ? NETOBJTYPE_LASER : NETOBJTYPE_DDNETLASER;
					const int aLaserTypes[] = {LASERTYPE_RIFLE, LASERTYPE_SHOTGUN, LASERTYPE_DOOR, LASERTYPE_DRAGGER, LASERTYPE_PLASMA, LASERTYPE_FREEZE};
					const int LType = aLaserTypes[Rand() % 6];
					const vec2 To = Pos + direction((Rand() % 360) * (pi / 180.0f)) * (float)(Rand() % 200);
					if(Kind == 5)
						It.m_vData = {(int)To.x, (int)To.y, (int)Pos.x, (int)Pos.y, Start};
					else
						It.m_vData = {(int)To.x, (int)To.y, (int)Pos.x, (int)Pos.y, Start, LType == LASERTYPE_PLASMA ? Owner : (Rand() % 3 ? -1 : Owner),
							LType, (int)(Rand() % 3) - 1, (int)(Rand() % 8) - 1, Rand() % 6 == 0 ? LASERFLAG_NO_PREDICT : 0};
				}
				if(Rand() % 4 == 0)
				{
					It.m_HasEx = true;
					It.m_Ex.m_SwitchNumber = Rand() % 3;
					It.m_Ex.m_Layer = Rand() % 2 ? LAYER_SWITCH : LAYER_GAME;
					It.m_Ex.m_EntityClass = Rand() % 2 ? ENTITYCLASS_PROJECTILE : ENTITYCLASS_LIGHT;
				}
				Server[It.m_Id] = It;
			}
			// server entities stay, move a little, change or leave
			for(auto It = Server.begin(); It != Server.end();)
			{
				const int Roll = Rand() % 20;
				if(Roll == 0)
				{
					It = Server.erase(It);
					continue;
				}
				SNetItem Item = It->second;
				if(Roll == 1 && !Item.m_vData.empty())
					Item.m_vData[0] += Rand() % 2 ? 1 : 5;
				if(Roll == 2 && Item.m_Type == NETOBJTYPE_DDNETLASER)
					Item.m_vData[8] = (int)(Rand() % 8) - 1; // dragger strength, plasma kind
				if(Roll == 3 && Item.m_Type == NETOBJTYPE_DDNETLASER)
					Item.m_vData[5] = aIds[Rand() % WEAPON_CHARS]; // dragger target, plasma owner
				It->second = Item;
				if(Roll != 7) // sometimes missing from one snapshot
					vItems.push_back(Item);
				++It;
			}
			std::stable_sort(vItems.begin(), vItems.end(), [](const SNetItem &a, const SNetItem &b) { return a.m_Id < b.m_Id; });
			for(const SNetItem &It : vItems)
			{
				fprintf(f, "item %s %d", NetObjTypeName(It.m_Type), It.m_Id);
				for(int v : It.m_vData)
					fprintf(f, " %d", v);
				if(It.m_HasEx)
					fprintf(f, " | %d %d %d", It.m_Ex.m_SwitchNumber, It.m_Ex.m_Layer, It.m_Ex.m_EntityClass);
				fprintf(f, "\n");
			}

			// the snapshot's characters: their predicted state; one leaves for a while
			World.NetObjBegin(Teams, Local);
			for(int i = 0; i < WEAPON_CHARS; i++)
			{
				const int Id = aIds[i];
				CCharacter *pChar = World.GetCharacterById(Id);
				if(i == 2 && Snap % 10 >= 7)
					continue;
				CNetObj_Character Obj = aLastObj[Id]; // a character that left comes back there
				if(pChar)
				{
					pChar->GetCore().Write((CNetObj_CharacterCore *)&Obj);
					Obj.m_Weapon = pChar->GetActiveWeapon();
					Obj.m_AttackTick = pChar->GetAttackTick();
				}
				Obj.m_Tick = GameTick;
				Obj.m_AmmoCount = 10;
				aLastObj[Id] = Obj;
				fprintf(f, "netchar %d", Id);
				for(int k = 0; k < (int)(sizeof(Obj) / sizeof(int)); k++)
					fprintf(f, " %d", ((const int *)&Obj)[k]);
				fprintf(f, "\n");
				World.NetCharAdd(Id, &Obj, &aExt[Id], i % 2, Id == Local);
			}
			for(SNetItem &It : vItems)
			{
				It.m_vData.resize(16, 0);
				World.NetObjAdd(It.m_Id, It.m_Type, It.m_vData.data(), It.m_HasEx ? &It.m_Ex : nullptr);
			}
			World.NetObjEnd();
			DumpNetWorld(f, "world", World);
			fprintf(f, "match");
			for(SNetItem &It : vItems)
				PrintMatch(f, World, World.FindMatch(It.m_Id, It.m_Type, It.m_vData.data()));
			fprintf(f, "\n");

			CGameWorld Copy;
			Copy.CopyWorld(&World);
			TickWorld(Copy, false);
			DumpNetWorld(f, "copy", Copy);
			World.m_pChild = nullptr; // the copy goes away
		}
	}
	fclose(f);
	return 0;
}

// One laser snapshot item: its type, id, ints and optional EntityEx.
struct SLaserItem
{
	int m_Type;
	int m_Id;
	std::vector<int> m_vData;
	bool m_HasEx;
	CNetObj_EntityEx m_Ex;
};

static constexpr int DOOR_SNAPS = 60;
static constexpr int DOOR_LASERS = 200;
static const int s_aDoorSides[8][2] = {{0, 1}, {1, 1}, {1, 0}, {1, -1}, {0, -1}, {-1, -1}, {-1, 0}, {-1, 1}};

// Every door tile of the game and switch layers also becomes a front-layer door, with the
// laser tiles around it copied to the front layer: unchanged for every second door, one
// length further for the others, so their layers configure different lengths.
static void MakeFrontDoors(IMap *pMap, CLayers *pLayers)
{
	if(!pLayers->FrontLayer())
		return;
	CTile *pFront = static_cast<CTile *>(pMap->GetData(pLayers->FrontLayer()->m_Front));
	CTile *pGame = static_cast<CTile *>(pMap->GetData(pLayers->GameLayer()->m_Data));
	CSwitchTile *pSwitch = pLayers->SwitchLayer() ? static_cast<CSwitchTile *>(pMap->GetData(pLayers->SwitchLayer()->m_Switch)) : nullptr;
	const int Width = pLayers->GameLayer()->m_Width, Height = pLayers->GameLayer()->m_Height;
	if(!pFront || !pGame)
		return;
	int Doors = 0;
	for(int i = 0; i < Width * Height; i++)
		for(int Layer = 0; Layer < 2; Layer++)
		{
			const auto Source = [&](int Index) -> int { return Layer == 0 ? pGame[Index].m_Index : (pSwitch ? pSwitch[Index].m_Type : 0); };
			if(Source(i) != ENTITY_OFFSET + ENTITY_DOOR)
				continue;
			Doors++;
			pFront[i].m_Index = ENTITY_OFFSET + ENTITY_DOOR;
			for(const auto &Side : s_aDoorSides)
			{
				const int x = i % Width + Side[0], y = i / Width + Side[1];
				if(x < 0 || x >= Width || y < 0 || y >= Height)
					continue;
				const int Laser = Source(y * Width + x) - ENTITY_OFFSET;
				if(Laser < ENTITY_LASER_SHORT || Laser > ENTITY_LASER_LONG)
					continue;
				pFront[y * Width + x].m_Index = ENTITY_OFFSET + (Doors % 2 ? Laser : ENTITY_LASER_SHORT + (Laser - ENTITY_LASER_SHORT + 1) % 3);
			}
		}
}

static int DoorsMode(const char *pMapFile, const char *pOutFile)
{
	std::unique_ptr<IStorage> pStorage = CreateLocalStorage();
	std::unique_ptr<IMap> pMap = CreateMap();
	CLayers Layers;
	FILE *f = fopen(pOutFile, "wb");
	if(!pStorage || !f || !LoadMapAndLayers(pStorage.get(), pMapFile, pMap.get(), &Layers))
		return -1;
	MakeFrontDoors(pMap.get(), &Layers);
	CCollision Collision;
	Collision.Init(&Layers);
	const int Width = Collision.GetWidth(), Height = Collision.GetHeight();

	// the doors the server creates (IGameController::OnEntity and server CDoor), snapped
	// as for a DDNet client (CDoor::Snap, SnapLaserObject)
	std::vector<SLaserItem> vPool;
	int NextId = 10;
	const auto AddDoor = [&](vec2 Pos, int Side, int Length, int Number) {
		const float Rotation = pi / 4 * Side;
		const vec2 Direction = vec2(std::sin(Rotation), std::cos(Rotation));
		vec2 To;
		Collision.IntersectNoLaser(Pos, Pos + normalize(Direction) * (float)Length, &To, nullptr);
		vPool.push_back({NETOBJTYPE_DDNETLASER, NextId++,
			{round_to_int(Pos.x), round_to_int(Pos.y), round_to_int(To.x), round_to_int(To.y), -1, -1, LASERTYPE_DOOR, Number, 0, 0}, false, {}});
	};
	for(int y = 0; y < Height; y++)
		for(int x = 0; x < Width; x++)
			for(const int Layer : {LAYER_GAME, LAYER_FRONT, LAYER_SWITCH})
			{
				if((Layer == LAYER_FRONT && !Collision.FrontLayer()) || (Layer == LAYER_SWITCH && !Collision.SwitchLayer()))
					continue;
				if(Collision.Entity(x, y, Layer) != ENTITY_DOOR)
					continue;
				const int Number = Layer == LAYER_SWITCH ? Collision.SwitchLayer()[y * Width + x].m_Number : 0;
				for(int i = 0; i < 8; i++)
				{
					const int Side = Collision.Entity(x + s_aDoorSides[i][0], y + s_aDoorSides[i][1], Layer);
					if(Side >= ENTITY_LASER_SHORT && Side <= ENTITY_LASER_LONG)
						AddDoor(vec2(x * 32.0f + 16.0f, y * 32.0f + 16.0f), i, 32 * 3 + 32 * (Side - ENTITY_LASER_SHORT) * 3, Number);
				}
			}
	const int ServerDoors = (int)vPool.size();
	// more doors on the same tiles, in other directions and with other switch numbers
	for(int i = 0; i < ServerDoors; i++)
		if(Rand() % 2)
		{
			const int aNumbers[] = {0, 0, 1, 2, 7};
			const vec2 Pos(vPool[i].m_vData[0], vPool[i].m_vData[1]);
			const int Side = Rand() % 8;
			const int Length = 32 * (1 + Rand() % 9);
			AddDoor(Pos, Side, Length, aNumbers[Rand() % 5]);
		}
	// tiles a door can start on that the client treats specially
	// (game solid, game death/nolaser, front death/nolaser)
	std::vector<std::vector<int>> vvSpecial;
	for(const auto &IsSpecial : std::vector<std::function<bool(int)>>{
		    [&](int i) { return Collision.GetTileIndex(i) == TILE_SOLID; },
		    [&](int i) { return Collision.GetTileIndex(i) == TILE_DEATH || Collision.GetTileIndex(i) == TILE_NOLASER; },
		    [&](int i) { return Collision.GetFrontTileIndex(i) == TILE_DEATH || Collision.GetFrontTileIndex(i) == TILE_NOLASER; }})
	{
		std::vector<int> vTiles;
		for(int i = 0; i < Width * Height; i++)
			if(IsSpecial(i))
				vTiles.push_back(i);
		if(!vTiles.empty())
			vvSpecial.push_back(vTiles);
	}
	// lasers a server could send: other types, vanilla lasers with EntityEx, other
	// owners, no-predict flags, odd switch numbers, doors off or across the map
	for(int i = 0; i < DOOR_LASERS; i++)
	{
		const int Range = (Width + Height) * 32;
		const auto Coord = [&](int Size) {
			switch(Rand() % 8)
			{
			case 0: return (int)(Rand() % 2000000) - 1000000;
			case 1: return (int)(Rand() % (Range * 2)) - Range / 2;
			default: return (int)(Rand() % (Size * 32));
			}
		};
		int X = Coord(Width);
		int Y = Coord(Height);
		if(!vvSpecial.empty() && Rand() % 3 == 0)
		{
			const std::vector<int> &vSpecial = vvSpecial[Rand() % vvSpecial.size()];
			const int Index = vSpecial[Rand() % vSpecial.size()];
			X = Index % Width * 32 + 16;
			Y = Index / Width * 32 + 16;
		}
		int FromX = X, FromY = Y;
		switch(Rand() % 4)
		{
		case 0: break; // zero length
		case 1:
			FromX = Coord(Width);
			FromY = Coord(Height);
			break;
		default:
		{
			// along one of the eight door sides, sometimes off by a bit
			const int Side = Rand() % 8;
			const float Len = 32.0f * (1 + Rand() % 12);
			FromX = X + round_to_int(std::sin(pi / 4 * Side) * Len) + (Rand() % 4 == 0 ? (int)(Rand() % 21) - 10 : 0);
			FromY = Y + round_to_int(std::cos(pi / 4 * Side) * Len);
		}
		}
		const int aNumbers[] = {-1, 0, 0, 1, 2, Collision.m_HighestSwitchNumber, Collision.m_HighestSwitchNumber + 1, 300};
		const int Number = aNumbers[Rand() % 8];
		const int aTypes[] = {LASERTYPE_DOOR, LASERTYPE_DOOR, LASERTYPE_DOOR, LASERTYPE_RIFLE, LASERTYPE_DRAGGER, LASERTYPE_FREEZE};
		const int aOwners[] = {-1, -1, -1, 0, 1, 2, 3};
		const int Owner = aOwners[Rand() % 7];
		SLaserItem Item;
		Item.m_Id = NextId++;
		Item.m_HasEx = Rand() % 2 == 0;
		Item.m_Ex.m_SwitchNumber = aNumbers[Rand() % 8];
		Item.m_Ex.m_Layer = 0;
		const int aClasses[] = {ENTITYCLASS_DOOR, ENTITYCLASS_DOOR, ENTITYCLASS_LIGHT, ENTITYCLASS_GUN_NORMAL, ENTITYCLASS_DRAGGER_WEAK, ENTITYCLASS_PICKUP};
		Item.m_Ex.m_EntityClass = aClasses[Rand() % 6];
		if(Rand() % 3 == 0)
		{
			Item.m_Type = NETOBJTYPE_LASER;
			Item.m_vData = {X, Y, FromX, FromY, -1};
		}
		else
		{
			const int Type = aTypes[Rand() % 6];
			const int Flags = Rand() % 6 == 0 ? LASERFLAG_NO_PREDICT : 0;
			Item.m_Type = NETOBJTYPE_DDNETLASER;
			Item.m_vData = {X, Y, FromX, FromY, -1, Owner, Type, Number, 0, Flags};
		}
		vPool.push_back(Item);
	}
	fprintf(f, "pool %d %d\n", ServerDoors, (int)vPool.size());

	g_Config.m_SvFreezeDelay = 3;
	CGameWorld World;
	World.Init(&Collision, nullptr, nullptr);
	CTeamsCore Teams;
	Teams.Team(2, 1); // owner 2 is not in the local player's team
	for(int Snap = 0; Snap < DOOR_SNAPS; Snap++)
	{
		World.m_WorldConfig.m_PredictTiles = Snap % 10 != 9;
		// most items stay, some leave and come back, some move under the same id
		std::vector<SLaserItem> vSnap;
		for(SLaserItem &Item : vPool)
		{
			if(Snap > 0 && Rand() % 10 == 0)
				Item.m_vData[2] += 32 * ((int)(Rand() % 3) - 1);
			if(Snap == 0 || Rand() % 5 != 0)
				vSnap.push_back(Item);
		}
		fprintf(f, "snap %d %d %d\n", Snap, World.m_WorldConfig.m_PredictTiles, (int)vSnap.size());
		World.NetObjBegin(Teams, 0);
		for(const SLaserItem &Item : vSnap) // sorted by id, like SnapCollectEntities
		{
			fprintf(f, "%s %d", Item.m_Type == NETOBJTYPE_LASER ? "laser" : "ddnetlaser", Item.m_Id); // ex type numbers are not stable
			for(int v : Item.m_vData)
				fprintf(f, " %d", v);
			if(Item.m_HasEx)
				fprintf(f, " ex %d %d %d", Item.m_Ex.m_SwitchNumber, Item.m_Ex.m_Layer, Item.m_Ex.m_EntityClass);
			fprintf(f, "\n");
			World.NetObjAdd(Item.m_Id, Item.m_Type, Item.m_vData.data(), Item.m_HasEx ? &Item.m_Ex : nullptr);
		}
		World.NetObjEnd();
		SQueries Q;
		int Tiles = 0;
		for(int i = 0; i < Width * Height; i++)
		{
			CDoorTile Door;
			Collision.GetDoorTile(i, &Door);
			Q.Int(Door.m_Index);
			Q.Int(Door.m_Flags);
			Q.Int(Door.m_Number);
			Tiles += Door.m_Index != 0;
		}
		fprintf(f, "grid %d %08x\n", Tiles, Q.Crc());
	}
	fclose(f);
	return 0;
}

// "png": every *.png below the given directories (sorted relative paths) through
// CImageLoader::LoadPng (from memory, as the file overload does after reading) and
// ConvertToRgbaAlloc:
// "<dir index> <path> fail" | "<dir index> <path> <w> <h> <format> <pnglite flags> <crc data> <crc rgba>"
static int PngMode(const char *pOutFile, int NumDirs, const char **ppDirs)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	for(int d = 0; d < NumDirs; d++)
	{
		std::vector<std::string> vFiles;
		for(const auto &Entry : std::filesystem::recursive_directory_iterator(ppDirs[d]))
			if(Entry.is_regular_file() && Entry.path().extension() == ".png")
				vFiles.push_back(std::filesystem::relative(Entry.path(), ppDirs[d]).generic_string());
		std::sort(vFiles.begin(), vFiles.end());
		for(const std::string &Name : vFiles)
		{
			const std::string Path = std::string(ppDirs[d]) + "/" + Name;
			IOHANDLE File = io_open(Path.c_str(), IOFLAG_READ);
			void *pFileData = nullptr;
			unsigned FileDataSize = 0;
			const bool Read = File && io_read_all(File, &pFileData, &FileDataSize);
			if(File)
				io_close(File);
			CImageInfo Image;
			int Incompatible = 0;
			bool Loaded = false;
			if(Read)
			{
				CByteBufferReader Reader(static_cast<const uint8_t *>(pFileData), FileDataSize);
				Loaded = CImageLoader::LoadPng(Reader, Name.c_str(), Image, Incompatible);
			}
			free(pFileData);
			if(!Loaded)
			{
				fprintf(f, "%d %s fail\n", d, Name.c_str());
				continue;
			}
			uint8_t *pRgba;
			ConvertToRgbaAlloc(pRgba, Image);
			fprintf(f, "%d %s %d %d %d %d %08x %08x\n", d, Name.c_str(), (int)Image.m_Width, (int)Image.m_Height, (int)Image.m_Format,
				Incompatible, Crc(Image.m_pData, Image.DataSize()), Crc(pRgba, Image.m_Width * Image.m_Height * 4));
			free(pRgba);
			Image.Free();
		}
	}
	fclose(f);
	return 0;
}

// "mapimages": the map image decisions of CMapImages::OnMapLoadImpl (mapimages.cpp:59)
// on the real map, then on 100 copies with image and layer item ints changed in memory.
// Per run "run <n>" and the changes "set <item> <int> <value>", then per image one of
// unused | noname | type | external <path> | dims | embedded <w> <h> <crc> | nodata |
// truncated (an item too short for the ints read; C++ would read past it).
static void DumpMapImages(FILE *f, IMap *pMap, CLayers *pLayers)
{
	int Start, Count;
	pMap->GetType(MAPITEMTYPE_IMAGE, &Start, &Count);
	Count = std::clamp<int>(Count, 0, MAX_MAPIMAGES);
	unsigned char aUsed[MAX_MAPIMAGES] = {0};
	for(int g = 0; g < pLayers->NumGroups(); g++)
	{
		const CMapItemGroup *pGroup = pLayers->GetGroup(g);
		if(!pGroup)
			continue;
		for(int l = 0; l < pGroup->m_NumLayers; l++)
		{
			const CMapItemLayer *pLayer = pLayers->GetLayer(pGroup->m_StartLayer + l);
			if(!pLayer)
				continue;
			if(pLayer->m_Type == LAYERTYPE_TILES)
			{
				const int Image = reinterpret_cast<const CMapItemLayerTilemap *>(pLayer)->m_Image;
				if(Image >= 0 && Image < Count)
					aUsed[Image] |= 1;
			}
			else if(pLayer->m_Type == LAYERTYPE_QUADS)
			{
				const int Image = reinterpret_cast<const CMapItemLayerQuads *>(pLayer)->m_Image;
				if(Image >= 0 && Image < Count)
					aUsed[Image] |= 2;
			}
		}
	}
	for(int i = 0; i < Count; i++)
	{
		fprintf(f, "image %d %d ", i, aUsed[i]);
		if(aUsed[i] == 0)
		{
			fprintf(f, "unused\n");
			continue;
		}
		const CMapItemImage_v2 *pImg = static_cast<const CMapItemImage_v2 *>(pMap->GetItem(Start + i));
		const int Size = pMap->GetItemSize(Start + i);
		if(Size < (int)sizeof(CMapItemImage_v1) || (pImg->m_Version > 1 && Size < (int)sizeof(CMapItemImage_v2)))
		{
			fprintf(f, "truncated\n");
			continue;
		}
		const char *pName = pMap->GetDataString(pImg->m_ImageName);
		if(pName == nullptr || pName[0] == '\0')
		{
			if(pImg->m_External)
			{
				fprintf(f, "noname\n");
				continue;
			}
			pName = "(error)";
		}
		if(pImg->m_Version > 1 && pImg->m_MustBe1 != 1)
		{
			fprintf(f, "type\n");
			continue;
		}
		if(pImg->m_External)
		{
			fprintf(f, "external mapres/%s.png\n", pName);
			continue;
		}
		if(pImg->m_Width <= 0 || pImg->m_Height <= 0)
		{
			fprintf(f, "dims\n");
			continue;
		}
		const size_t DataSize = (size_t)pImg->m_Width * pImg->m_Height * 4;
		const void *pData = pMap->GetData(pImg->m_ImageData);
		if(pData && (size_t)pMap->GetDataSize(pImg->m_ImageData) >= DataSize)
			fprintf(f, "embedded %d %d %08x\n", pImg->m_Width, pImg->m_Height, Crc(pData, DataSize));
		else
			fprintf(f, "nodata\n");
	}
}

static int MapImagesMode(const char *pMapFile, const char *pOutFile)
{
	std::unique_ptr<IStorage> pStorage = CreateLocalStorage();
	std::unique_ptr<IMap> pMap = CreateMap();
	CLayers Layers;
	FILE *f = fopen(pOutFile, "wb");
	if(!pStorage || !f || !LoadMapAndLayers(pStorage.get(), pMapFile, pMap.get(), &Layers))
		return -1;
	int ImageStart, ImageCount, LayerStart, LayerCount;
	pMap->GetType(MAPITEMTYPE_IMAGE, &ImageStart, &ImageCount);
	pMap->GetType(MAPITEMTYPE_LAYER, &LayerStart, &LayerCount);
	for(int Run = 0; Run <= 100; Run++)
	{
		fprintf(f, "run %d\n", Run);
		std::vector<std::tuple<int, int, int>> vSaved; // item, int, old value
		for(int k = 0; Run > 0 && k < 1 + (int)(Rand() % 3); k++)
		{
			int Item, Int;
			if((ImageCount > 0 && Rand() % 3 != 0) || LayerCount == 0)
			{
				if(ImageCount == 0)
					break;
				Item = ImageStart + Rand() % ImageCount;
				if(pMap->GetItemSize(Item) < 4)
					continue;
				Int = Rand() % std::min(7, pMap->GetItemSize(Item) / 4);
			}
			else
			{
				Item = LayerStart + Rand() % LayerCount;
				const int Type = static_cast<const CMapItemLayer *>(pMap->GetItem(Item))->m_Type;
				Int = Type == LAYERTYPE_TILES ? 13 : Type == LAYERTYPE_QUADS ? 6 : 1;
				if(Int * 4 >= pMap->GetItemSize(Item))
					continue;
			}
			const int aValues[] = {-1, 0, 1, 2, 3, 5, ImageCount, ImageCount - 1, (int)pMap->NumData() - 1, (int)pMap->NumData(), 0x7fffffff, 1 << 16, (int)(Rand() % 100)};
			const int Value = aValues[Rand() % (sizeof(aValues) / sizeof(aValues[0]))];
			int *pInts = static_cast<int *>(pMap->GetItem(Item));
			vSaved.emplace_back(Item, Int, pInts[Int]);
			pInts[Int] = Value;
			fprintf(f, "set %d %d %d\n", Item, Int, Value);
		}
		DumpMapImages(f, pMap.get(), &Layers);
		for(auto It = vSaved.rbegin(); It != vSaved.rend(); ++It)
			static_cast<int *>(pMap->GetItem(std::get<0>(*It)))[std::get<1>(*It)] = std::get<2>(*It);
	}
	fclose(f);
	return 0;
}

// CEnvelopeState::EnvelopeEval (envelope_state.cpp) on a point access, copied because
// CEnvelopeState needs the whole game client; RenderEvalEnvelope is the real one.
static void EnvelopeStateEval(CMapBasedEnvelopePointAccess &Access, std::chrono::nanoseconds Time, int TimeOffsetMillis, int EnvelopeIndex, ColorRGBA &Result, size_t Channels)
{
	if(EnvelopeIndex < 0)
		return;
	const std::vector<const CMapItemEnvelope *> &vpEnvelopes = Access.Envelopes();
	if(EnvelopeIndex >= (int)vpEnvelopes.size())
		return;
	const CMapItemEnvelope *pEnvelope = vpEnvelopes[EnvelopeIndex];
	if(pEnvelope->m_Channels <= 0)
		return;
	Channels = std::min({Channels, (size_t)pEnvelope->m_Channels, (size_t)CEnvPoint::MAX_CHANNELS});
	Access.SetPointsRange(pEnvelope->m_StartPoint, pEnvelope->m_NumPoints);
	if(Access.NumPoints() == 0)
		return;
	CRenderMap::RenderEvalEnvelope(&Access, Time + std::chrono::milliseconds(TimeOffsetMillis), Result, Channels);
}

static void PrintColor(FILE *f, const ColorRGBA &c)
{
	fprintf(f, " %08x %08x %08x %08x\n", FloatBits(c.r), FloatBits(c.g), FloatBits(c.b), FloatBits(c.a));
}

static long long RandomTime(long long MaxMillis)
{
	const long long Range = std::max(MaxMillis, 1LL) * 1000000LL;
	switch(Rand() % 4)
	{
	case 0: return (long long)(Rand() % 100) * 1000000LL; // early, whole ms
	case 1: return ((long long)Rand() << 20 | Rand()) % (4 * Range) - Range; // incl. negative
	case 2: return ((long long)Rand() << 24 | Rand()) * 1000LL; // large
	default: return ((long long)Rand() << 16 | Rand()) % (2 * Range);
	}
}

// "envelopes": every envelope of the map (and the invalid indices around them) at many
// times, offsets and channel counts, as the client evaluates layer and quad envelopes:
// "<index> <time ns> <offset ms> <channels> <init 0|1> <r> <g> <b> <a>" (float bits).
static int EnvelopesMode(const char *pMapFile, const char *pOutFile)
{
	std::unique_ptr<IStorage> pStorage = CreateLocalStorage();
	std::unique_ptr<IMap> pMap = CreateMap();
	CLayers Layers;
	FILE *f = fopen(pOutFile, "wb");
	if(!pStorage || !f || !LoadMapAndLayers(pStorage.get(), pMapFile, pMap.get(), &Layers))
		return -1;
	CMapBasedEnvelopePointAccess Access(pMap.get());
	const int Num = (int)Access.Envelopes().size();
	fprintf(f, "envelopes %d points %d\n", Num, Access.NumPointsMax());
	for(int Index = -1; Index <= Num; Index++)
	{
		long long MaxMillis = 1000;
		if(Index >= 0 && Index < Num)
		{
			Access.SetPointsRange(Access.Envelopes()[Index]->m_StartPoint, Access.Envelopes()[Index]->m_NumPoints);
			if(Access.NumPoints() > 0)
				MaxMillis = Access.GetPoint(Access.NumPoints() - 1)->m_Time.GetInternal();
		}
		for(int i = 0; i < 100; i++)
		{
			const long long Time = RandomTime(MaxMillis);
			const int Offset = Rand() % 3 == 0 ? 0 : (int)(Rand() % 20001) - 10000;
			const int Channels = 3 + Rand() % 2;
			const bool Init = Rand() % 2;
			ColorRGBA Result = Init ? ColorRGBA(1.0f, 1.0f, 1.0f, 1.0f) : ColorRGBA(0.0f, 0.0f, 0.0f, 0.0f);
			EnvelopeStateEval(Access, std::chrono::nanoseconds(Time), Offset, Index, Result, Channels);
			fprintf(f, "%d %lld %d %d %d", Index, Time, Offset, Channels, Init);
			PrintColor(f, Result);
		}
	}
	fclose(f);
	return 0;
}

class CListPointAccess : public IEnvelopePointAccess
{
public:
	std::vector<CEnvPoint> m_vPoints;
	std::vector<CEnvPointBezier> m_vBeziers; // empty: no bezier data
	int NumPoints() const override { return (int)m_vPoints.size(); }
	const CEnvPoint *GetPoint(int Index) const override { return Index >= 0 && Index < NumPoints() ? &m_vPoints[Index] : nullptr; }
	const CEnvPointBezier *GetBezier(int Index) const override { return Index >= 0 && Index < (int)m_vBeziers.size() ? &m_vBeziers[Index] : nullptr; }
};

// "envelopes-synthetic": random envelopes through RenderEvalEnvelope: every curve type
// (and unknown ones), equal and decreasing point times, bezier tangents of all sizes.
// "env <points> <bezier 0|1>", then "p <time> <curve> <v0..v3> [<16 bezier ints>]",
// then "e <time ns> <channels> <r> <g> <b> <a>" (float bits, result starts at 0).
static int EnvelopesSyntheticMode(const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	auto Signed = [](int Range) { return (int)(Rand() % (2 * Range + 1)) - Range; };
	for(int e = 0; e < 500; e++)
	{
		CListPointAccess Access;
		const int Num = 1 + Rand() % 8;
		const bool HasBezier = Rand() % 3 != 0;
		int Time = Rand() % 100;
		for(int p = 0; p < Num; p++)
		{
			CEnvPoint Point;
			Point.m_Time = CFixedTime(Time);
			Point.m_Curvetype = Rand() % 9 == 0 ? 6 + Rand() % 3 : (Rand() % 3 == 0 ? CURVETYPE_BEZIER : Rand() % 5);
			for(int &Value : Point.m_aValues)
				Value = Rand() % 4 == 0 ? Signed(1024) : Signed(300000);
			Access.m_vPoints.push_back(Point);
			if(HasBezier)
			{
				CEnvPointBezier Bezier;
				for(int c = 0; c < CEnvPoint::MAX_CHANNELS; c++)
				{
					Bezier.m_aInTangentDeltaX[c] = CFixedTime(Signed(Rand() % 2 ? 50 : 3000));
					Bezier.m_aInTangentDeltaY[c] = Signed(Rand() % 2 ? 2048 : 200000);
					Bezier.m_aOutTangentDeltaX[c] = CFixedTime(Signed(Rand() % 2 ? 50 : 3000));
					Bezier.m_aOutTangentDeltaY[c] = Signed(Rand() % 2 ? 2048 : 200000);
				}
				Access.m_vBeziers.push_back(Bezier);
			}
			const int Step = Rand() % 6;
			Time += Step == 0 ? 0 : Step == 1 ? -(int)(Rand() % 50) : (int)(Rand() % 3000);
		}
		fprintf(f, "env %d %d\n", Num, HasBezier);
		for(int p = 0; p < Num; p++)
		{
			const CEnvPoint &P = Access.m_vPoints[p];
			fprintf(f, "p %d %d %d %d %d %d", P.m_Time.GetInternal(), P.m_Curvetype, P.m_aValues[0], P.m_aValues[1], P.m_aValues[2], P.m_aValues[3]);
			if(HasBezier)
			{
				const CEnvPointBezier &B = Access.m_vBeziers[p];
				for(int c = 0; c < 4; c++)
					fprintf(f, " %d", B.m_aInTangentDeltaX[c].GetInternal());
				for(int c = 0; c < 4; c++)
					fprintf(f, " %d", B.m_aInTangentDeltaY[c]);
				for(int c = 0; c < 4; c++)
					fprintf(f, " %d", B.m_aOutTangentDeltaX[c].GetInternal());
				for(int c = 0; c < 4; c++)
					fprintf(f, " %d", B.m_aOutTangentDeltaY[c]);
			}
			fprintf(f, "\n");
		}
		const long long MaxMillis = std::max(Access.m_vPoints.back().m_Time.GetInternal(), 1);
		for(int i = 0; i < 40; i++)
		{
			long long Time = RandomTime(MaxMillis);
			if(i < Num)
				Time = Access.m_vPoints[i].m_Time.GetInternal() * 1000000LL; // exactly on a point
			const int Channels = 1 + Rand() % 4;
			ColorRGBA Result(0.0f, 0.0f, 0.0f, 0.0f);
			CRenderMap::RenderEvalEnvelope(&Access, std::chrono::nanoseconds(Time), Result, Channels);
			fprintf(f, "e %lld %d", Time, Channels);
			PrintColor(f, Result);
		}
	}
	fclose(f);
	return 0;
}

// "screen": IGraphics::MapScreenToWorld (engine/graphics.cpp, with CalcScreenParams) for
// random cameras, aspects, zooms and group parallax/offsets, as CRenderLayerGroup::Render
// calls it: "<inputs as float bits> <left> <top> <right> <bottom>".
static int ScreenMode(const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	// Both functions only use their arguments, so no real IGraphics object is needed
	// (IGraphics is abstract; the tool has no graphics backend).
	alignas(IGraphics) static unsigned char s_aNoGraphics[sizeof(IGraphics)] = {};
	const IGraphics *pGraphics = reinterpret_cast<const IGraphics *>(s_aNoGraphics);
	const float aAspects[] = {4.0f / 3.0f, 16.0f / 9.0f, 16.0f / 10.0f, 21.0f / 9.0f, 1.0f, 0.5f, 3.0f};
	const float aZooms[] = {1.0f, 0.5f, 2.0f, 0.9f, 1.1f};
	for(int i = 0; i < 5000; i++)
	{
		const float Aspect = Rand() % 2 ? aAspects[Rand() % 7] : (float)(1 + Rand() % 4000) / (float)(1 + Rand() % 4000);
		const float Zoom = Rand() % 2 ? aZooms[Rand() % 5] : (1 + Rand() % 2000) / 100.0f;
		const float CenterX = (float)((int)(Rand() % 200001) - 100000) + (Rand() % 1000) / 1000.0f;
		const float CenterY = (float)((int)(Rand() % 200001) - 100000) + (Rand() % 1000) / 1000.0f;
		const int ParallaxX = Rand() % 3 == 0 ? 100 : (int)(Rand() % 501) - 200;
		const int ParallaxY = Rand() % 3 == 0 ? 100 : (int)(Rand() % 501) - 200;
		const float ParallaxZoom = (float)std::clamp(std::max(ParallaxX, ParallaxY), 0, 100);
		const int OffsetX = Rand() % 2 ? 0 : (int)(Rand() % 10001) - 5000;
		const int OffsetY = Rand() % 2 ? 0 : (int)(Rand() % 10001) - 5000;
		const CScreenRect Rect = pGraphics->MapScreenToWorld(CenterX, CenterY, ParallaxX, ParallaxY, ParallaxZoom, OffsetX, OffsetY, Aspect, Zoom);
		fprintf(f, "%08x %08x %08x %08x %d %d %d %d %08x %08x %08x %08x\n", FloatBits(Aspect), FloatBits(Zoom), FloatBits(CenterX), FloatBits(CenterY),
			ParallaxX, ParallaxY, OffsetX, OffsetY,
			FloatBits(Rect.m_TopLeft.x), FloatBits(Rect.m_TopLeft.y), FloatBits(Rect.m_BottomRight.x), FloatBits(Rect.m_BottomRight.y));
	}
	fclose(f);
	return 0;
}

static void PrintKeyframe(FILE *f, const CAnimKeyframe *pFrame)
{
	fprintf(f, " %08x %08x %08x", FloatBits(pFrame->m_X), FloatBits(pFrame->m_Y), FloatBits(pFrame->m_Angle));
}

static void PrintAnimState(FILE *f, const CAnimState &State)
{
	PrintKeyframe(f, State.GetBody());
	PrintKeyframe(f, State.GetBackFoot());
	PrintKeyframe(f, State.GetFrontFoot());
	PrintKeyframe(f, State.GetAttach());
	fprintf(f, "\n");
}

// "anims": CAnimState as CPlayers::RenderPlayer blends it (players.cpp:650): base, one
// movement animation at a time (keyframe times, random times, and the walk/run times
// RenderPlayer derives from a position), then hammer or ninja swing at a time in [0, 1].
// Lines: "idle <state>", "set <anim> <time> <state>", "add <anim> <time> <amount>
// <state>" and "player <x> <anim> <swing anim> <swing time> <walk> <run> <state>",
// floats by their bits; <state> is x, y, angle of body, back foot, front foot, attach.
static int AnimsMode(const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	fprintf(f, "idle");
	PrintAnimState(f, *CAnimState::GetIdle());
	std::vector<float> vTimes = {0.0f, 1.0f, 0.5f};
	for(int a = 0; a < NUM_ANIMS; a++)
	{
		const CAnimation &Anim = g_pData->m_aAnimations[a];
		for(const CAnimSequence *pSeq : {&Anim.m_Body, &Anim.m_BackFoot, &Anim.m_FrontFoot, &Anim.m_Attach})
			for(int i = 0; i < pSeq->m_NumFrames; i++)
				vTimes.push_back(pSeq->m_aFrames[i].m_Time);
	}
	for(int i = 0; i < 100; i++)
		vTimes.push_back((Rand() % 1000001) / 1000000.0f);
	for(int a = 0; a < NUM_ANIMS; a++)
	{
		for(float Time : vTimes)
		{
			CAnimState State;
			State.Set(&g_pData->m_aAnimations[a], Time);
			fprintf(f, "set %d %08x", a, FloatBits(Time));
			PrintAnimState(f, State);
			const float Amount = (Rand() % 2001) / 1000.0f - 1.0f;
			State.Set(&g_pData->m_aAnimations[ANIM_BASE], 0.0f);
			State.Add(&g_pData->m_aAnimations[a], Time, Amount);
			fprintf(f, "add %d %08x %08x", a, FloatBits(Time), FloatBits(Amount));
			PrintAnimState(f, State);
		}
	}
	const int aMoves[] = {ANIM_INAIR, ANIM_SIT_LEFT, ANIM_SIT_RIGHT, ANIM_IDLE, ANIM_WALK, ANIM_RUN_LEFT, ANIM_RUN_RIGHT};
	for(int i = 0; i < 5000; i++)
	{
		const float X = (float)((int)(Rand() % 400001) - 200000) / (float)(1 + Rand() % 64);
		const int Move = aMoves[Rand() % std::size(aMoves)];
		const int Swing = Rand() % 3 == 0 ? -1 : (Rand() % 2 ? ANIM_HAMMER_SWING : ANIM_NINJA_SWING);
		const float SwingTime = std::clamp(((int)(Rand() % 1400) - 200) / 1000.0f, 0.0f, 1.0f);
		// C++: RenderPlayer
		float WalkTime = std::fmod(X, 100.0f) / 100.0f;
		float RunTime = std::fmod(X, 200.0f) / 200.0f;
		if(WalkTime < 0.0f)
			WalkTime += 1.0f;
		if(RunTime < 0.0f)
			RunTime += 1.0f;
		CAnimState State;
		State.Set(&g_pData->m_aAnimations[ANIM_BASE], 0.0f);
		const float MoveTime = Move == ANIM_WALK ? WalkTime : (Move == ANIM_RUN_LEFT || Move == ANIM_RUN_RIGHT) ? RunTime : 0.0f;
		State.Add(&g_pData->m_aAnimations[Move], MoveTime, 1.0f);
		if(Swing >= 0)
			State.Add(&g_pData->m_aAnimations[Swing], SwingTime, 1.0f);
		fprintf(f, "player %08x %d %d %08x %08x %08x", FloatBits(X), Move, Swing, FloatBits(SwingTime), FloatBits(WalkTime), FloatBits(RunTime));
		PrintAnimState(f, State);
	}
	fclose(f);
	return 0;
}

// "colors": tee colours from packed HSL ints, as CTeeRenderInfo::ApplyColors
// (UnclampLighting) and the team colours (plain color_cast). Lines: "<packed> <r g b a
// with UnclampLighting> <r g b a without>", floats by their bits.
static int ColorsMode(const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	std::vector<int> vColors = {0, 255, 65408, 7039872, 65461, 10223541, 0xffffff, 0xff0000, 0xfe0000, 0xd5ffff, 0x2aff80, -1, (int)0x80000000};
	for(int h = 0; h < 256; h++)
		vColors.push_back(h << 16 | 0xff80);
	for(int i = 0; i < 5000; i++)
		vColors.push_back((int)(Rand() << 8 ^ Rand()));
	for(int c : vColors)
	{
		const ColorRGBA A = color_cast<ColorRGBA>(ColorHSLA(c).UnclampLighting(ColorHSLA::DARKEST_LGT));
		const ColorRGBA B = color_cast<ColorRGBA>(ColorHSLA(c));
		fprintf(f, "%d %08x %08x %08x %08x %08x %08x %08x %08x\n", c, FloatBits(A.r), FloatBits(A.g), FloatBits(A.b), FloatBits(A.a),
			FloatBits(B.r), FloatBits(B.g), FloatBits(B.b), FloatBits(B.a));
	}
	fclose(f);
	return 0;
}

// Copies of CGraphics_Threaded::CheckImageDivisibility (graphics_threaded.cpp:593, without
// the warning), CheckMetrics (skins.cpp:254) and CSkins::LoadSkinData (skins.cpp:289):
// they are members of classes that need a running client; the image functions they call
// (ResizeImage, HighestBit, ConvertToGrayscale) are the real ones.
static bool SkinCheckImageDivisibility(CImageInfo &Image, int DivX, int DivY, bool AllowResize)
{
	bool ImageIsValid = true;
	bool WidthBroken = Image.m_Width == 0 || (Image.m_Width % DivX) != 0;
	bool HeightBroken = Image.m_Height == 0 || (Image.m_Height % DivY) != 0;
	if(WidthBroken || HeightBroken)
		ImageIsValid = false;
	if(AllowResize && !ImageIsValid && Image.m_Width > 0 && Image.m_Height > 0)
	{
		int NewWidth = DivX;
		int NewHeight = DivY;
		if(WidthBroken)
		{
			NewWidth = std::max(HighestBit(Image.m_Width), DivX);
			NewHeight = (NewWidth / DivX) * DivY;
		}
		else
		{
			NewHeight = std::max(HighestBit(Image.m_Height), DivY);
			NewWidth = (NewHeight / DivY) * DivX;
		}
		ResizeImage(Image, NewWidth, NewHeight);
		ImageIsValid = true;
	}
	return ImageIsValid;
}

struct SSkinMetric
{
	int m_Width = std::numeric_limits<int>::lowest(), m_Height = std::numeric_limits<int>::lowest();
	int m_OffsetX = std::numeric_limits<int>::max(), m_OffsetY = std::numeric_limits<int>::max();
	int m_MaxWidth = std::numeric_limits<int>::lowest(), m_MaxHeight = std::numeric_limits<int>::lowest();
};

static void SkinCheckMetrics(SSkinMetric &Metrics, const uint8_t *pImg, int ImgWidth, int ImgX, int ImgY, int CheckWidth, int CheckHeight)
{
	int MaxY = -1;
	int MinY = CheckHeight + 1;
	int MaxX = -1;
	int MinX = CheckWidth + 1;
	for(int y = 0; y < CheckHeight; y++)
	{
		for(int x = 0; x < CheckWidth; x++)
		{
			int OffsetAlpha = (y + ImgY) * ImgWidth + (x + ImgX) * 4 + 3;
			uint8_t AlphaValue = pImg[OffsetAlpha];
			if(AlphaValue > 0)
			{
				if(MaxY < y)
					MaxY = y;
				if(MinY > y)
					MinY = y;
				if(MaxX < x)
					MaxX = x;
				if(MinX > x)
					MinX = x;
			}
		}
	}
	// CSkinMetricVariableSize keeps the maximum, CSkinMetricVariableInt the minimum
	Metrics.m_Width = std::max(Metrics.m_Width, std::clamp((MaxX - MinX) + 1, 1, CheckWidth));
	Metrics.m_Height = std::max(Metrics.m_Height, std::clamp((MaxY - MinY) + 1, 1, CheckHeight));
	Metrics.m_OffsetX = std::min(Metrics.m_OffsetX, std::clamp(MinX, 0, CheckWidth - 1));
	Metrics.m_OffsetY = std::min(Metrics.m_OffsetY, std::clamp(MinY, 0, CheckHeight - 1));
	Metrics.m_MaxWidth = std::max(Metrics.m_MaxWidth, CheckWidth);
	Metrics.m_MaxHeight = std::max(Metrics.m_MaxHeight, CheckHeight);
}

static bool SkinLoadData(CImageInfo &Info, CImageInfo &InfoGrayscale, ColorRGBA &BloodColor, SSkinMetric &Body, SSkinMetric &Feet)
{
	if(!SkinCheckImageDivisibility(Info, g_pData->m_aSprites[SPRITE_TEE_BODY].m_pSet->m_Gridx, g_pData->m_aSprites[SPRITE_TEE_BODY].m_pSet->m_Gridy, true))
		return false;
	if(Info.m_Format != CImageInfo::FORMAT_RGBA)
		return false;
	const size_t BodyWidth = g_pData->m_aSprites[SPRITE_TEE_BODY].m_W * (Info.m_Width / g_pData->m_aSprites[SPRITE_TEE_BODY].m_pSet->m_Gridx);
	const size_t BodyHeight = g_pData->m_aSprites[SPRITE_TEE_BODY].m_H * (Info.m_Height / g_pData->m_aSprites[SPRITE_TEE_BODY].m_pSet->m_Gridy);
	if(BodyWidth > Info.m_Width || BodyHeight > Info.m_Height)
		return false;
	auto Rect = [&](int Sprite, int &X, int &Y, int &W, int &H) {
		const CDataSprite &S = g_pData->m_aSprites[Sprite];
		const int GridW = Info.m_Width / S.m_pSet->m_Gridx, GridH = Info.m_Height / S.m_pSet->m_Gridy;
		X = S.m_X * GridW;
		Y = S.m_Y * GridH;
		W = S.m_W * GridW;
		H = S.m_H * GridH;
	};
	const size_t PixelStep = Info.PixelSize();
	const size_t Pitch = Info.m_Width * PixelStep;
	{
		int64_t aColors[3] = {0};
		for(size_t y = 0; y < BodyHeight; y++)
			for(size_t x = 0; x < BodyWidth; x++)
			{
				const size_t Offset = y * Pitch + x * PixelStep;
				if(Info.m_pData[Offset + 3] > 128)
					for(size_t c = 0; c < 3; c++)
						aColors[c] += Info.m_pData[Offset + c];
			}
		const vec3 NormalizedColor = normalize(vec3(aColors[0], aColors[1], aColors[2]));
		BloodColor = ColorRGBA(NormalizedColor.x, NormalizedColor.y, NormalizedColor.z);
	}
	int X, Y, W, H;
	SkinCheckMetrics(Body, Info.m_pData, Pitch, 0, 0, BodyWidth, BodyHeight);
	Rect(SPRITE_TEE_BODY_OUTLINE, X, Y, W, H);
	SkinCheckMetrics(Body, Info.m_pData, Pitch, X, Y, W, H);
	Rect(SPRITE_TEE_FOOT, X, Y, W, H);
	SkinCheckMetrics(Feet, Info.m_pData, Pitch, X, Y, W, H);
	Rect(SPRITE_TEE_FOOT_OUTLINE, X, Y, W, H);
	SkinCheckMetrics(Feet, Info.m_pData, Pitch, X, Y, W, H);

	InfoGrayscale = Info.DeepCopy();
	ConvertToGrayscale(InfoGrayscale);
	int aFreq[256] = {0};
	uint8_t OrgWeight = 1;
	uint8_t NewWeight = 192;
	for(size_t y = 0; y < BodyHeight; y++)
		for(size_t x = 0; x < BodyWidth; x++)
		{
			const size_t Offset = y * Pitch + x * PixelStep;
			if(InfoGrayscale.m_pData[Offset + 3] > 128)
				aFreq[InfoGrayscale.m_pData[Offset]]++;
		}
	for(int i = 1; i < 256; i++)
		if(aFreq[OrgWeight] < aFreq[i])
			OrgWeight = i;
	for(size_t y = 0; y < BodyHeight; y++)
		for(size_t x = 0; x < BodyWidth; x++)
		{
			const size_t Offset = y * Pitch + x * PixelStep;
			uint8_t v = InfoGrayscale.m_pData[Offset];
			if(v <= OrgWeight)
				v = (uint8_t)((v / (float)OrgWeight) * NewWeight);
			else
				v = (uint8_t)(((v - OrgWeight) / (float)(255 - OrgWeight)) * (255 - NewWeight) + NewWeight);
			InfoGrayscale.m_pData[Offset] = v;
			InfoGrayscale.m_pData[Offset + 1] = v;
			InfoGrayscale.m_pData[Offset + 2] = v;
		}
	return true;
}

// "skins": every PNG in <dir> loaded like CSkins::LoadSkinDirect (file PNG loader, then
// LoadSkinData), as is and as variants: cropped to sizes that need the bicubic resize,
// and without alpha. Lines: "<file> <variant> fail" or "<file> <variant> <w> <h> <blood
// r g b bits> <body metric> <feet metric> <crc image> <crc grayscale>".
static int SkinsMode(const char *pOutFile, const char *pDir)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	std::vector<std::string> vFiles;
	for(const auto &Entry : std::filesystem::directory_iterator(pDir))
		if(Entry.is_regular_file() && Entry.path().extension() == ".png")
			vFiles.push_back(Entry.path().filename().generic_string());
	std::sort(vFiles.begin(), vFiles.end());
	for(const std::string &Name : vFiles)
	{
		const std::string Path = std::string(pDir) + "/" + Name;
		CImageInfo Original;
		int Incompatible;
		IOHANDLE File = io_open(Path.c_str(), IOFLAG_READ);
		if(!File || !CImageLoader::LoadPng(File, Name.c_str(), Original, Incompatible))
		{
			fprintf(f, "%s - fail\n", Name.c_str());
			continue;
		}
		// variants: 0 as is, 1/2 cropped (width, height not divisible), 3 RGB
		for(int Variant = 0; Variant < 4; Variant++)
		{
			CImageInfo Info = Original.DeepCopy();
			if(Variant == 1 || Variant == 2)
			{
				const size_t W = Variant == 1 ? Info.m_Width * 3 / 4 + 1 : Info.m_Width;
				const size_t H = Variant == 2 ? Info.m_Height * 3 / 4 + 1 : Info.m_Height;
				CImageInfo Cropped;
				Cropped.m_Width = W;
				Cropped.m_Height = H;
				Cropped.m_Format = Info.m_Format;
				Cropped.Allocate();
				Cropped.CopyRectFrom(Info, 0, 0, W, H, 0, 0);
				Info.Free();
				Info = std::move(Cropped);
			}
			else if(Variant == 3 && Info.m_Format == CImageInfo::FORMAT_RGBA)
			{
				for(size_t i = 0; i < Info.m_Width * Info.m_Height; i++)
					mem_copy(&Info.m_pData[i * 3], &Info.m_pData[i * 4], 3);
				Info.m_Format = CImageInfo::FORMAT_RGB;
			}
			CImageInfo Gray;
			ColorRGBA Blood;
			SSkinMetric Body, Feet;
			if(!SkinLoadData(Info, Gray, Blood, Body, Feet))
				fprintf(f, "%s %d fail\n", Name.c_str(), Variant);
			else
				fprintf(f, "%s %d %d %d %08x %08x %08x %d %d %d %d %d %d %d %d %d %d %d %d %08x %08x\n", Name.c_str(), Variant, (int)Info.m_Width, (int)Info.m_Height,
					FloatBits(Blood.r), FloatBits(Blood.g), FloatBits(Blood.b),
					Body.m_Width, Body.m_Height, Body.m_OffsetX, Body.m_OffsetY, Body.m_MaxWidth, Body.m_MaxHeight,
					Feet.m_Width, Feet.m_Height, Feet.m_OffsetX, Feet.m_OffsetY, Feet.m_MaxWidth, Feet.m_MaxHeight,
					Crc(Info.m_pData, Info.DataSize()), Crc(Gray.m_pData, Gray.DataSize()));
			Info.Free();
			Gray.Free();
		}
		Original.Free();
	}
	fclose(f);
	return 0;
}

// Copy of the non-local part of CPlayers::GetPlayerTargetAngle (players.cpp:123): a
// member of a component that needs a running client.
static float OtherTargetAngle(const CNetObj_Character *pPrevChar, const CNetObj_Character *pPlayerChar, const CNetObj_DDNetCharacter *pExtendedData, const CNetObj_DDNetCharacter *pPrevExtendedData, float Intra)
{
	if(pExtendedData)
	{
		if(pPrevExtendedData)
		{
			float MixX = mix((float)pPrevExtendedData->m_TargetX, (float)pExtendedData->m_TargetX, Intra);
			float MixY = mix((float)pPrevExtendedData->m_TargetY, (float)pExtendedData->m_TargetY, Intra);
			return angle(vec2(MixX, MixY));
		}
		else
		{
			return angle(vec2(pExtendedData->m_TargetX, pExtendedData->m_TargetY));
		}
	}
	else
	{
		if(pPlayerChar->m_Angle > (256.0f * pi) && pPrevChar->m_Angle < 0)
		{
			return mix((float)pPrevChar->m_Angle, (float)(pPlayerChar->m_Angle - 256.0f * 2 * pi), Intra) / 256.0f;
		}
		else if(pPlayerChar->m_Angle < 0 && pPrevChar->m_Angle > (256.0f * pi))
		{
			return mix((float)pPrevChar->m_Angle, (float)(pPlayerChar->m_Angle + 256.0f * 2 * pi), Intra) / 256.0f;
		}
		else
		{
			return mix((float)pPrevChar->m_Angle, (float)pPlayerChar->m_Angle, Intra) / 256.0f;
		}
	}
}

// "interp": the render fractions of CClient::Update (client.cpp:2934, copied: CClient
// needs a running client) for random snapshot ticks and game/predicted times, positions
// mixed like UpdateRenderedCharacters (gameclient.cpp:3762) and GetPlayerTargetAngle for
// other players (copied above), with angles that cross the top, DDNet targets with and
// without a previous one. Lines: "intra <prev tick> <cur tick> <game now> <pred now>
// <intra> <since prev> <tick time> <pred intra> <prev pred tick>", "pos <prev x y> <cur
// x y> <amount> <x> <y>", "angle <prev> <cur> <mode> <prev target x y> <target x y>
// <intra> <angle>" (mode 0: no target, 1: target, 2: target and previous target).
static int InterpMode(const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	const int64_t Freq = time_freq();
	const int TickSpeed = SERVER_TICK_SPEED;
	for(int i = 0; i < 20000; i++)
	{
		const int CurTick = 1 + Rand() % 2000000;
		const int PrevTick = CurTick - 1 - (Rand() % 4 == 0 ? Rand() % 50 : 0);
		const int64_t PrevTickStart = PrevTick * Freq / TickSpeed;
		const int64_t Now = PrevTickStart + (int64_t)((int)(Rand() % 60) - 10) * (Freq / 1000) + Rand() % 1000000;
		const int64_t PredNow = Now + (int64_t)(Rand() % 300) * (Freq / 1000) + Rand() % 1000000;
		// C++: CClient::Update
		int64_t CurTickStart = CurTick * Freq / TickSpeed;
		int PrevPredTick = (int)(PredNow * TickSpeed / Freq);
		int NewPredTick = PrevPredTick + 1;
		const float GameIntraTick = (Now - PrevTickStart) / (float)(CurTickStart - PrevTickStart);
		const float GameTickTime = (Now - PrevTickStart) / (float)Freq;
		const float GameIntraTickSincePrev = (Now - PrevTickStart) / (float)(Freq / TickSpeed);
		int64_t CurPredTickStart = NewPredTick * Freq / TickSpeed;
		int64_t PrevPredTickStart = PrevPredTick * Freq / TickSpeed;
		const float PredIntraTick = (PredNow - PrevPredTickStart) / (float)(CurPredTickStart - PrevPredTickStart);
		fprintf(f, "intra %d %d %lld %lld %08x %08x %08x %08x %d\n", PrevTick, CurTick, (long long)Now, (long long)PredNow,
			FloatBits(GameIntraTick), FloatBits(GameIntraTickSincePrev), FloatBits(GameTickTime), FloatBits(PredIntraTick), PrevPredTick);

		auto RandPos = [] { return (int)(Rand() % 400000) - 50000; };
		CNetObj_Character Prev = {}, Cur = {};
		Prev.m_X = RandPos();
		Prev.m_Y = RandPos();
		Cur.m_X = Rand() % 2 ? Prev.m_X + (int)(Rand() % 61) - 30 : RandPos();
		Cur.m_Y = Rand() % 2 ? Prev.m_Y + (int)(Rand() % 61) - 30 : RandPos();
		const float Amount = Rand() % 2 ? GameIntraTick : PredIntraTick;
		const vec2 Pos = mix(vec2(Prev.m_X, Prev.m_Y), vec2(Cur.m_X, Cur.m_Y), Amount);
		fprintf(f, "pos %d %d %d %d %08x %08x %08x\n", Prev.m_X, Prev.m_Y, Cur.m_X, Cur.m_Y, FloatBits(Amount), FloatBits(Pos.x), FloatBits(Pos.y));

		// angles as CCharacterCore sends them: (int)(angle * 256), angle in (-pi/2, 3pi/2),
		// a third of them crossing the top (between below 0 and above 256 pi)
		auto RandAngle = [] { return (int)(Rand() % 1609) - 402; };
		Prev.m_Angle = RandAngle();
		Cur.m_Angle = RandAngle();
		if(Rand() % 3 == 0)
		{
			Prev.m_Angle = -(int)(Rand() % 402) - 1;
			Cur.m_Angle = 805 + Rand() % 400;
			if(Rand() % 2)
				std::swap(Prev.m_Angle, Cur.m_Angle);
		}
		CNetObj_DDNetCharacter Ext = {}, PrevExt = {};
		const int Mode = Rand() % 3;
		Ext.m_TargetX = (int)(Rand() % 2001) - 1000;
		Ext.m_TargetY = (int)(Rand() % 2001) - 1000;
		PrevExt.m_TargetX = Rand() % 4 == 0 ? Ext.m_TargetX : (int)(Rand() % 2001) - 1000;
		PrevExt.m_TargetY = Rand() % 4 == 0 ? 0 : (int)(Rand() % 2001) - 1000;
		if(Rand() % 10 == 0)
			Ext.m_TargetX = Ext.m_TargetY = 0;
		const float Angle = OtherTargetAngle(&Prev, &Cur, Mode ? &Ext : nullptr, Mode == 2 ? &PrevExt : nullptr, GameIntraTick);
		fprintf(f, "angle %d %d %d %d %d %d %d %08x %08x\n", Prev.m_Angle, Cur.m_Angle, Mode, PrevExt.m_TargetX, PrevExt.m_TargetY, Ext.m_TargetX, Ext.m_TargetY,
			FloatBits(GameIntraTick), FloatBits(Angle));
	}
	fclose(f);
	return 0;
}

// "items": snapshot entities through the real ExtractProjectileInfo, ExtractPickupInfo and
// ExtractLaserInfo (with and without EntityEx, with and without tune zones on the map)
// and projectile positions through CalcPos at random times and tunings. Lines: "<kind>
// <object type name> <ints...> ex <has> <3 ints> tz <0|1> => <results>", floats by their bits.
static int ItemsMode(const char *pMapFile, const char *pOutFile)
{
	std::unique_ptr<IStorage> pStorage = CreateLocalStorage();
	std::unique_ptr<IMap> pMap = CreateMap();
	CLayers Layers;
	FILE *f = fopen(pOutFile, "wb");
	if(!pStorage || !f || !LoadMapAndLayers(pStorage.get(), pMapFile, pMap.get(), &Layers))
		return -1;
	CCollision Collision;
	Collision.Init(&Layers);
	static CTuningParams s_aItemTuning[TuneZone::NUM];
	CGameWorld World;
	World.Init(&Collision, s_aItemTuning, nullptr);
	const int MapW = Collision.GetWidth() * 32, MapH = Collision.GetHeight() * 32;
	auto RandInt = [] { return Rand() % 4 == 0 ? (int)(Rand() % 21) - 10 : (int)(Rand() << 8 ^ Rand()); };
	auto PrintInts = [&](const char *pKind, int Type, const int *pInts, int Num, const CNetObj_EntityEx *pEx, bool Tune) {
		// type names: the extended types' numbers are not part of the protocol
		const char *pType = Type == NETOBJTYPE_PROJECTILE ? "projectile" : Type == NETOBJTYPE_DDRACEPROJECTILE ? "ddraceprojectile" : Type == NETOBJTYPE_DDNETPROJECTILE ? "ddnetprojectile" : Type == NETOBJTYPE_PICKUP ? "pickup" : Type == NETOBJTYPE_DDNETPICKUP ? "ddnetpickup" : Type == NETOBJTYPE_LASER ? "laser" : "ddnetlaser";
		fprintf(f, "%s %s", pKind, pType);
		for(int i = 0; i < Num; i++)
			fprintf(f, " %d", pInts[i]);
		fprintf(f, " ex %d %d %d %d tz %d =>", pEx != nullptr, pEx ? pEx->m_SwitchNumber : 0, pEx ? pEx->m_Layer : 0, pEx ? pEx->m_EntityClass : 0, Tune);
	};
	for(int i = 0; i < 12000; i++)
	{
		const bool Tune = Rand() % 2;
		World.m_WorldConfig.m_UseTuneZones = Tune;
		CNetObj_EntityEx Ex = {(int)(Rand() % 4 == 0 ? RandInt() : Rand() % 20), (int)(Rand() % 4), (int)(Rand() % 12)};
		const CNetObj_EntityEx *pEx = Rand() % 2 ? &Ex : nullptr;
		int aInts[10];
		for(int &v : aInts)
			v = RandInt();
		const int Kind = i % 3;
		if(Kind == 0)
		{
			const int aTypes[] = {NETOBJTYPE_PROJECTILE, NETOBJTYPE_DDRACEPROJECTILE, NETOBJTYPE_DDNETPROJECTILE};
			const int Type = aTypes[Rand() % 3];
			// positions on the map (hundredths for the DDNet/DDRace objects), sane velocities
			const bool Hundredths = Type != NETOBJTYPE_PROJECTILE || Rand() % 2;
			aInts[0] = (int)(Rand() % (MapW + 200)) - 100;
			aInts[1] = (int)(Rand() % (MapH + 200)) - 100;
			if(Hundredths)
			{
				aInts[0] *= 100;
				aInts[1] *= 100;
			}
			if(Rand() % 2)
			{
				aInts[2] = (int)(Rand() % 4001) - 2000;
				aInts[3] = (int)(Rand() % 4001) - 2000;
			}
			if(Type == NETOBJTYPE_PROJECTILE && Rand() % 2)
				aInts[3] = (int)(Rand() % 0x4000) | LEGACYPROJECTILEFLAG_IS_DDNET; // legacy extra info
			if(Type == NETOBJTYPE_DDRACEPROJECTILE)
				aInts[2] = (int)(Rand() % 6283186) - 3141593; // angle in millionths
			aInts[4] = Rand() % 2 ? (int)(Rand() % 6) : aInts[4];
			aInts[5] = (int)(Rand() % 100000);
			if(Type == NETOBJTYPE_DDNETPROJECTILE)
			{
				aInts[6] = Rand() % 2 ? (int)(Rand() % 130) - 1 : aInts[6];
				aInts[9] = (int)(Rand() % 64);
			}
			const int Num = Type == NETOBJTYPE_DDNETPROJECTILE ? 10 : 6;
			PrintInts("proj", Type, aInts, Num, pEx, Tune);
			const CProjectileData D = ExtractProjectileInfo(Type, aInts, &World, pEx);
			fprintf(f, " %08x %08x %08x %08x %d %d %d %d %d %d %d %d %d", FloatBits(D.m_StartPos.x), FloatBits(D.m_StartPos.y), FloatBits(D.m_StartVel.x), FloatBits(D.m_StartVel.y),
				D.m_Type, D.m_StartTick, D.m_ExtraInfo, D.m_Owner, D.m_Explosive, D.m_Bouncing, D.m_Freeze, D.m_SwitchNumber, D.m_TuneZone);
			// C++ RenderProjectile: CalcPos at a time and the time 1 ms before
			const float Curvature = Rand() % 3 ? (float)(Rand() % 1000) / 100.0f : 1.25f;
			const float Speed = Rand() % 3 ? (float)(Rand() % 400000) / 100.0f : 2200.0f;
			const float Ct = (Rand() % 3000) / 1000.0f + (Rand() % 1000) / 1000000.0f;
			const vec2 Pos = CalcPos(D.m_StartPos, D.m_StartVel, Curvature, Speed, Ct);
			const vec2 Prev = CalcPos(D.m_StartPos, D.m_StartVel, Curvature, Speed, Ct - 0.001f);
			fprintf(f, " calc %08x %08x %08x %08x %08x %08x %08x\n", FloatBits(Curvature), FloatBits(Speed), FloatBits(Ct), FloatBits(Pos.x), FloatBits(Pos.y), FloatBits(Prev.x), FloatBits(Prev.y));
		}
		else if(Kind == 1)
		{
			const int Type = Rand() % 2 ? NETOBJTYPE_PICKUP : NETOBJTYPE_DDNETPICKUP;
			const int Num = Type == NETOBJTYPE_DDNETPICKUP ? 6 : 4;
			PrintInts("pickup", Type, aInts, Num, pEx, Tune);
			const CPickupData D = ExtractPickupInfo(Type, aInts, pEx);
			fprintf(f, " %08x %08x %d %d %d %d\n", FloatBits(D.m_Pos.x), FloatBits(D.m_Pos.y), D.m_Type, D.m_Subtype, D.m_SwitchNumber, D.m_Flags);
		}
		else
		{
			const int Type = Rand() % 2 ? NETOBJTYPE_LASER : NETOBJTYPE_DDNETLASER;
			// "from" on the map for tune zones
			const int FromX = 2, FromY = 3; // m_FromX/m_FromY in both objects
			aInts[FromX] = (int)(Rand() % (MapW + 200)) - 100;
			aInts[FromY] = (int)(Rand() % (MapH + 200)) - 100;
			if(Type == NETOBJTYPE_DDNETLASER && Rand() % 2)
				aInts[7] = (int)(Rand() % 5) - 2;
			const int Num = Type == NETOBJTYPE_DDNETLASER ? 10 : 5;
			PrintInts("laser", Type, aInts, Num, pEx, Tune);
			const CLaserData D = ExtractLaserInfo(Type, aInts, &World, pEx);
			fprintf(f, " %08x %08x %08x %08x %d %d %d %d %d %d %d %d\n", FloatBits(D.m_From.x), FloatBits(D.m_From.y), FloatBits(D.m_To.x), FloatBits(D.m_To.y),
				D.m_StartTick, D.m_ExtraInfo, D.m_Owner, D.m_Type, D.m_SwitchNumber, D.m_Subtype, D.m_TuneZone, D.m_Predict);
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
	if(argc >= 4 && str_comp(argv[1], "png") == 0)
		return PngMode(argv[2], argc - 3, argv + 3);
	if(argc == 4 && str_comp(argv[1], "envelopes") == 0)
		return EnvelopesMode(argv[2], argv[3]);
	if(argc == 3 && str_comp(argv[1], "screen") == 0)
		return ScreenMode(argv[2]);
	if(argc == 3 && str_comp(argv[1], "anims") == 0)
		return AnimsMode(argv[2]);
	if(argc == 3 && str_comp(argv[1], "colors") == 0)
		return ColorsMode(argv[2]);
	if(argc == 3 && str_comp(argv[1], "interp") == 0)
		return InterpMode(argv[2]);
	if(argc == 3 && str_comp(argv[1], "movement") == 0)
		return MovementMode(argv[2]);
	if(argc == 3 && str_comp(argv[1], "camera") == 0)
		return CameraMode(argv[2]);
	if(argc == 4 && str_comp(argv[1], "items") == 0)
		return ItemsMode(argv[2], argv[3]);
	if(argc == 4 && str_comp(argv[1], "skins") == 0)
		return SkinsMode(argv[2], argv[3]);
	if(argc == 3 && str_comp(argv[1], "envelopes-synthetic") == 0)
		return EnvelopesSyntheticMode(argv[2]);
	if(argc == 4 && str_comp(argv[1], "mapimages") == 0)
		return MapImagesMode(argv[2], argv[3]);
	if(argc == 4 && str_comp(argv[1], "doors") == 0)
		return DoorsMode(argv[2], argv[3]);
	if(argc == 4 && str_comp(argv[1], "netents") == 0)
		return NetEntsMode(argv[2], argv[3]);
	if(argc == 3 && str_comp(argv[1], "chat") == 0)
		return ChatMode(argv[2]);
	if(argc == 3 && str_comp(argv[1], "scoreboard") == 0)
		return ScoreboardMode(argv[2]);
	if(argc >= 4 && str_comp(argv[1], "wv") == 0)
		return WvMode(argv[2], argc - 3, argv + 3);
	if(argc == 3 && str_comp(argv[1], "antiping-timing") == 0)
		return AntiPingMode(nullptr, argv[2]);
	if(argc == 4 && (str_comp(argv[1], "msgs") == 0 || str_comp(argv[1], "antiping") == 0))
	{
		std::unique_ptr<IStorage> pStorage = CreateLocalStorage();
		std::unique_ptr<IMap> pMap = CreateMap();
		CLayers Layers;
		if(!pStorage || !LoadMapAndLayers(pStorage.get(), argv[2], pMap.get(), &Layers))
			return -1;
		CCollision Collision;
		Collision.Init(&Layers);
		if(str_comp(argv[1], "antiping") == 0)
			return AntiPingMode(&Collision, argv[3]);
		return MsgsMode(pMap.get(), &Collision, argv[3]);
	}
	if(argc >= 4 && (str_comp(argv[1], "collision") == 0 || str_comp(argv[1], "core") == 0 || str_comp(argv[1], "world") == 0 || str_comp(argv[1], "weapons") == 0))
	{
		bool HookTele = false, Verbose = false;
		for(int i = 4; i < argc; i++)
		{
			HookTele |= str_comp(argv[i], "hooktele") == 0;
			Verbose |= str_comp(argv[i], "verbose") == 0;
		}
		if(str_comp(argv[1], "world") == 0)
			return WorldMode(argv[2], argv[3], HookTele, Verbose);
		if(str_comp(argv[1], "weapons") == 0)
			return WeaponsMode(argv[2], argv[3], Verbose);
		if(str_comp(argv[1], "core") == 0)
			return CoreMode(argv[2], argv[3], HookTele, Verbose);
		return CollisionMode(argv[2], argv[3], HookTele, Verbose);
	}
	log_error(TOOL_NAME, "Usage: %s map <map file> <output file> <scratch dir> | collision|core|world|weapons <map file> <output file> [hooktele] [verbose] | doors|netents|mapimages|envelopes|items|msgs|antiping <map file> <output file> | envelopes-synthetic|screen|anims|colors|interp|camera|movement|antiping-timing|chat|scoreboard <output file> | wv <output file> <dir>... | png <output file> <dir>... | skins <output file> <dir>", argc > 0 ? argv[0] : TOOL_NAME);
	return -1;
}
