// Parity data for the Rust client (rust-client/): runs the real C++ map loading and
// physics code and writes what it computed, for rust-client/ddnet/tests/parity.rs.
//
// Usage: rs_parity map <map file> <output file> <scratch dir>
//        rs_parity collision <map file> <output file> [hooktele] [verbose]
//        rs_parity core <map file> <output file> [hooktele] [verbose]
//        rs_parity world <map file> <output file> [hooktele] [verbose]
//        rs_parity doors <map file> <output file>
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
#include <engine/shared/config.h>
#include <engine/shared/uuid_manager.h>
#include <engine/storage.h>

#include <game/client/prediction/entities/character.h>
#include <game/client/prediction/gameworld.h>
#include <game/collision.h>
#include <game/gamecore.h>
#include <game/teamscore.h>
#include <game/layers.h>
#include <game/mapitems.h>

#include <zlib.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <functional>
#include <map>
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

int main(int argc, const char **argv)
{
	CCmdlineFix CmdlineFix(&argc, &argv);
	log_set_global_logger_default();
	if(argc == 5 && str_comp(argv[1], "map") == 0)
		return MapMode(argv[2], argv[3], argv[4]);
	if(argc == 4 && str_comp(argv[1], "doors") == 0)
		return DoorsMode(argv[2], argv[3]);
	if(argc >= 4 && (str_comp(argv[1], "collision") == 0 || str_comp(argv[1], "core") == 0 || str_comp(argv[1], "world") == 0))
	{
		bool HookTele = false, Verbose = false;
		for(int i = 4; i < argc; i++)
		{
			HookTele |= str_comp(argv[i], "hooktele") == 0;
			Verbose |= str_comp(argv[i], "verbose") == 0;
		}
		if(str_comp(argv[1], "world") == 0)
			return WorldMode(argv[2], argv[3], HookTele, Verbose);
		if(str_comp(argv[1], "core") == 0)
			return CoreMode(argv[2], argv[3], HookTele, Verbose);
		return CollisionMode(argv[2], argv[3], HookTele, Verbose);
	}
	log_error(TOOL_NAME, "Usage: %s map <map file> <output file> <scratch dir> | collision|core|world <map file> <output file> [hooktele] [verbose] | doors <map file> <output file>", argc > 0 ? argv[0] : TOOL_NAME);
	return -1;
}
