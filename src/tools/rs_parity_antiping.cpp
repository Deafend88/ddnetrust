// "antiping" mode of rs_parity: the client prediction loop (M8.5) and its timing, for
// rust-client/ddnet/tests/parity.rs.
//
// The prediction loop (UpdatePrediction, ApplyPreInputs, OnPredict incl. the smoothing
// block, GetSmoothPos, IsOtherTeam, the AntiPing gates, GetGameInfo) and the timing
// functions (GetPredictionTime, GetPredictionTick, GetSmoothTick) are CGameClient and
// CClient members that cannot be linked into this tool. They are copied here word for
// word into stand-in classes of the same names (in their own namespace), marked
// "C++ copy: <file>:<lines>" ... "end of copy"; the Rust test `cpp_copies_match_source`
// checks that every copy is still identical (whitespace aside) to those source lines.
// What the copies call is the real code: CGameWorld, CCharacter, the entities,
// CCharacterCore, CTeamsCore, CCharOrder, CTuningParams. `time_get()` is this file's
// clock, so the dump can give the Rust side the same "now".
//
// A "server" CGameWorld runs 4 tees on random inputs (the local one on the inputs the
// client sent) with weapons; every 1-3 ticks a snapshot of its characters, projectiles
// and lasers goes to the stand-in client (UpdatePrediction), with Sv_PreInputs for the
// other tees, then 1-3 frames predict ahead (OnPredict) at a latency that drifts.

#include <base/log.h>
#include <base/math.h>
#include <base/str.h>
#include <base/time.h>
#include <base/vmath.h>

#include <engine/client.h>
#include <engine/shared/config.h>
#include <engine/shared/protocol.h>

#include <game/client/gameclient.h>
#include <game/client/laser_data.h>
#include <game/client/prediction/entities/character.h>
#include <game/client/prediction/entities/laser.h>
#include <game/client/prediction/entities/projectile.h>
#include <game/client/prediction/gameworld.h>
#include <game/client/projectile_data.h>
#include <game/collision.h>
#include <game/gamecore.h>
#include <game/teamscore.h>

#include <generated/protocol.h>

#include <zlib.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <map>
#include <memory>
#include <vector>

// rs_parity.cpp
void CoreStateInts(std::vector<int> &vValues, const CCharacterCore &Core);
void NetWorldInts(std::vector<int> &vValues, CGameWorld &World);
void CharStateInts(std::vector<int> &vValues, CCharacter *pChar);

namespace rs_antiping {

static unsigned s_Seed = 31337;
static unsigned Rand()
{
	s_Seed = s_Seed * 1103515245u + 12345u;
	return (s_Seed >> 8) & 0xffffff;
}

// 48 random bits (Rand() alone is smaller than a tick in time_freq units)
static int64_t Rand64()
{
	const int64_t High = Rand();
	return (High << 24) | Rand();
}

static unsigned Crc(const std::vector<int> &v)
{
	return crc32(0, (const unsigned char *)v.data(), (unsigned)(v.size() * sizeof(int)));
}

static unsigned Bits(float f)
{
	unsigned u;
	mem_copy(&u, &f, sizeof(u));
	return u;
}

// the copies' time_get(): the dump's "now"
static int64_t s_Now = 0;
static int64_t time_get()
{
	return s_Now;
}

struct STimeValue
{
	int64_t m_Value = 0;
	int64_t Get(int64_t Now) const { return m_Value; }
};

// The members of the real CClient that the copied code reads.
class CClient
{
public:
	STimeValue m_PredictedTime;
	STimeValue m_aGameTime[2];
	int m_GameTick = 0;
	int m_PrevGameTick = 0;
	int m_PredGameTick = 0;
	float m_IntraGameTick = 0.0f;
	float m_PredIntraGameTick = 0.0f;
	// the inputs sent, by tick
	mutable std::map<int, CNetObj_PlayerInput> m_Inputs;

	int GameTickSpeed() const { return SERVER_TICK_SPEED; }
	int GameTick(int Conn) const { return m_GameTick; }
	int PrevGameTick(int Conn) const { return m_PrevGameTick; }
	int PredGameTick(int Conn) const { return m_PredGameTick; }
	float IntraGameTick(int Conn) const { return m_IntraGameTick; }
	float PredIntraGameTick(int Conn) const { return m_PredIntraGameTick; }
	bool DummyConnected() const { return false; }
	int State() const { return IClient::STATE_ONLINE; }
	// the current snapshot's items for ProcessEvents
	std::vector<IClient::CSnapItem> m_vSnapItems;
	int SnapNumItems(int SnapId) const { return (int)m_vSnapItems.size(); }
	IClient::CSnapItem SnapGetItem(int SnapId, int Index) const { return m_vSnapItems[Index]; }
	// C++ CClient::GetInput: the latest input sent for a tick at or before `Tick`
	int *GetInput(int Tick, int IsDummy = 0) const
	{
		auto It = m_Inputs.upper_bound(Tick);
		if(It == m_Inputs.begin())
			return nullptr;
		return (int *)&std::prev(It)->second;
	}
	int GetPredictionTime();
	int GetPredictionTick();
	void GetSmoothTick(int *pSmoothTick, float *pSmoothIntraTick, float MixAmount);
};

// C++ copy: engine/client/client.cpp:5383-5426
int CClient::GetPredictionTime()
{
	int64_t Now = time_get();
	return (int)((m_PredictedTime.Get(Now) - m_aGameTime[g_Config.m_ClDummy].Get(Now)) * 1000 / (float)time_freq());
}

int CClient::GetPredictionTick()
{
	int PredictionTick = GetPredictionTime() * GameTickSpeed() / 1000.0f;

	int PredictionMin = g_Config.m_ClAntiPingLimit * GameTickSpeed() / 1000.0f;

	if(g_Config.m_ClAntiPingLimit == 0)
	{
		float PredictionPercentage = 1 - g_Config.m_ClAntiPingPercent / 100.0f;
		PredictionMin = std::floor(PredictionTick * PredictionPercentage);
	}

	if(PredictionMin > PredictionTick - 1)
	{
		PredictionMin = PredictionTick - 1;
	}

	if(PredictionMin <= 0)
		return PredGameTick(g_Config.m_ClDummy);

	PredictionTick = PredGameTick(g_Config.m_ClDummy) - PredictionMin;

	if(PredictionTick < GameTick(g_Config.m_ClDummy) + 1)
	{
		PredictionTick = GameTick(g_Config.m_ClDummy) + 1;
	}
	return PredictionTick;
}

void CClient::GetSmoothTick(int *pSmoothTick, float *pSmoothIntraTick, float MixAmount)
{
	int64_t GameTime = m_aGameTime[g_Config.m_ClDummy].Get(time_get());
	int64_t PredTime = m_PredictedTime.Get(time_get());
	int64_t SmoothTime = std::clamp(GameTime + (int64_t)(MixAmount * (PredTime - GameTime)), GameTime, PredTime);

	*pSmoothTick = (int)(SmoothTime * GameTickSpeed() / time_freq()) + 1;
	*pSmoothIntraTick = (SmoothTime - (*pSmoothTick - 1) * time_freq() / GameTickSpeed()) / (float)(time_freq() / GameTickSpeed());
}
// end of copy

struct SCharacterSnap
{
	bool m_Active = false;
	CNetObj_Character m_Prev = {};
	CNetObj_Character m_Cur = {};
	bool m_HasExtendedData = false;
	CNetObj_DDNetCharacter m_ExtendedData = {};
};

struct SSpecInfo
{
	bool m_Active = false;
	int m_SpectatorId = SPEC_FREEVIEW;
};

struct SSnapState
{
	const CNetObj_Character *m_pLocalCharacter = nullptr;
	const CNetObj_Character *m_pLocalPrevCharacter = nullptr;
	const CNetObj_GameInfo *m_pGameInfoObj = nullptr;
	int m_LocalClientId = -1;
	SSpecInfo m_SpecInfo;
	SCharacterSnap m_aCharacters[MAX_CLIENTS];
};

struct SClientData
{
	CCharacterCore m_Predicted;
	CCharacterCore m_PrevPredicted;
	int64_t m_aSmoothStart[2] = {};
	int64_t m_aSmoothLen[2] = {};
	vec2 m_aPredPos[200] = {};
	int m_aPredTick[200] = {};
	CNetMsg_Sv_PreInput m_aPreInputs[200] = {};
	bool m_Solo = false;
	int m_Team = 0;
	bool m_Paused = false;
	CNetObj_Character m_RenderCur = {};
	CNetObj_Character m_RenderPrev = {};
	bool m_IsPredicted = false;
	bool m_IsPredictedLocal = false;
	vec2 m_RenderPos = vec2(0, 0);
};

// The sounds the copies play (CSounds/CMapSounds calls; CEffects plays its sounds itself
// when snd_game is on): "<set>:<x bits>:<y bits>", map sounds "m<id>:<x bits>:<y bits>",
// dumped and cleared with each state.
static std::vector<std::string> s_vSounds;
static void Played(const char *pPrefix, int Id, vec2 Pos)
{
	char aBuf[64];
	str_format(aBuf, sizeof(aBuf), "%s%d:%08x:%08x", pPrefix, Id, Bits(Pos.x), Bits(Pos.y));
	s_vSounds.emplace_back(aBuf);
}

struct SSounds
{
	void PlayAt(int Channel, int SetId, float Volume, vec2 Position) { Played("", SetId, Position); }
	void PlayAndRecord(int Channel, int SetId, float Volume, vec2 Position) { PlayAt(Channel, SetId, Volume, Position); }
};

struct SMapSounds
{
	void PlayAt(int Channel, int SoundId, vec2 Position) { Played("m", SoundId, Position); }
};

// CEffects: only its sounds (effects.cpp:48, 214, 408)
struct SEffects
{
	void AirJump(vec2 Pos, float Alpha, float Volume)
	{
		if(g_Config.m_SndGame)
			Played("", SOUND_PLAYER_AIRJUMP, Pos);
	}
	void PlayerSpawn(vec2 Pos, float Alpha, float Volume)
	{
		if(g_Config.m_SndGame)
			Played("", SOUND_PLAYER_SPAWN, Pos);
	}
	void HammerHit(vec2 Pos, float Alpha, float Volume)
	{
		if(g_Config.m_SndGame)
			Played("", SOUND_HAMMER_HIT, Pos);
	}
	void DamageIndicator(vec2 Pos, vec2 Dir, float Alpha) {}
	void Explosion(vec2 Pos, float Alpha) {}
	void Confetti(vec2 Pos, float Alpha) {}
	void PlayerDeath(vec2 Pos, int ClientId, float Alpha) {}
};

struct SGhost
{
	void OnNewPredictedSnapshot() {}
};

// The members of the real CGameClient that the copied code reads and writes.
class CGameClient
{
public:
	SSnapState m_Snap;
	SClientData m_aClients[MAX_CLIENTS];
	CGameWorld m_GameWorld;
	CGameWorld m_PredictedWorld;
	CGameWorld m_PrevPredictedWorld;
	CCharacterCore m_PredictedChar;
	CCharacterCore m_PredictedPrevChar;
	int m_PredictedTick = -1;
	int m_aLastNewPredictedTick[2] = {-1, -1};
	bool m_NewPredictedTick = false;
	bool m_NewTick = false;
	vec2 m_aLastPos[MAX_CLIENTS] = {};
	bool m_aLastActive[MAX_CLIENTS] = {};
	bool m_SuppressEvents = false;
	int m_IsDummySwapping = 0;
	int m_aLocalIds[2] = {-1, -1};
	CTeamsCore m_Teams;
	CCharOrder m_CharOrder;
	CGameInfo m_GameInfo = {};
	CTuningParams m_aTuning[2];
	SEffects m_Effects;
	SSounds m_Sounds;
	SMapSounds m_MapSounds;
	CCollision *m_pCollision = nullptr;
	SGhost m_Ghost;
	CClient m_Client;
	std::vector<CSnapEntities> m_vSnapEntities;
	vec2 m_LocalCharacterPos = vec2(0, 0);

	CGameClient()
	{
		for(auto &Client : m_aClients)
		{
			Client.m_Predicted.Reset();
			Client.m_PrevPredicted.Reset();
			for(auto &PreInput : Client.m_aPreInputs)
				PreInput.m_IntendedTick = -1;
		}
		m_PredictedChar.Reset();
		m_PredictedPrevChar.Reset();
	}

	CClient *Client() { return &m_Client; }
	const CClient *Client() const { return &m_Client; }
	const std::vector<CSnapEntities> &SnapEntities() const { return m_vSnapEntities; }
	bool IsTeamPlay() const { return m_Snap.m_pGameInfoObj && (m_Snap.m_pGameInfoObj->m_GameFlags & GAMEFLAG_TEAMS) != 0; }
	bool IsWorldPaused() const { return m_Snap.m_pGameInfoObj && (m_Snap.m_pGameInfoObj->m_GameStateFlags & (GAMESTATEFLAG_GAMEOVER | GAMESTATEFLAG_PAUSED)) != 0; }
	void DetectStrongHook() {} // not reached: every character has the extended object
	void HandlePredictedEvents(const int Tick);
	void ProcessEvents();
	void DetectOtherAirJumps();
	CCollision *Collision() { return m_pCollision; }
	CConfig *Config() { return &g_Config; }

	int AntiPingPlayers() const;
	bool AntiPingGrenade() const;
	bool AntiPingWeapons() const;
	bool AntiPingGunfire() const;
	bool Predict() const;
	bool PredictDummy() const;
	void ApplyPreInputs(int Tick, bool Direct, CGameWorld &GameWorld);
	void OnPredict();
	void UpdatePrediction();
	vec2 GetSmoothPos(int ClientId);
	bool IsOtherTeam(int ClientId) const;
	void UpdateRenderedCharacters();
};

// C++ copy: game/client/gameclient.cpp:955-1006
int CGameClient::AntiPingPlayers() const
{
	if(g_Config.m_ClAntiPing &&
		g_Config.m_ClAntiPingPlayers &&
		!m_Snap.m_SpecInfo.m_Active &&
		Client()->State() != IClient::STATE_DEMOPLAYBACK)
	{
		return g_Config.m_ClAntiPingPlayers;
	}
	return 0;
}

bool CGameClient::AntiPingGrenade() const
{
	return g_Config.m_ClAntiPing &&
	       g_Config.m_ClAntiPingGrenade &&
	       !m_Snap.m_SpecInfo.m_Active &&
	       Client()->State() != IClient::STATE_DEMOPLAYBACK;
}

bool CGameClient::AntiPingWeapons() const
{
	return g_Config.m_ClAntiPing &&
	       g_Config.m_ClAntiPingWeapons &&
	       !m_Snap.m_SpecInfo.m_Active &&
	       Client()->State() != IClient::STATE_DEMOPLAYBACK;
}

bool CGameClient::AntiPingGunfire() const
{
	return AntiPingGrenade() &&
	       AntiPingWeapons() &&
	       g_Config.m_ClAntiPingGunfire;
}

bool CGameClient::Predict() const
{
	return g_Config.m_ClPredict &&
	       !IsWorldPaused() &&
	       Client()->State() != IClient::STATE_DEMOPLAYBACK &&
	       !m_Snap.m_SpecInfo.m_Active &&
	       m_Snap.m_pLocalCharacter;
}

bool CGameClient::PredictDummy() const
{
	return g_Config.m_ClPredictDummy &&
	       Client()->DummyConnected() &&
	       m_Snap.m_LocalClientId >= 0 &&
	       m_aLocalIds[!g_Config.m_ClDummy] >= 0 &&
	       !m_aClients[m_aLocalIds[!g_Config.m_ClDummy]].m_Paused;
}
// end of copy

// C++ copy: game/client/gameclient.cpp:1492-1681
static CGameInfo GetGameInfo(const CNetObj_GameInfoEx *pInfoEx, int InfoExSize, const CServerInfo *pFallbackServerInfo)
{
	int Version = -1;
	if(InfoExSize >= 12)
	{
		Version = pInfoEx->m_Version;
	}
	else if(InfoExSize >= 8)
	{
		Version = std::min(pInfoEx->m_Version, 4);
	}
	else if(InfoExSize >= 4)
	{
		Version = 0;
	}
	int Flags = 0;
	if(Version >= 0)
	{
		Flags = pInfoEx->m_Flags;
	}
	int Flags2 = 0;
	if(Version >= 5)
	{
		Flags2 = pInfoEx->m_Flags2;
	}
	bool Race;
	bool FastCap;
	bool FNG;
	bool DDRace;
	bool DDNet;
	bool BlockWorlds;
	bool City;
	bool Vanilla;
	bool Plus;
	bool FDDrace;
	if(Version < 1)
	{
		// The game type is intentionally only available inside this
		// `if`. Game type sniffing should be avoided and ideally not
		// extended. Mods should set the relevant game flags instead.
		const char *pGameType = pFallbackServerInfo->m_aGameType;
		Race = str_find_nocase(pGameType, "race") || str_find_nocase(pGameType, "fastcap");
		FastCap = str_find_nocase(pGameType, "fastcap");
		FNG = str_find_nocase(pGameType, "fng");
		DDRace = str_find_nocase(pGameType, "ddrace") || str_find_nocase(pGameType, "mkrace");
		DDNet = str_find_nocase(pGameType, "ddracenet") || str_find_nocase(pGameType, "ddnet");
		BlockWorlds = str_startswith(pGameType, "bw  ") || str_comp_nocase(pGameType, "bw") == 0;
		City = str_find_nocase(pGameType, "city");
		Vanilla = str_comp(pGameType, "DM") == 0 || str_comp(pGameType, "TDM") == 0 || str_comp(pGameType, "CTF") == 0;
		Plus = str_find(pGameType, "+");
		FDDrace = false;
	}
	else
	{
		Race = Flags & GAMEINFOFLAG_GAMETYPE_RACE;
		FastCap = Flags & GAMEINFOFLAG_GAMETYPE_FASTCAP;
		FNG = Flags & GAMEINFOFLAG_GAMETYPE_FNG;
		DDRace = Flags & GAMEINFOFLAG_GAMETYPE_DDRACE;
		DDNet = Flags & GAMEINFOFLAG_GAMETYPE_DDNET;
		BlockWorlds = Flags & GAMEINFOFLAG_GAMETYPE_BLOCK_WORLDS;
		Vanilla = Flags & GAMEINFOFLAG_GAMETYPE_VANILLA;
		Plus = Flags & GAMEINFOFLAG_GAMETYPE_PLUS;
		City = Version >= 5 && Flags2 & GAMEINFOFLAG2_GAMETYPE_CITY;
		FDDrace = Version >= 6 && Flags2 & GAMEINFOFLAG2_GAMETYPE_FDDRACE;

		// Ensure invariants upheld by the server info parsing business.
		DDRace = DDRace || DDNet || FDDrace;
		Race = Race || FastCap || DDRace;
	}

	CGameInfo Info;
	Info.m_FlagStartsRace = FastCap;
	Info.m_TimeScore = Race;
	Info.m_UnlimitedAmmo = Race;
	Info.m_DDRaceRecordMessage = DDRace && !DDNet;
	Info.m_RaceRecordMessage = DDNet || (Race && !DDRace);
	Info.m_RaceSounds = DDRace || FNG || BlockWorlds;
	Info.m_AllowEyeWheel = DDRace || BlockWorlds || City || Plus;
	Info.m_AllowHookColl = DDRace;
	Info.m_AllowZoom = Race || BlockWorlds || City;
	Info.m_BugDDRaceGhost = DDRace;
	Info.m_BugDDRaceInput = DDRace;
	Info.m_BugFNGLaserRange = FNG;
	Info.m_BugVanillaBounce = Vanilla;
	Info.m_PredictFNG = FNG;
	Info.m_PredictDDRace = DDRace;
	Info.m_PredictDDRaceTiles = DDRace && !BlockWorlds;
	Info.m_PredictVanilla = Vanilla || FastCap;
	Info.m_EntitiesDDNet = DDNet;
	Info.m_EntitiesDDRace = DDRace;
	Info.m_EntitiesRace = Race;
	Info.m_EntitiesFNG = FNG;
	Info.m_EntitiesVanilla = Vanilla;
	Info.m_EntitiesBW = BlockWorlds;
	Info.m_Race = Race;
	Info.m_Pvp = !Race;
	Info.m_DontMaskEntities = !DDNet;
	Info.m_AllowXSkins = false;
	Info.m_EntitiesFDDrace = FDDrace;
	Info.m_HudHealthArmor = true;
	Info.m_HudAmmo = true;
	Info.m_HudDDRace = false;
	Info.m_NoWeakHookAndBounce = false;
	Info.m_NoSkinChangeForFrozen = false;
	Info.m_DDRaceTeam = false;
	Info.m_PredictEvents = Vanilla;
	Info.m_MinTeamSize = 0;
	Info.m_MaxTeamSize = 0;
	Info.m_NumDDRaceTeams = 65; // `TEAM_SUPER + 1`, fallback for ddrace64 servers
	Info.m_OldLaser = false;

	if(Version >= 0)
	{
		Info.m_TimeScore = Flags & GAMEINFOFLAG_TIMESCORE;
	}
	if(Version >= 2)
	{
		Info.m_FlagStartsRace = Flags & GAMEINFOFLAG_FLAG_STARTS_RACE;
		Info.m_UnlimitedAmmo = Flags & GAMEINFOFLAG_UNLIMITED_AMMO;
		Info.m_DDRaceRecordMessage = Flags & GAMEINFOFLAG_DDRACE_RECORD_MESSAGE;
		Info.m_RaceRecordMessage = Flags & GAMEINFOFLAG_RACE_RECORD_MESSAGE;
		Info.m_AllowEyeWheel = Flags & GAMEINFOFLAG_ALLOW_EYE_WHEEL;
		Info.m_AllowHookColl = Flags & GAMEINFOFLAG_ALLOW_HOOK_COLL;
		Info.m_AllowZoom = Flags & GAMEINFOFLAG_ALLOW_ZOOM;
		Info.m_BugDDRaceGhost = Flags & GAMEINFOFLAG_BUG_DDRACE_GHOST;
		Info.m_BugDDRaceInput = Flags & GAMEINFOFLAG_BUG_DDRACE_INPUT;
		Info.m_BugFNGLaserRange = Flags & GAMEINFOFLAG_BUG_FNG_LASER_RANGE;
		Info.m_BugVanillaBounce = Flags & GAMEINFOFLAG_BUG_VANILLA_BOUNCE;
		Info.m_PredictFNG = Flags & GAMEINFOFLAG_PREDICT_FNG;
		Info.m_PredictDDRace = Flags & GAMEINFOFLAG_PREDICT_DDRACE;
		Info.m_PredictDDRaceTiles = Flags & GAMEINFOFLAG_PREDICT_DDRACE_TILES;
		Info.m_PredictVanilla = Flags & GAMEINFOFLAG_PREDICT_VANILLA;
		Info.m_EntitiesDDNet = Flags & GAMEINFOFLAG_ENTITIES_DDNET;
		Info.m_EntitiesDDRace = Flags & GAMEINFOFLAG_ENTITIES_DDRACE;
		Info.m_EntitiesRace = Flags & GAMEINFOFLAG_ENTITIES_RACE;
		Info.m_EntitiesFNG = Flags & GAMEINFOFLAG_ENTITIES_FNG;
		Info.m_EntitiesVanilla = Flags & GAMEINFOFLAG_ENTITIES_VANILLA;
	}
	if(Version >= 3)
	{
		Info.m_Race = Flags & GAMEINFOFLAG_RACE;
		Info.m_DontMaskEntities = Flags & GAMEINFOFLAG_DONT_MASK_ENTITIES;
	}
	if(Version >= 4)
	{
		Info.m_EntitiesBW = Flags & GAMEINFOFLAG_ENTITIES_BW;
	}
	if(Version >= 5)
	{
		Info.m_AllowXSkins = Flags2 & GAMEINFOFLAG2_ALLOW_X_SKINS;
	}
	if(Version >= 6)
	{
		Info.m_EntitiesFDDrace = Flags2 & GAMEINFOFLAG2_ENTITIES_FDDRACE;
	}
	if(Version >= 7)
	{
		Info.m_HudHealthArmor = Flags2 & GAMEINFOFLAG2_HUD_HEALTH_ARMOR;
		Info.m_HudAmmo = Flags2 & GAMEINFOFLAG2_HUD_AMMO;
		Info.m_HudDDRace = Flags2 & GAMEINFOFLAG2_HUD_DDRACE;
	}
	if(Version >= 8)
	{
		Info.m_NoWeakHookAndBounce = Flags2 & GAMEINFOFLAG2_NO_WEAK_HOOK;
	}
	if(Version >= 9)
	{
		Info.m_NoSkinChangeForFrozen = Flags2 & GAMEINFOFLAG2_NO_SKIN_CHANGE_FOR_FROZEN;
	}
	if(Version >= 10)
	{
		Info.m_DDRaceTeam = Flags2 & GAMEINFOFLAG2_DDRACE_TEAM;
	}
	if(Version >= 11)
	{
		Info.m_PredictEvents = Flags2 & GAMEINFOFLAG2_PREDICT_EVENTS;
	}
	if(Version >= 12)
	{
		Info.m_MinTeamSize = pInfoEx->m_MinTeamSize;
		Info.m_MaxTeamSize = pInfoEx->m_MaxTeamSize;
		// Servers from before this field was added send 0, and this client cannot represent more
		// teams than it was compiled with. Fall back to our own count for anything out of range.
		const int NumDDRaceTeams = pInfoEx->m_NumDDRaceTeams;
		Info.m_NumDDRaceTeams = NumDDRaceTeams > TEAM_FLOCK + 1 && NumDDRaceTeams <= NUM_DDRACE_TEAMS ? NumDDRaceTeams : NUM_DDRACE_TEAMS;
		Info.m_OldLaser = Flags2 & GAMEINFOFLAG2_OLD_LASER;
	}

	return Info;
}
// end of copy

// C++ copy: game/client/gameclient.cpp:2540-2854
void CGameClient::ApplyPreInputs(int Tick, bool Direct, CGameWorld &GameWorld)
{
	if(!g_Config.m_ClAntiPingPreInput)
		return;

	for(int ClientId = 0; ClientId < MAX_CLIENTS; ClientId++)
	{
		if(CCharacter *pChar = GameWorld.GetCharacterById(ClientId))
		{
			if(ClientId == m_aLocalIds[0] || (Client()->DummyConnected() && ClientId == m_aLocalIds[1]))
				continue;

			const CNetMsg_Sv_PreInput PreInput = m_aClients[ClientId].m_aPreInputs[Tick % 200];
			if(PreInput.m_IntendedTick != Tick)
				continue;

			//convert preinput to input
			CNetObj_PlayerInput Input = {0};
			Input.m_Direction = PreInput.m_Direction;
			Input.m_TargetX = PreInput.m_TargetX;
			Input.m_TargetY = PreInput.m_TargetY;
			Input.m_Jump = PreInput.m_Jump;
			Input.m_Fire = PreInput.m_Fire;
			Input.m_Hook = PreInput.m_Hook;
			Input.m_WantedWeapon = PreInput.m_WantedWeapon;
			Input.m_NextWeapon = PreInput.m_NextWeapon;
			Input.m_PrevWeapon = PreInput.m_PrevWeapon;

			if(Direct)
			{
				pChar->OnDirectInput(&Input);
			}
			else
			{
				pChar->OnPredictedInput(&Input);
			}
		}
	}
}

void CGameClient::OnPredict()
{
	// store the previous values so we can detect prediction errors
	CCharacterCore BeforePrevChar = m_PredictedPrevChar;
	CCharacterCore BeforeChar = m_PredictedChar;

	// we can't predict without our own id or own character
	if(m_Snap.m_LocalClientId == -1 || !m_Snap.m_aCharacters[m_Snap.m_LocalClientId].m_Active)
		return;

	// don't predict anything if we are paused
	if(m_Snap.m_pGameInfoObj && m_Snap.m_pGameInfoObj->m_GameStateFlags & GAMESTATEFLAG_PAUSED)
	{
		if(m_Snap.m_pLocalCharacter)
		{
			m_PredictedChar.Read(m_Snap.m_pLocalCharacter);
			m_PredictedChar.m_ActiveWeapon = m_Snap.m_pLocalCharacter->m_Weapon;
		}
		if(m_Snap.m_pLocalPrevCharacter)
		{
			m_PredictedPrevChar.Read(m_Snap.m_pLocalPrevCharacter);
			m_PredictedPrevChar.m_ActiveWeapon = m_Snap.m_pLocalPrevCharacter->m_Weapon;
		}
		return;
	}

	vec2 aBeforeRender[MAX_CLIENTS];
	for(int i = 0; i < MAX_CLIENTS; i++)
		aBeforeRender[i] = GetSmoothPos(i);

	// init
	bool Dummy = g_Config.m_ClDummy ^ m_IsDummySwapping;

	// PredictedEvents are only handled in predicted world, so update them here
	m_GameWorld.m_PredictedEvents = m_PredictedWorld.m_PredictedEvents;
	m_PredictedWorld.CopyWorld(&m_GameWorld);

	// don't predict inactive players, or entities from other teams
	for(int i = 0; i < MAX_CLIENTS; i++)
		if(CCharacter *pChar = m_PredictedWorld.GetCharacterById(i))
			if((!m_Snap.m_aCharacters[i].m_Active && pChar->m_SnapTicks > 10) || IsOtherTeam(i))
				pChar->Destroy();

	CProjectile *pProjNext = nullptr;
	for(CProjectile *pProj = (CProjectile *)m_PredictedWorld.FindFirst(CGameWorld::ENTTYPE_PROJECTILE); pProj; pProj = pProjNext)
	{
		pProjNext = (CProjectile *)pProj->TypeNext();
		if(IsOtherTeam(pProj->GetOwner()))
		{
			pProj->Destroy();
		}
	}

	CCharacter *pLocalChar = m_PredictedWorld.GetCharacterById(m_Snap.m_LocalClientId);
	if(!pLocalChar)
		return;
	CCharacter *pDummyChar = nullptr;
	if(PredictDummy())
		pDummyChar = m_PredictedWorld.GetCharacterById(m_aLocalIds[!g_Config.m_ClDummy]);

	int PredictionTick = Client()->GetPredictionTick();
	// predict
	for(int Tick = Client()->GameTick(g_Config.m_ClDummy) + 1; Tick <= Client()->PredGameTick(g_Config.m_ClDummy); Tick++)
	{
		// fetch the previous characters
		if(Tick == PredictionTick)
		{
			for(int i = 0; i < MAX_CLIENTS; i++)
				if(CCharacter *pChar = m_PredictedWorld.GetCharacterById(i))
					m_aClients[i].m_PrevPredicted = pChar->GetCore();
		}

		if(Tick == Client()->PredGameTick(g_Config.m_ClDummy))
		{
			m_PredictedPrevChar = pLocalChar->GetCore();
			m_aClients[m_Snap.m_LocalClientId].m_PrevPredicted = pLocalChar->GetCore();

			if(pDummyChar)
				m_aClients[m_aLocalIds[!g_Config.m_ClDummy]].m_PrevPredicted = pDummyChar->GetCore();
		}

		// optionally allow some movement in freeze by not predicting freeze the last one to two ticks
		if(g_Config.m_ClPredictFreeze == 2 && Client()->PredGameTick(g_Config.m_ClDummy) - 1 - Client()->PredGameTick(g_Config.m_ClDummy) % 2 <= Tick)
			pLocalChar->m_CanMoveInFreeze = true;

		// apply inputs and tick
		CNetObj_PlayerInput *pInputData = (CNetObj_PlayerInput *)Client()->GetInput(Tick, m_IsDummySwapping);
		CNetObj_PlayerInput *pDummyInputData = !pDummyChar ? nullptr : (CNetObj_PlayerInput *)Client()->GetInput(Tick, m_IsDummySwapping ^ 1);
		bool DummyFirst = pInputData && pDummyInputData && pDummyChar->GetCid() < pLocalChar->GetCid();

		if(DummyFirst)
			pDummyChar->OnDirectInput(pDummyInputData);
		if(pInputData)
			pLocalChar->OnDirectInput(pInputData);
		if(pDummyInputData && !DummyFirst)
			pDummyChar->OnDirectInput(pDummyInputData);

		ApplyPreInputs(Tick, true, m_PredictedWorld);

		m_PredictedWorld.m_GameTick = Tick;
		if(pInputData)
			pLocalChar->OnPredictedInput(pInputData);
		if(pDummyInputData)
			pDummyChar->OnPredictedInput(pDummyInputData);

		ApplyPreInputs(Tick, false, m_PredictedWorld);

		m_PredictedWorld.Tick();

		// fetch the current characters
		if(Tick == PredictionTick)
		{
			m_PrevPredictedWorld.CopyWorld(&m_PredictedWorld);

			for(int i = 0; i < MAX_CLIENTS; i++)
				if(CCharacter *pChar = m_PredictedWorld.GetCharacterById(i))
					m_aClients[i].m_Predicted = pChar->GetCore();
		}

		if(Tick == Client()->PredGameTick(g_Config.m_ClDummy))
		{
			m_PredictedChar = pLocalChar->GetCore();
			m_aClients[m_Snap.m_LocalClientId].m_Predicted = pLocalChar->GetCore();

			if(pDummyChar)
				m_aClients[m_aLocalIds[!g_Config.m_ClDummy]].m_Predicted = pDummyChar->GetCore();
		}

		for(int i = 0; i < MAX_CLIENTS; i++)
			if(CCharacter *pChar = m_PredictedWorld.GetCharacterById(i))
			{
				m_aClients[i].m_aPredPos[Tick % 200] = pChar->Core()->m_Pos;
				m_aClients[i].m_aPredTick[Tick % 200] = Tick;
			}

		// check if we want to trigger effects
		if(Tick > m_aLastNewPredictedTick[Dummy])
		{
			m_aLastNewPredictedTick[Dummy] = Tick;
			m_NewPredictedTick = true;
			vec2 Pos = pLocalChar->Core()->m_Pos;
			int Events = pLocalChar->Core()->m_TriggeredEvents;
			if(g_Config.m_ClPredict && !m_SuppressEvents)
				if(Events & COREEVENT_AIR_JUMP)
					m_Effects.AirJump(Pos, 1.0f, 1.0f);
			if(g_Config.m_SndGame && !m_SuppressEvents)
			{
				if(Events & COREEVENT_GROUND_JUMP)
					m_Sounds.PlayAndRecord(CSounds::CHN_WORLD, SOUND_PLAYER_JUMP, 1.0f, Pos);
				if(Events & COREEVENT_HOOK_ATTACH_GROUND)
					m_Sounds.PlayAndRecord(CSounds::CHN_WORLD, SOUND_HOOK_ATTACH_GROUND, 1.0f, Pos);
				if(Events & COREEVENT_HOOK_HIT_NOHOOK)
					m_Sounds.PlayAndRecord(CSounds::CHN_WORLD, SOUND_HOOK_NOATTACH, 1.0f, Pos);
				if(Events & COREEVENT_HOOK_ATTACH_PLAYER)
				{
					m_PredictedWorld.CreatePredictedSound(Pos, SOUND_HOOK_ATTACH_PLAYER, pLocalChar->GetCid());
				}
			}
		}

		// check if we want to trigger predicted airjump for dummy
		if(AntiPingPlayers() && pDummyChar && Tick > m_aLastNewPredictedTick[!Dummy])
		{
			m_aLastNewPredictedTick[!Dummy] = Tick;
			vec2 Pos = pDummyChar->Core()->m_Pos;
			int Events = pDummyChar->Core()->m_TriggeredEvents;
			if(g_Config.m_ClPredict && !m_SuppressEvents)
				if(Events & COREEVENT_AIR_JUMP)
					m_Effects.AirJump(Pos, 1.0f, 1.0f);
		}

		HandlePredictedEvents(Tick);
	}

	// detect mispredictions of other players and make corrections smoother when possible
	if(g_Config.m_ClAntiPingSmooth && Predict() && AntiPingPlayers() && m_NewTick && m_PredictedTick >= MIN_TICK && absolute(m_PredictedTick - Client()->PredGameTick(g_Config.m_ClDummy)) <= 1 && absolute(Client()->GameTick(g_Config.m_ClDummy) - Client()->PrevGameTick(g_Config.m_ClDummy)) <= 2)
	{
		int PredTime = std::clamp(Client()->GetPredictionTime(), 0, 800);
		float SmoothPace = 4 - 1.5f * PredTime / 800.f; // smoothing pace (a lower value will make the smoothing quicker)
		int64_t Len = 1000 * PredTime * SmoothPace;

		for(int i = 0; i < MAX_CLIENTS; i++)
		{
			if(!m_Snap.m_aCharacters[i].m_Active || i == m_Snap.m_LocalClientId || !m_aLastActive[i])
				continue;
			vec2 NewPos = (m_PredictedTick == Client()->PredGameTick(g_Config.m_ClDummy)) ? m_aClients[i].m_Predicted.m_Pos : m_aClients[i].m_PrevPredicted.m_Pos;
			vec2 PredErr = (m_aLastPos[i] - NewPos) / (float)std::min(Client()->GetPredictionTime(), 200);
			if(in_range(length(PredErr), 0.05f, 5.f))
			{
				vec2 PredPos = mix(m_aClients[i].m_PrevPredicted.m_Pos, m_aClients[i].m_Predicted.m_Pos, Client()->PredIntraGameTick(g_Config.m_ClDummy));
				vec2 CurPos = mix(
					vec2(m_Snap.m_aCharacters[i].m_Prev.m_X, m_Snap.m_aCharacters[i].m_Prev.m_Y),
					vec2(m_Snap.m_aCharacters[i].m_Cur.m_X, m_Snap.m_aCharacters[i].m_Cur.m_Y),
					Client()->IntraGameTick(g_Config.m_ClDummy));
				vec2 RenderDiff = PredPos - aBeforeRender[i];
				vec2 PredDiff = PredPos - CurPos;

				float aMixAmount[2];
				for(int j = 0; j < 2; j++)
				{
					aMixAmount[j] = 1.0f;
					if(absolute(PredErr[j]) > 0.05f)
					{
						aMixAmount[j] = 0.0f;
						if(absolute(RenderDiff[j]) > 0.01f)
						{
							aMixAmount[j] = 1.f - std::clamp(RenderDiff[j] / PredDiff[j], 0.f, 1.f);
							aMixAmount[j] = 1.f - std::pow(1.f - aMixAmount[j], 1 / 1.2f);
						}
					}
					int64_t TimePassed = time_get() - m_aClients[i].m_aSmoothStart[j];
					if(in_range(TimePassed, (int64_t)0, Len - 1))
						aMixAmount[j] = std::min(aMixAmount[j], (float)(TimePassed / (double)Len));
				}
				for(int j = 0; j < 2; j++)
					if(absolute(RenderDiff[j]) < 0.01f && absolute(PredDiff[j]) < 0.01f && absolute(m_aClients[i].m_PrevPredicted.m_Pos[j] - m_aClients[i].m_Predicted.m_Pos[j]) < 0.01f && aMixAmount[j] > aMixAmount[j ^ 1])
						aMixAmount[j] = aMixAmount[j ^ 1];
				for(int j = 0; j < 2; j++)
				{
					// don't smooth for longer than 700ms, or more than 300ms longer along one axis than the other axis
					int64_t Remaining = std::min({
						(1.f - aMixAmount[j]) * Len,
						time_freq() * 0.700f,
						(1.f - aMixAmount[j ^ 1]) * Len + time_freq() * 0.300f,
					});
					int64_t Start = time_get() - (Len - Remaining);
					if(!in_range(Start + Len, m_aClients[i].m_aSmoothStart[j], m_aClients[i].m_aSmoothStart[j] + Len))
					{
						m_aClients[i].m_aSmoothStart[j] = Start;
						m_aClients[i].m_aSmoothLen[j] = Len;
					}
				}
			}
		}
	}

	for(int i = 0; i < MAX_CLIENTS; i++)
	{
		if(m_Snap.m_aCharacters[i].m_Active)
		{
			m_aLastPos[i] = m_aClients[i].m_Predicted.m_Pos;
			m_aLastActive[i] = true;
		}
		else
		{
			m_aLastActive[i] = false;
		}
	}

	if(g_Config.m_Debug && g_Config.m_ClPredict && m_PredictedTick == Client()->PredGameTick(g_Config.m_ClDummy))
	{
		CNetObj_CharacterCore Before = {0}, Now = {0}, BeforePrev = {0}, NowPrev = {0};
		BeforeChar.Write(&Before);
		BeforePrevChar.Write(&BeforePrev);
		m_PredictedChar.Write(&Now);
		m_PredictedPrevChar.Write(&NowPrev);

		if(mem_comp(&Before, &Now, sizeof(CNetObj_CharacterCore)) != 0)
		{
			log_trace("client", "prediction error");
			for(unsigned i = 0; i < sizeof(CNetObj_CharacterCore) / sizeof(int); i++)
			{
				if(((int *)&Before)[i] != ((int *)&Now)[i])
				{
					log_trace("client", "	%d %d %d (%d %d)", i, ((int *)&Before)[i], ((int *)&Now)[i], ((int *)&BeforePrev)[i], ((int *)&NowPrev)[i]);
				}
			}
		}
	}

	m_PredictedTick = Client()->PredGameTick(g_Config.m_ClDummy);

	if(m_NewPredictedTick)
		m_Ghost.OnNewPredictedSnapshot();
}
// end of copy

// C++ copy: game/client/gameclient.cpp:3487-3616
void CGameClient::UpdatePrediction()
{
	m_GameWorld.m_WorldConfig.m_IsVanilla = m_GameInfo.m_PredictVanilla;
	m_GameWorld.m_WorldConfig.m_IsDDRace = m_GameInfo.m_PredictDDRace;
	m_GameWorld.m_WorldConfig.m_IsFNG = m_GameInfo.m_PredictFNG;
	m_GameWorld.m_WorldConfig.m_PredictDDRace = m_GameInfo.m_PredictDDRace;
	m_GameWorld.m_WorldConfig.m_PredictTiles = m_GameInfo.m_PredictDDRace && m_GameInfo.m_PredictDDRaceTiles;
	m_GameWorld.m_WorldConfig.m_PredictFreeze = g_Config.m_ClPredictFreeze;
	m_GameWorld.m_WorldConfig.m_PredictWeapons = AntiPingWeapons();
	m_GameWorld.m_WorldConfig.m_BugDDRaceInput = m_GameInfo.m_BugDDRaceInput;
	m_GameWorld.m_WorldConfig.m_NoWeakHookAndBounce = m_GameInfo.m_NoWeakHookAndBounce;
	m_GameWorld.m_WorldConfig.m_PredictEvents = m_GameInfo.m_PredictEvents;
	m_GameWorld.m_WorldConfig.m_OldLaser = m_GameInfo.m_OldLaser;

	if(!m_Snap.m_pLocalCharacter)
	{
		if(CCharacter *pLocalChar = m_GameWorld.GetCharacterById(m_Snap.m_LocalClientId))
			pLocalChar->Destroy();
		return;
	}

	if(m_Snap.m_pLocalCharacter->m_AmmoCount > 0 && m_Snap.m_pLocalCharacter->m_Weapon != WEAPON_NINJA)
		m_GameWorld.m_WorldConfig.m_InfiniteAmmo = false;
	m_GameWorld.m_WorldConfig.m_IsSolo = !m_Snap.m_aCharacters[m_Snap.m_LocalClientId].m_HasExtendedData && !m_aTuning[g_Config.m_ClDummy].m_PlayerCollision && !m_aTuning[g_Config.m_ClDummy].m_PlayerHooking;

	CCharacter *pLocalChar = m_GameWorld.GetCharacterById(m_Snap.m_LocalClientId);
	CCharacter *pDummyChar = nullptr;
	if(PredictDummy())
		pDummyChar = m_GameWorld.GetCharacterById(m_aLocalIds[!g_Config.m_ClDummy]);

	// update strong and weak hook
	if(pLocalChar && !m_Snap.m_SpecInfo.m_Active && Client()->State() != IClient::STATE_DEMOPLAYBACK && (m_aTuning[g_Config.m_ClDummy].m_PlayerCollision || m_aTuning[g_Config.m_ClDummy].m_PlayerHooking))
	{
		if(m_Snap.m_aCharacters[m_Snap.m_LocalClientId].m_HasExtendedData)
		{
			int aIds[MAX_CLIENTS];
			std::fill(std::begin(aIds), std::end(aIds), -1);
			for(int i = 0; i < MAX_CLIENTS; i++)
				if(CCharacter *pChar = m_GameWorld.GetCharacterById(i))
					aIds[pChar->GetStrongWeakId()] = i;
			for(int Id : aIds)
				if(Id >= 0)
					m_CharOrder.GiveStrong(Id);
		}
		else
		{
			// manual detection
			DetectStrongHook();
		}
		for(int i : m_CharOrder.m_Ids)
		{
			if(CCharacter *pChar = m_GameWorld.GetCharacterById(i))
			{
				m_GameWorld.RemoveEntity(pChar);
				m_GameWorld.InsertEntity(pChar);
			}
		}
	}

	// advance the gameworld to the current gametick
	if(pLocalChar && absolute(m_GameWorld.GameTick() - Client()->GameTick(g_Config.m_ClDummy)) < Client()->GameTickSpeed())
	{
		for(int Tick = m_GameWorld.GameTick() + 1; Tick <= Client()->GameTick(g_Config.m_ClDummy); Tick++)
		{
			CNetObj_PlayerInput *pInput = (CNetObj_PlayerInput *)Client()->GetInput(Tick);
			CNetObj_PlayerInput *pDummyInput = nullptr;
			if(pDummyChar)
				pDummyInput = (CNetObj_PlayerInput *)Client()->GetInput(Tick, 1);
			if(pInput)
				pLocalChar->OnDirectInput(pInput);
			if(pDummyInput)
				pDummyChar->OnDirectInput(pDummyInput);

			ApplyPreInputs(Tick, true, m_GameWorld);

			m_GameWorld.m_GameTick = Tick;
			if(pInput)
				pLocalChar->OnPredictedInput(pInput);
			if(pDummyInput)
				pDummyChar->OnPredictedInput(pDummyInput);

			ApplyPreInputs(Tick, false, m_GameWorld);

			m_GameWorld.Tick();

			for(int i = 0; i < MAX_CLIENTS; i++)
				if(CCharacter *pChar = m_GameWorld.GetCharacterById(i))
				{
					m_aClients[i].m_aPredPos[Tick % 200] = pChar->Core()->m_Pos;
					m_aClients[i].m_aPredTick[Tick % 200] = Tick;
				}
		}
	}
	else
	{
		// skip to current gametick
		m_GameWorld.m_GameTick = Client()->GameTick(g_Config.m_ClDummy);
		if(pLocalChar)
			if(CNetObj_PlayerInput *pInput = (CNetObj_PlayerInput *)Client()->GetInput(Client()->GameTick(g_Config.m_ClDummy)))
				pLocalChar->SetInput(pInput);
		if(pDummyChar)
			if(CNetObj_PlayerInput *pInput = (CNetObj_PlayerInput *)Client()->GetInput(Client()->GameTick(g_Config.m_ClDummy), 1))
				pDummyChar->SetInput(pInput);
	}

	for(int i = 0; i < MAX_CLIENTS; i++)
		if(CCharacter *pChar = m_GameWorld.GetCharacterById(i))
		{
			m_aClients[i].m_aPredPos[Client()->GameTick(g_Config.m_ClDummy) % 200] = pChar->Core()->m_Pos;
			m_aClients[i].m_aPredTick[Client()->GameTick(g_Config.m_ClDummy) % 200] = Client()->GameTick(g_Config.m_ClDummy);
		}

	// update the local gameworld with the new snapshot
	m_GameWorld.NetObjBegin(m_Teams, m_Snap.m_LocalClientId);

	for(int i = 0; i < MAX_CLIENTS; i++)
		if(m_Snap.m_aCharacters[i].m_Active)
		{
			bool IsLocal = (i == m_Snap.m_LocalClientId || (PredictDummy() && i == m_aLocalIds[!g_Config.m_ClDummy]));
			int GameTeam = IsTeamPlay() ? m_aClients[i].m_Team : i;
			m_GameWorld.NetCharAdd(i, &m_Snap.m_aCharacters[i].m_Cur,
				m_Snap.m_aCharacters[i].m_HasExtendedData ? &m_Snap.m_aCharacters[i].m_ExtendedData : nullptr,
				GameTeam, IsLocal);
		}

	for(const CSnapEntities &EntData : SnapEntities())
		m_GameWorld.NetObjAdd(EntData.m_Item.m_Id, EntData.m_Item.m_Type, EntData.m_Item.m_pData, EntData.m_pDataEx);

	m_GameWorld.NetObjEnd();
}
// end of copy

// C++ copy: game/client/gameclient.cpp:3762-3817
void CGameClient::UpdateRenderedCharacters()
{
	for(int i = 0; i < MAX_CLIENTS; i++)
	{
		if(!m_Snap.m_aCharacters[i].m_Active)
			continue;
		m_aClients[i].m_RenderCur = m_Snap.m_aCharacters[i].m_Cur;
		m_aClients[i].m_RenderPrev = m_Snap.m_aCharacters[i].m_Prev;
		m_aClients[i].m_IsPredicted = false;
		m_aClients[i].m_IsPredictedLocal = false;
		vec2 UnpredPos = mix(
			vec2(m_Snap.m_aCharacters[i].m_Prev.m_X, m_Snap.m_aCharacters[i].m_Prev.m_Y),
			vec2(m_Snap.m_aCharacters[i].m_Cur.m_X, m_Snap.m_aCharacters[i].m_Cur.m_Y),
			Client()->IntraGameTick(g_Config.m_ClDummy));
		vec2 Pos = UnpredPos;

		CCharacter *pChar = m_PredictedWorld.GetCharacterById(i);
		const bool IsDummy = PredictDummy() && i == m_aLocalIds[!g_Config.m_ClDummy];
		bool AntiPingPlayer = AntiPingPlayers() == 1 || (AntiPingPlayers() >= 2 && (IsDummy || (pChar && pChar->IsInterfering())));
		if(Predict() && (i == m_Snap.m_LocalClientId || (AntiPingPlayer && !IsOtherTeam(i))) && pChar)
		{
			m_aClients[i].m_Predicted.Write(&m_aClients[i].m_RenderCur);
			m_aClients[i].m_PrevPredicted.Write(&m_aClients[i].m_RenderPrev);

			m_aClients[i].m_IsPredicted = true;

			Pos = mix(
				vec2(m_aClients[i].m_RenderPrev.m_X, m_aClients[i].m_RenderPrev.m_Y),
				vec2(m_aClients[i].m_RenderCur.m_X, m_aClients[i].m_RenderCur.m_Y),
				m_aClients[i].m_IsPredicted ? Client()->PredIntraGameTick(g_Config.m_ClDummy) : Client()->IntraGameTick(g_Config.m_ClDummy));

			if(i == m_Snap.m_LocalClientId || IsDummy)
			{
				m_aClients[i].m_IsPredictedLocal = true;
				if(AntiPingGunfire() && ((pChar->m_NinjaJetpack && pChar->m_FreezeTime == 0) || m_Snap.m_aCharacters[i].m_Cur.m_Weapon != WEAPON_NINJA || m_Snap.m_aCharacters[i].m_Cur.m_Weapon == m_aClients[i].m_Predicted.m_ActiveWeapon))
				{
					m_aClients[i].m_RenderCur.m_AttackTick = pChar->GetAttackTick();
					if(m_Snap.m_aCharacters[i].m_Cur.m_Weapon != WEAPON_NINJA && !(pChar->m_NinjaJetpack && pChar->Core()->m_ActiveWeapon == WEAPON_GUN))
						m_aClients[i].m_RenderCur.m_Weapon = m_aClients[i].m_Predicted.m_ActiveWeapon;
				}
			}
			else
			{
				// use unpredicted values for other players
				m_aClients[i].m_RenderPrev.m_Angle = m_Snap.m_aCharacters[i].m_Prev.m_Angle;
				m_aClients[i].m_RenderCur.m_Angle = m_Snap.m_aCharacters[i].m_Cur.m_Angle;

				if(g_Config.m_ClAntiPingSmooth)
					Pos = GetSmoothPos(i);
			}
		}
		m_aClients[i].m_RenderPos = Pos;
		if(Predict() && i == m_Snap.m_LocalClientId)
			m_LocalCharacterPos = Pos;
	}
}
// end of copy

// C++ copy: game/client/gameclient.cpp:3951-3970
vec2 CGameClient::GetSmoothPos(int ClientId)
{
	vec2 Pos = mix(m_aClients[ClientId].m_PrevPredicted.m_Pos, m_aClients[ClientId].m_Predicted.m_Pos, Client()->PredIntraGameTick(g_Config.m_ClDummy));
	int64_t Now = time_get();
	for(int i = 0; i < 2; i++)
	{
		int64_t Len = std::clamp(m_aClients[ClientId].m_aSmoothLen[i], (int64_t)1, time_freq());
		int64_t TimePassed = Now - m_aClients[ClientId].m_aSmoothStart[i];
		if(in_range(TimePassed, (int64_t)0, Len - 1))
		{
			float MixAmount = 1.f - std::pow(1.f - TimePassed / (float)Len, 1.2f);
			int SmoothTick;
			float SmoothIntra;
			Client()->GetSmoothTick(&SmoothTick, &SmoothIntra, MixAmount);
			if(SmoothTick > 0 && m_aClients[ClientId].m_aPredTick[(SmoothTick - 1) % 200] >= Client()->PrevGameTick(g_Config.m_ClDummy) && m_aClients[ClientId].m_aPredTick[SmoothTick % 200] <= Client()->PredGameTick(g_Config.m_ClDummy))
				Pos[i] = mix(m_aClients[ClientId].m_aPredPos[(SmoothTick - 1) % 200][i], m_aClients[ClientId].m_aPredPos[SmoothTick % 200][i], SmoothIntra);
		}
	}
	return Pos;
}
// end of copy

// C++ copy: game/client/gameclient.cpp:3977-4005
bool CGameClient::IsOtherTeam(int ClientId) const
{
	bool Local = m_Snap.m_LocalClientId == ClientId;

	if(m_Snap.m_LocalClientId < 0)
	{
		return false;
	}
	else if((m_Snap.m_SpecInfo.m_Active && m_Snap.m_SpecInfo.m_SpectatorId == SPEC_FREEVIEW) || ClientId < 0)
	{
		return false;
	}
	else if(m_Snap.m_SpecInfo.m_Active && m_Snap.m_SpecInfo.m_SpectatorId != SPEC_FREEVIEW)
	{
		if(m_Teams.Team(ClientId) == m_Teams.TeamSuper() || m_Teams.Team(m_Snap.m_SpecInfo.m_SpectatorId) == m_Teams.TeamSuper())
			return false;
		return m_Teams.Team(ClientId) != m_Teams.Team(m_Snap.m_SpecInfo.m_SpectatorId);
	}
	else if((m_aClients[m_Snap.m_LocalClientId].m_Solo || m_aClients[ClientId].m_Solo) && !Local)
	{
		return true;
	}

	if(m_Teams.Team(ClientId) == m_Teams.TeamSuper() || m_Teams.Team(m_Snap.m_LocalClientId) == m_Teams.TeamSuper())
		return false;

	return m_Teams.Team(ClientId) != m_Teams.Team(m_Snap.m_LocalClientId);
}

// end of copy

// The timing parts of CItems::RenderProjectile and CItems::RenderLaser.
class CItems
{
public:
	CGameClient *m_pGameClient;
	CGameClient *GameClient() { return m_pGameClient; }
	CClient *Client() { return &m_pGameClient->m_Client; }

	float ProjectileTime(const CProjectileData *pCurrent, bool LocalPlayerInGame, float s_LastGameTickTime)
	{
// C++ copy: game/client/components/items.cpp:55-63
	bool IsOtherTeam = (pCurrent->m_ExtraInfo && pCurrent->m_Owner >= 0 && GameClient()->IsOtherTeam(pCurrent->m_Owner));

	int PredictionTick = Client()->GetPredictionTick();

	float Ct;
	if(GameClient()->Predict() && GameClient()->AntiPingGrenade() && LocalPlayerInGame && !IsOtherTeam)
		Ct = ((float)(PredictionTick - 1 - pCurrent->m_StartTick) + Client()->PredIntraGameTick(g_Config.m_ClDummy)) / (float)Client()->GameTickSpeed();
	else
		Ct = (Client()->PrevGameTick(g_Config.m_ClDummy) - pCurrent->m_StartTick) / (float)Client()->GameTickSpeed() + s_LastGameTickTime;
// end of copy
		return Ct;
	}

	void LaserTicks(const CLaserData *pCurrent, int Type, bool IsPredicted, float *pTicks, float *pTicksHead)
	{
// C++ copy: game/client/components/items.cpp:313-329
	float Ticks;
	float TicksHead = Client()->GameTick(g_Config.m_ClDummy);
	if(Type == LASERTYPE_DOOR)
	{
		Ticks = 1.0f;
	}
	else if(IsPredicted)
	{
		int PredictionTick = Client()->GetPredictionTick();
		Ticks = (float)(PredictionTick - pCurrent->m_StartTick) + Client()->PredIntraGameTick(g_Config.m_ClDummy);
		TicksHead += Client()->PredIntraGameTick(g_Config.m_ClDummy);
	}
	else
	{
		Ticks = (float)(Client()->GameTick(g_Config.m_ClDummy) - pCurrent->m_StartTick) + Client()->IntraGameTick(g_Config.m_ClDummy);
		TicksHead += Client()->IntraGameTick(g_Config.m_ClDummy);
	}
// end of copy
		*pTicks = Ticks;
		*pTicksHead = TicksHead;
	}
};

// The item timings for every projectile and laser of a world.
static void ItemTimes(std::vector<int> &v, CGameClient &Gc, CGameWorld &World)
{
	CItems Items{&Gc};
	const bool LocalPlayerInGame = Gc.m_Client.m_PredGameTick % 3 != 0;
	const float LastGameTickTime = Gc.m_Client.m_PredIntraGameTick * 0.02f;
	for(CEntity *pEnt = World.FindFirst(CGameWorld::ENTTYPE_PROJECTILE); pEnt; pEnt = pEnt->TypeNext())
	{
		const CProjectileData Data = static_cast<CProjectile *>(pEnt)->GetData();
		v.push_back(Bits(Items.ProjectileTime(&Data, LocalPlayerInGame, LastGameTickTime)));
	}
	for(CEntity *pEnt = World.FindFirst(CGameWorld::ENTTYPE_LASER); pEnt; pEnt = pEnt->TypeNext())
	{
		const CLaserData Data = static_cast<CLaser *>(pEnt)->GetData();
		for(int Type : {std::clamp(Data.m_Type, -1, NUM_LASERTYPES - 1), (int)LASERTYPE_DOOR})
			for(bool IsPredicted : {false, true})
			{
				float Ticks, TicksHead;
				Items.LaserTicks(&Data, Type, IsPredicted, &Ticks, &TicksHead);
				v.push_back(Bits(Ticks));
				v.push_back(Bits(TicksHead));
			}
	}
}

static constexpr int RUNS = 12;
static constexpr int SNAPS = 60;
static constexpr int CHARS = 4;
static constexpr int START_TICK = 10000;

static void PrintInts(FILE *f, const void *pData, int Num)
{
	for(int k = 0; k < Num; k++)
		fprintf(f, " %d", ((const int *)pData)[k]);
}

// The snapshot type id the Rust client gives an object after unpacking: extended
// objects are numbered from 0x10000 in protocol order (ddnet/src/protocol/generated.rs).
static int RsType(int Type)
{
	switch(Type)
	{
	case NETOBJTYPE_DDNETCHARACTER: return 0x10001;
	case NETOBJTYPE_GAMEINFOEX: return 0x10003;
	case NETOBJTYPE_DDNETLASER: return 0x10005;
	case NETEVENTTYPE_MAPSOUNDWORLD: return 0x10011;
	}
	return Type;
}

// One snapshot item for the client: its type, id and ints.
struct SItem
{
	int m_Type;
	int m_Id;
	std::vector<int> m_vData;
};

// The timing functions over random game and predicted times, limits and percents.
static void TimingVectors(FILE *f)
{
	CClient Client;
	const int64_t TickTime = time_freq() / SERVER_TICK_SPEED;
	for(int i = 0; i < 3000; i++)
	{
		const int GameTick = 1 + Rand() % 100000;
		const int Lead = (int)(Rand() % 40) - 3;
		Client.m_GameTick = GameTick;
		Client.m_PredGameTick = GameTick + Lead;
		Client.m_aGameTime[0].m_Value = (GameTick - 1) * TickTime + Rand64() % TickTime;
		Client.m_PredictedTime.m_Value = Client.m_aGameTime[0].m_Value + (int64_t)Lead * TickTime + (int64_t)(Rand64() % (5 * TickTime)) - 2 * TickTime;
		g_Config.m_ClAntiPingLimit = Rand() % 3 == 0 ? Rand() % 501 : 0;
		g_Config.m_ClAntiPingPercent = Rand() % 3 == 0 ? Rand() % 101 : 100;
		const float Mix = Rand() % 5 == 0 ? (Rand() % 3) * 0.5f : (Rand() % 100001) / 100000.0f;
		int SmoothTick;
		float SmoothIntra;
		Client.GetSmoothTick(&SmoothTick, &SmoothIntra, Mix);
		fprintf(f, "timing %d %d %lld %lld %d %d %08x = %d %d %d %08x\n", Client.m_GameTick, Client.m_PredGameTick,
			(long long)Client.m_aGameTime[0].m_Value, (long long)Client.m_PredictedTime.m_Value,
			g_Config.m_ClAntiPingLimit, g_Config.m_ClAntiPingPercent, Bits(Mix),
			Client.GetPredictionTime(), Client.GetPredictionTick(), SmoothTick, Bits(SmoothIntra));
	}
}

// What the dump compares after UpdatePrediction and OnPredict.
// The fields CCharacterCore::ReadDDNet sets (after Reset, the rest is undefined).
static void DDNetFields(std::vector<int> &v, const CCharacterCore &c)
{
	for(bool b : {c.m_Solo, c.m_Jetpack, c.m_CollisionDisabled, c.m_HammerHitDisabled, c.m_ShotgunHitDisabled, c.m_GrenadeHitDisabled,
		    c.m_LaserHitDisabled, c.m_HookHitDisabled, c.m_Super, c.m_Invincible, c.m_EndlessHook, c.m_EndlessJump, c.m_DeepFrozen,
		    c.m_LiveFrozen, c.m_HasTelegunGrenade, c.m_HasTelegunGun, c.m_HasTelegunLaser})
		v.push_back(b);
	for(int w = 0; w < NUM_WEAPONS; w++)
		v.push_back(c.m_aWeapons[w].m_Got);
	v.push_back(c.m_FreezeEnd);
	v.push_back(c.m_Jumps);
	v.push_back(c.m_JumpedTotal);
}

// `aSeen`: the clients with a DDNetCharacter in a snapshot of this run.
static void PrintState(FILE *f, CGameClient &Gc, const bool *aSeen, bool Frame)
{
	std::vector<int> Cores;
	for(int i = 0; i < MAX_CLIENTS; i++)
	{
		const SClientData &c = Gc.m_aClients[i];
		Cores.push_back(Bits(c.m_PrevPredicted.m_Pos.x));
		Cores.push_back(Bits(c.m_PrevPredicted.m_Pos.y));
		Cores.push_back(Bits(c.m_Predicted.m_Pos.x));
		Cores.push_back(Bits(c.m_Predicted.m_Pos.y));
		if(Gc.m_PredictedWorld.GetCharacterById(i))
		{
			CoreStateInts(Cores, c.m_PrevPredicted);
			CoreStateInts(Cores, c.m_Predicted);
		}
		else if(aSeen[i])
			DDNetFields(Cores, c.m_Predicted);
	}
	std::vector<int> PredPos;
	for(const SClientData &c : Gc.m_aClients)
		for(int k = 0; k < 200; k++)
		{
			PredPos.push_back(Bits(c.m_aPredPos[k].x));
			PredPos.push_back(Bits(c.m_aPredPos[k].y));
			PredPos.push_back(c.m_aPredTick[k]);
		}
	std::vector<int> Smooth, Render, ItemTime;
	if(Frame)
	{
		ItemTimes(ItemTime, Gc, Gc.m_PrevPredictedWorld);
		ItemTimes(ItemTime, Gc, Gc.m_GameWorld);
	}
	if(Frame)
		Gc.UpdateRenderedCharacters();
	for(int i = 0; i < MAX_CLIENTS; i++)
	{
		const SClientData &c = Gc.m_aClients[i];
		for(int j = 0; j < 2; j++)
		{
			Smooth.push_back((int)c.m_aSmoothStart[j]);
			Smooth.push_back((int)(c.m_aSmoothStart[j] >> 32));
			Smooth.push_back((int)c.m_aSmoothLen[j]);
			Smooth.push_back((int)(c.m_aSmoothLen[j] >> 32));
		}
		Smooth.push_back(Bits(Gc.m_aLastPos[i].x));
		Smooth.push_back(Bits(Gc.m_aLastPos[i].y));
		Smooth.push_back(Gc.m_aLastActive[i]);
		if(Frame)
		{
			const vec2 Pos = Gc.GetSmoothPos(i);
			Smooth.push_back(Bits(Pos.x));
			Smooth.push_back(Bits(Pos.y));
		}
		if(Frame && Gc.m_Snap.m_aCharacters[i].m_Active)
		{
			// UpdateRenderedCharacters
			Render.push_back(i);
			Render.insert(Render.end(), (const int *)&c.m_RenderPrev, (const int *)(&c.m_RenderPrev + 1));
			Render.insert(Render.end(), (const int *)&c.m_RenderCur, (const int *)(&c.m_RenderCur + 1));
			Render.push_back(c.m_IsPredicted);
			Render.push_back(c.m_IsPredictedLocal);
			Render.push_back(Bits(c.m_RenderPos.x));
			Render.push_back(Bits(c.m_RenderPos.y));
		}
	}
	std::vector<int> World, Predicted, PrevPredicted;
	NetWorldInts(World, Gc.m_GameWorld);
	NetWorldInts(Predicted, Gc.m_PredictedWorld);
	NetWorldInts(PrevPredicted, Gc.m_PrevPredictedWorld);
	if(getenv("RS_VERBOSE"))
		fprintf(stderr, "--\n");
	if(getenv("RS_VERBOSE"))
		for(int i = 0; i < MAX_CLIENTS; i++)
			if(CCharacter *pChar = Gc.m_GameWorld.GetCharacterById(i))
			{
				std::vector<int> v;
				CharStateInts(v, pChar);
				fprintf(stderr, "# %d", i);
				for(int x : v)
					fprintf(stderr, " %d", x);
				fprintf(stderr, "\n");
			}
	// m_PredictedChar is left out: only the ghost reads it (not ported)
	fprintf(f, " = %08x %08x %08x %08x %08x %08x %d %08x %08x", Crc(Cores), Crc(PredPos), Crc(Smooth), Crc(World), Crc(Predicted),
		Crc(PrevPredicted), Gc.m_PredictedTick, Crc(Render), Crc(ItemTime));
	for(const std::string &Sound : s_vSounds)
		fprintf(f, " %s", Sound.c_str());
	fprintf(f, "\n");
	s_vSounds.clear();
}

// C++ copy: game/client/gameclient.cpp:1404-1490
void CGameClient::ProcessEvents()
{
	if(m_SuppressEvents)
		return;

	int SnapType = IClient::SNAP_CURRENT;
	int Num = Client()->SnapNumItems(SnapType);
	for(int Index = 0; Index < Num; Index++)
	{
		const IClient::CSnapItem Item = Client()->SnapGetItem(SnapType, Index);

		// TODO: We don't have enough info about us, others, to know a correct alpha or volume value.
		const float Alpha = 1.0f;
		const float Volume = 1.0f;

		if(Item.m_Type == NETEVENTTYPE_DAMAGEIND)
		{
			const CNetEvent_DamageInd *pEvent = (const CNetEvent_DamageInd *)Item.m_pData;

			vec2 DamageIndPos = vec2(pEvent->m_X, pEvent->m_Y);
			if(!m_PredictedWorld.CheckPredictedEventHandled(CGameWorld::CPredictedEvent(Item.m_Type, DamageIndPos, -1, Client()->GameTick(g_Config.m_ClDummy), pEvent->m_Angle)))
			{
				m_Effects.DamageIndicator(vec2(pEvent->m_X, pEvent->m_Y), direction(pEvent->m_Angle / 256.0f), Alpha);
			}
		}
		else if(Item.m_Type == NETEVENTTYPE_EXPLOSION)
		{
			const CNetEvent_Explosion *pEvent = (const CNetEvent_Explosion *)Item.m_pData;

			vec2 ExplosionPos = vec2(pEvent->m_X, pEvent->m_Y);
			if(!m_PredictedWorld.CheckPredictedEventHandled(CGameWorld::CPredictedEvent(Item.m_Type, ExplosionPos, -1, Client()->GameTick(g_Config.m_ClDummy))))
			{
				m_Effects.Explosion(ExplosionPos, Alpha);
			}
		}
		else if(Item.m_Type == NETEVENTTYPE_HAMMERHIT)
		{
			const CNetEvent_HammerHit *pEvent = (const CNetEvent_HammerHit *)Item.m_pData;

			vec2 HammerHitPos = vec2(pEvent->m_X, pEvent->m_Y);
			if(!m_PredictedWorld.CheckPredictedEventHandled(CGameWorld::CPredictedEvent(Item.m_Type, HammerHitPos, -1, Client()->GameTick(g_Config.m_ClDummy))))
			{
				m_Effects.HammerHit(HammerHitPos, Alpha, Volume);
			}
		}
		else if(Item.m_Type == NETEVENTTYPE_BIRTHDAY)
		{
			const CNetEvent_Birthday *pEvent = (const CNetEvent_Birthday *)Item.m_pData;
			m_Effects.Confetti(vec2(pEvent->m_X, pEvent->m_Y), Alpha);
		}
		else if(Item.m_Type == NETEVENTTYPE_FINISH)
		{
			const CNetEvent_Finish *pEvent = (const CNetEvent_Finish *)Item.m_pData;
			m_Effects.Confetti(vec2(pEvent->m_X, pEvent->m_Y), Alpha);
		}
		else if(Item.m_Type == NETEVENTTYPE_SPAWN)
		{
			const CNetEvent_Spawn *pEvent = (const CNetEvent_Spawn *)Item.m_pData;
			m_Effects.PlayerSpawn(vec2(pEvent->m_X, pEvent->m_Y), Alpha, Volume);
		}
		else if(Item.m_Type == NETEVENTTYPE_DEATH)
		{
			const CNetEvent_Death *pEvent = (const CNetEvent_Death *)Item.m_pData;
			m_Effects.PlayerDeath(vec2(pEvent->m_X, pEvent->m_Y), pEvent->m_ClientId, Alpha);
		}
		else if(Item.m_Type == NETEVENTTYPE_SOUNDWORLD)
		{
			const CNetEvent_SoundWorld *pEvent = (const CNetEvent_SoundWorld *)Item.m_pData;
			if(!Config()->m_SndGame)
				continue;

			if(m_GameInfo.m_RaceSounds && ((pEvent->m_SoundId == SOUND_GUN_FIRE && !g_Config.m_SndGun) || (pEvent->m_SoundId == SOUND_PLAYER_PAIN_LONG && !g_Config.m_SndLongPain)))
				continue;

			vec2 SoundPos = vec2(pEvent->m_X, pEvent->m_Y);
			if(!m_PredictedWorld.CheckPredictedEventHandled(CGameWorld::CPredictedEvent(Item.m_Type, SoundPos, -1, Client()->GameTick(g_Config.m_ClDummy), pEvent->m_SoundId)))
			{
				m_Sounds.PlayAt(CSounds::CHN_WORLD, pEvent->m_SoundId, 1.0f, SoundPos);
			}
		}
		else if(Item.m_Type == NETEVENTTYPE_MAPSOUNDWORLD)
		{
			CNetEvent_MapSoundWorld *pEvent = (CNetEvent_MapSoundWorld *)Item.m_pData;
			m_MapSounds.PlayAt(CSounds::CHN_WORLD, pEvent->m_SoundId, vec2(pEvent->m_X, pEvent->m_Y));
		}
	}
}
// end of copy

// OnNewSnapshot's air jumps of other players
void CGameClient::DetectOtherAirJumps()
{
// C++ copy: game/client/gameclient.cpp:2405-2430
	// detect air jump for other players
	for(int i = 0; i < MAX_CLIENTS; i++)
	{
		if(m_Snap.m_aCharacters[i].m_Active && (m_Snap.m_aCharacters[i].m_Cur.m_Jumped & 2) && !(m_Snap.m_aCharacters[i].m_Prev.m_Jumped & 2))
		{
			bool IsDummy = Client()->DummyConnected() && i == m_aLocalIds[!g_Config.m_ClDummy];
			bool IsLocalPlayer = i == m_Snap.m_LocalClientId;

			if(!Predict() || (!IsLocalPlayer && !AntiPingPlayers()) || (!IsLocalPlayer && !IsDummy))
			{
				vec2 Pos = mix(vec2(m_Snap.m_aCharacters[i].m_Prev.m_X, m_Snap.m_aCharacters[i].m_Prev.m_Y),
					vec2(m_Snap.m_aCharacters[i].m_Cur.m_X, m_Snap.m_aCharacters[i].m_Cur.m_Y),
					Client()->IntraGameTick(g_Config.m_ClDummy));
				float Alpha = 1.0f;
				if(IsOtherTeam(i))
					Alpha = g_Config.m_ClShowOthersAlpha / 100.0f;
				const float Volume = 1.0f; // TODO snd_game_volume_others

				const bool Grounded = Collision()->IsOnGround(vec2(m_Snap.m_aCharacters[i].m_Prev.m_X, m_Snap.m_aCharacters[i].m_Prev.m_Y), CCharacterCore::PhysicalSize());
				if(!Grounded)
				{
					m_Effects.AirJump(Pos, Alpha, Volume);
				}
			}
		}
	}
// end of copy
}

// C++ copy: game/client/gameclient.cpp:3819-3865
void CGameClient::HandlePredictedEvents(const int Tick)
{
	const float Alpha = 1.0f;
	const float Volume = 1.0f;

	auto EventsIterator = m_PredictedWorld.m_PredictedEvents.begin();
	while(EventsIterator != m_PredictedWorld.m_PredictedEvents.end())
	{
		if(!EventsIterator->m_Handled && EventsIterator->m_Tick <= Tick)
		{
			if(EventsIterator->m_EventId == NETEVENTTYPE_SOUNDWORLD)
			{
				if(m_GameInfo.m_RaceSounds && ((EventsIterator->m_ExtraInfo == SOUND_GUN_FIRE && !g_Config.m_SndGun) || (EventsIterator->m_ExtraInfo == SOUND_PLAYER_PAIN_LONG && !g_Config.m_SndLongPain)))
				{
					EventsIterator = m_PredictedWorld.m_PredictedEvents.erase(EventsIterator);
					continue;
				}
				m_Sounds.PlayAt(CSounds::CHN_WORLD, EventsIterator->m_ExtraInfo, 1.0f, EventsIterator->m_Pos);
			}
			else if(EventsIterator->m_EventId == NETEVENTTYPE_EXPLOSION)
			{
				m_Effects.Explosion(EventsIterator->m_Pos, Alpha);
			}
			else if(EventsIterator->m_EventId == NETEVENTTYPE_HAMMERHIT)
			{
				m_Effects.HammerHit(EventsIterator->m_Pos, Alpha, Volume);
			}
			else if(EventsIterator->m_EventId == NETEVENTTYPE_DAMAGEIND)
			{
				m_Effects.DamageIndicator(EventsIterator->m_Pos, direction(EventsIterator->m_ExtraInfo / 256.0f), Alpha);
			}

			EventsIterator->m_Handled = true;
			++EventsIterator;
			continue;
		}
		else if(Tick - EventsIterator->m_Tick > 3 * Client()->GameTickSpeed()) // 3 seconds
		{
			// remove too old events
			EventsIterator = m_PredictedWorld.m_PredictedEvents.erase(EventsIterator);
		}
		else
		{
			++EventsIterator;
		}
	}
}
// end of copy

// The tee inputs: each tee's input per tick, made in tick order like rs_parity netents.
struct SInputs
{
	std::map<int, CNetObj_PlayerInput> m_Ticks;
	CNetObj_PlayerInput m_Cur = {};
	int m_aHold[4] = {};
	int m_Last = START_TICK;

	const CNetObj_PlayerInput &At(int Tick)
	{
		while(m_Last < Tick)
		{
			CNetObj_PlayerInput &In = m_Cur;
			if(--m_aHold[0] <= 0)
			{
				In.m_Direction = (int)(Rand() % 3) - 1;
				m_aHold[0] = 1 + Rand() % 30;
			}
			if(--m_aHold[1] <= 0)
			{
				In.m_Hook = Rand() % 5 == 0;
				m_aHold[1] = 1 + Rand() % 40;
				const float Angle = (Rand() % 360) * (pi / 180.0f);
				const int Length = 10 + Rand() % 300;
				In.m_TargetX = (int)(std::cos(Angle) * Length);
				In.m_TargetY = (int)(std::sin(Angle) * Length);
			}
			if(--m_aHold[2] <= 0)
			{
				In.m_Jump = Rand() % 5 == 0;
				m_aHold[2] = 1 + Rand() % 15;
			}
			if(--m_aHold[3] <= 0)
			{
				if((In.m_Fire & 1) != (int)(Rand() % 2))
					In.m_Fire = (In.m_Fire + 1) & INPUT_STATE_MASK;
				m_aHold[3] = 1 + Rand() % 6;
			}
			const int Extra = Rand() % 100;
			In.m_WantedWeapon = Extra < 6 ? 1 + Extra % 5 : 0;
			In.m_PlayerFlags = PLAYERFLAG_PLAYING;
			m_Ticks[++m_Last] = In;
		}
		return m_Ticks[Tick];
	}
};

static int LoopMode(CCollision *pCollision, FILE *f)
{
	const int Width = pCollision->GetWidth(), Height = pCollision->GetHeight();
	g_Config.m_SvFreezeDelay = 3;
	g_Config.m_SvDeepfly = 1;
	g_Config.m_SvHit = 1;
	g_Config.m_SvDraggerRange = 700;
	g_Config.m_SvOldTeleportHook = 0;
	g_Config.m_ClDummy = 0;
	g_Config.m_ClPredictDummy = 1;
	g_Config.m_SndGame = 1;
	g_Config.m_SndEnable = 1;
	g_Config.m_Debug = 0;
	g_Config.m_ClPredict = 1;

	static CTuningParams s_aTuning[TuneZone::NUM];
	const int aIds[CHARS] = {3, 0, 6, 1};
	const int aSorted[CHARS] = {0, 1, 3, 6};
	const int Local = aIds[0];
	const auto Free = [&](int x, int y) {
		const int t = pCollision->GetTileIndex(std::clamp(y, 0, Height - 1) * Width + std::clamp(x, 0, Width - 1));
		return t != TILE_SOLID && t != TILE_NOHOOK;
	};
	const int64_t TickTime = time_freq() / SERVER_TICK_SPEED;
	for(int Run = 0; Run < RUNS; Run++)
	{
		int Index = 0;
		for(int Try = 0; Try < 300; Try++)
		{
			Index = Rand() % (Width * Height);
			const int x = Index % Width, y = Index / Width;
			if(Free(x - 2, y) && Free(x - 1, y) && Free(x, y) && Free(x + 1, y) && Free(x + 2, y))
				break;
		}
		// the client settings of this run
		g_Config.m_ClAntiPing = Run % 4 != 0;
		g_Config.m_ClAntiPingPlayers = (Run / 4) % 4;
		g_Config.m_ClAntiPingWeapons = Run % 3 != 2;
		g_Config.m_ClAntiPingGrenade = 1;
		g_Config.m_ClAntiPingGunfire = 1;
		g_Config.m_ClAntiPingPreInput = Run % 5 != 4;
		g_Config.m_ClAntiPingSmooth = Run % 2;
		g_Config.m_ClAntiPingLimit = Run % 6 == 1 ? 20 + Rand() % 200 : 0;
		g_Config.m_ClAntiPingPercent = Run % 6 == 2 ? Rand() % 101 : 100;
		g_Config.m_ClPredictFreeze = Run % 3;
		g_Config.m_SndGun = Run % 5 != 1;
		g_Config.m_SndLongPain = Run % 7 != 2;
		g_Config.m_ClPredictEvents = Run % 8 != 7;
		const bool Vanilla = Run % 4 == 3;
		const bool TeamSplit = Run % 5 == 2 || Run % 5 == 4;
		const int SoloId = Run % 5 == 3 ? aIds[1] : -1;
		CNetObj_GameInfoEx InfoEx = {};
		InfoEx.m_Version = 12;
		InfoEx.m_Flags = Vanilla ? GAMEINFOFLAG_GAMETYPE_VANILLA | GAMEINFOFLAG_PREDICT_VANILLA :
					   GAMEINFOFLAG_GAMETYPE_DDNET | GAMEINFOFLAG_PREDICT_DDRACE | GAMEINFOFLAG_PREDICT_DDRACE_TILES;
		InfoEx.m_Flags2 = (Run % 7 == 3 ? GAMEINFOFLAG2_OLD_LASER : 0) | (Run % 7 == 5 ? GAMEINFOFLAG2_NO_WEAK_HOOK : 0) |
				  (Run % 6 != 4 ? GAMEINFOFLAG2_PREDICT_EVENTS : 0);
		fprintf(f, "run %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d\n", Run, Index, g_Config.m_ClAntiPing, g_Config.m_ClAntiPingPlayers,
			g_Config.m_ClAntiPingWeapons, g_Config.m_ClAntiPingPreInput, g_Config.m_ClAntiPingSmooth, g_Config.m_ClAntiPingLimit,
			g_Config.m_ClAntiPingPercent, g_Config.m_ClPredictFreeze, g_Config.m_ClAntiPingGrenade, g_Config.m_ClAntiPingGunfire,
			g_Config.m_SndGun, g_Config.m_SndLongPain, g_Config.m_ClPredictEvents);

		// the server
		CGameWorld Server;
		Server.Init(pCollision, s_aTuning, nullptr);
		Server.m_Core.InitSwitchers(pCollision->m_HighestSwitchNumber);
		Server.m_GameTick = START_TICK;
		auto &Config = Server.m_WorldConfig;
		Config.m_IsDDRace = !Vanilla;
		Config.m_IsVanilla = Vanilla;
		Config.m_InfiniteAmmo = true;
		Config.m_PredictTiles = !Vanilla;
		Config.m_PredictFreeze = 1;
		Config.m_PredictWeapons = true;
		Config.m_PredictDDRace = !Vanilla;
		Config.m_UseTuneZones = !Vanilla;
		Config.m_PredictEvents = true; // its events go into the snapshots
		CTeamsCore Teams;
		Teams.m_NumDDRaceTeams = NUM_DDRACE_TEAMS;
		if(TeamSplit)
			Teams.Team(aIds[2], 1);
		Server.m_Teams = Teams;
		const vec2 Spot = vec2((Index % Width) * 32 + 16, (Index / Width) * 32 + 16);
		CNetObj_DDNetCharacter aExt[MAX_CLIENTS] = {};
		for(int i = 0; i < CHARS; i++)
		{
			CNetObj_Character Obj = {};
			Obj.m_X = (int)Spot.x + i * 24 - 36;
			Obj.m_Y = (int)Spot.y;
			Obj.m_HookX = Obj.m_X;
			Obj.m_HookY = Obj.m_Y;
			Obj.m_HookedPlayer = -1;
			Obj.m_Weapon = 1 + (i + Run) % 4;
			Obj.m_AmmoCount = 10;
			Obj.m_AttackTick = START_TICK - 30;
			CNetObj_DDNetCharacter &Ext = aExt[aIds[i]];
			Ext.m_Flags = CHARACTERFLAG_WEAPON_HAMMER | CHARACTERFLAG_WEAPON_GUN | CHARACTERFLAG_WEAPON_SHOTGUN |
				      CHARACTERFLAG_WEAPON_GRENADE | CHARACTERFLAG_WEAPON_LASER | (aIds[i] == SoloId ? CHARACTERFLAG_SOLO : 0);
			Ext.m_Jumps = 2;
			Ext.m_JumpedTotal = -1;
			Ext.m_NinjaActivationTick = -1;
			Ext.m_FreezeStart = -1;
			Ext.m_StrongWeakId = i;
			Ext.m_TuneZoneOverride = TuneZone::OVERRIDE_NONE;
			CCharacter *pChar = new CCharacter(&Server, aIds[i], &Obj, &Ext);
			Server.InsertEntity(pChar);
			pChar->m_GameTeam = aIds[i];
		}

		// the client
		std::unique_ptr<CGameClient> pGc = std::make_unique<CGameClient>();
		CGameClient &Gc = *pGc;
		Gc.m_pCollision = pCollision;
		Gc.m_GameWorld.Init(pCollision, s_aTuning, nullptr);
		Gc.m_GameWorld.m_Core.InitSwitchers(pCollision->m_HighestSwitchNumber);
		Gc.m_GameWorld.m_WorldConfig.m_InfiniteAmmo = true;
		Gc.m_GameWorld.m_WorldConfig.m_PredictFreeze = 1;
		Gc.m_GameWorld.m_WorldConfig.m_IsSolo = false;
		Gc.m_PredictedWorld.Init(pCollision, s_aTuning, nullptr);
		Gc.m_PrevPredictedWorld.Init(pCollision, s_aTuning, nullptr);
		// OnNewSnapshot sets m_GameInfo after ProcessEvents: a fresh client's before that
		const CGameInfo Info = GetGameInfo(&InfoEx, sizeof(InfoEx), nullptr);
		Gc.m_Teams.m_NumDDRaceTeams = Info.m_NumDDRaceTeams;
		Gc.m_GameWorld.m_WorldConfig.m_UseTuneZones = Info.m_PredictDDRaceTiles;
		Gc.m_Snap.m_LocalClientId = Local;
		Gc.m_aLocalIds[0] = Local;
		if(TeamSplit)
		{
			// Sv_TeamsState
			fprintf(f, "teams");
			for(int i = 0; i < MAX_CLIENTS; i++)
			{
				Gc.m_Teams.Team(i, Teams.Team(i));
				fprintf(f, " %d", Teams.Team(i));
			}
			fprintf(f, "\n");
		}

		SInputs aInputs[MAX_CLIENTS];
		std::map<const CEntity *, int> EntIds;
		int NextEntId = 100;
		CNetObj_Character aPrevObj[MAX_CLIENTS] = {};
		bool aPrevIn[MAX_CLIENTS] = {};
		bool aSeen[MAX_CLIENTS] = {};
		int PrevSnapTick = 0;
		int Latency = 3 + Rand() % 4;
		int SentUpTo = START_TICK;
		CNetObj_GameInfo GameInfo = {};
		int64_t RunTime = (int64_t)(START_TICK + 100) * TickTime;
		for(int Snap = 0; Snap < SNAPS; Snap++)
		{
			// the server ticks: the local tee on the inputs sent, the others on theirs
			// once in some runs a long gap: the server world skips to the snapshot tick, and
			// the preinput ring wraps
			const int Ticks = (Run % 3 == 1 && Snap == 30) ? 150 : 1 + Rand() % 3;
			const int ClPredictEvents = g_Config.m_ClPredictEvents;
			g_Config.m_ClPredictEvents = 1; // the server's events
			for(int t = 0; t < Ticks; t++)
			{
				const int Tick = Server.GameTick() + 1;
				CNetObj_PlayerInput aIn[MAX_CLIENTS] = {};
				for(int Id : aSorted)
				{
					if(Id == Local)
					{
						if(const int *pIn = Gc.Client()->GetInput(Tick))
							mem_copy(&aIn[Id], pIn, sizeof(aIn[Id]));
					}
					else
						aIn[Id] = aInputs[Id].At(Tick);
				}
				for(int Id : aSorted)
					if(CCharacter *pChar = Server.GetCharacterById(Id))
						pChar->OnDirectInput(&aIn[Id]);
				Server.m_GameTick++;
				for(int Id : aSorted)
					if(CCharacter *pChar = Server.GetCharacterById(Id))
						pChar->OnPredictedInput(&aIn[Id]);
				Server.Tick();
			}
			g_Config.m_ClPredictEvents = ClPredictEvents;
			const int SnapTick = Server.GameTick();
			Latency = std::clamp(Latency + (int)(Rand() % 3) - 1, 2, 9);

			// the other tees' preinputs, some lost
			for(int Id : aSorted)
			{
				if(Id == Local)
					continue;
				for(int Tick = SnapTick + 1; Tick <= SnapTick + Latency + 1; Tick++)
				{
					if(Rand() % 10 < 3)
						continue;
					const CNetObj_PlayerInput &In = aInputs[Id].At(Tick);
					CNetMsg_Sv_PreInput Pre = {};
					// sometimes wrong at first, corrected by a later snapshot's copy
					Pre.m_Direction = (Tick > SnapTick + 2 && Rand() % 7 == 0) ? -In.m_Direction : In.m_Direction;
					Pre.m_TargetX = In.m_TargetX;
					Pre.m_TargetY = In.m_TargetY;
					Pre.m_Jump = In.m_Jump;
					Pre.m_Fire = In.m_Fire;
					Pre.m_Hook = In.m_Hook;
					Pre.m_WantedWeapon = In.m_WantedWeapon;
					Pre.m_NextWeapon = In.m_NextWeapon;
					Pre.m_PrevWeapon = In.m_PrevWeapon;
					Pre.m_Owner = Id;
					Pre.m_IntendedTick = Tick;
					Gc.m_aClients[Id].m_aPreInputs[Tick % 200] = Pre;
					fprintf(f, "pre %d %d %d %d %d %d %d %d %d %d %d\n", Pre.m_Direction, Pre.m_TargetX, Pre.m_TargetY, Pre.m_Jump,
						Pre.m_Fire, Pre.m_Hook, Pre.m_WantedWeapon, Pre.m_NextWeapon, Pre.m_PrevWeapon, Pre.m_Owner, Pre.m_IntendedTick);
				}
			}

			// the snapshot: game info, the local player, characters, projectiles, lasers
			GameInfo.m_GameStateFlags = (Run % 8 == 5 && Snap >= 30 && Snap < 34) ? GAMESTATEFLAG_PAUSED :
										(Run % 8 == 6 && Snap >= 40 && Snap < 42) ? GAMESTATEFLAG_GAMEOVER : 0;
			std::vector<SItem> vItems;
			vItems.push_back({NETOBJTYPE_GAMEINFO, 0, std::vector<int>((const int *)&GameInfo, (const int *)(&GameInfo + 1))});
			vItems.push_back({NETOBJTYPE_GAMEINFOEX, 0, std::vector<int>((const int *)&InfoEx, (const int *)(&InfoEx + 1))});
			CNetObj_PlayerInfo PlayerInfo = {};
			PlayerInfo.m_Local = 1;
			PlayerInfo.m_ClientId = Local;
			vItems.push_back({NETOBJTYPE_PLAYERINFO, Local, std::vector<int>((const int *)&PlayerInfo, (const int *)(&PlayerInfo + 1))});
			// frozen tees for a while (the local one too: cl_predict_freeze 2)
			for(int Id : aSorted)
			{
				const bool Frozen = (Snap >= 10 && Snap < 15) || (Snap >= 48 && Snap < 52 && Id != aIds[1]);
				aExt[Id].m_FreezeStart = Frozen ? SnapTick - 1 : -1;
				aExt[Id].m_FreezeEnd = Frozen ? (Rand() % 8 == 0 ? -1 : SnapTick + 2 + Rand() % 20) : 0;
			}
			CNetObj_Character aObj[MAX_CLIENTS] = {};
			bool aIn[MAX_CLIENTS] = {};
			for(int i = 0; i < CHARS; i++)
			{
				const int Id = aIds[i];
				CCharacter *pChar = Server.GetCharacterById(Id);
				if(!pChar || (i == 2 && Snap % 12 >= 9))
					continue;
				CNetObj_Character &Obj = aObj[Id];
				pChar->GetCore().Write((CNetObj_CharacterCore *)&Obj);
				Obj.m_Weapon = pChar->GetActiveWeapon();
				Obj.m_AttackTick = pChar->GetAttackTick();
				Obj.m_Tick = SnapTick;
				Obj.m_AmmoCount = 10;
				aIn[Id] = true;
				aSeen[Id] = true;
				vItems.push_back({NETOBJTYPE_CHARACTER, Id, std::vector<int>((const int *)&Obj, (const int *)(&Obj + 1))});
				vItems.push_back({NETOBJTYPE_DDNETCHARACTER, Id, std::vector<int>((const int *)&aExt[Id], (const int *)(&aExt[Id] + 1))});
			}
			std::map<const CEntity *, int> SeenIds;
			std::vector<SItem> vEntities;
			const auto EntId = [&](const CEntity *pEnt) {
				auto It = EntIds.find(pEnt);
				const int Id = It != EntIds.end() ? It->second : NextEntId++;
				SeenIds[pEnt] = Id;
				return Id;
			};
			for(CEntity *pEnt = Server.FindFirst(CGameWorld::ENTTYPE_PROJECTILE); pEnt; pEnt = pEnt->TypeNext())
			{
				const CProjectileData Data = static_cast<CProjectile *>(pEnt)->GetData();
				vEntities.push_back({NETOBJTYPE_PROJECTILE, EntId(pEnt), {(int)Data.m_StartPos.x, (int)Data.m_StartPos.y, (int)(Data.m_StartVel.x * 100.0f), (int)(Data.m_StartVel.y * 100.0f), Data.m_Type, Data.m_StartTick}});
			}
			for(CEntity *pEnt = Server.FindFirst(CGameWorld::ENTTYPE_LASER); pEnt; pEnt = pEnt->TypeNext())
			{
				const CLaserData Data = static_cast<CLaser *>(pEnt)->GetData();
				vEntities.push_back({NETOBJTYPE_DDNETLASER, EntId(pEnt), {(int)Data.m_To.x, (int)Data.m_To.y, (int)Data.m_From.x, (int)Data.m_From.y, Data.m_StartTick, Data.m_Owner, Data.m_Type, -1, -1, 0}});
			}
			EntIds = SeenIds;
			std::stable_sort(vEntities.begin(), vEntities.end(), [](const SItem &a, const SItem &b) { return a.m_Id < b.m_Id; });
			vItems.insert(vItems.end(), vEntities.begin(), vEntities.end());
			// the server's events since the last snapshot (as CGameContext::Create*: int
			// positions), sometimes a spawn or a map sound
			std::vector<SItem> vEvents;
			for(const CGameWorld::CPredictedEvent &Event : Server.m_PredictedEvents)
			{
				std::vector<int> vData = {(int)Event.m_Pos.x, (int)Event.m_Pos.y};
				if(Event.m_EventId == NETEVENTTYPE_SOUNDWORLD || Event.m_EventId == NETEVENTTYPE_DAMAGEIND)
					vData.push_back(Event.m_ExtraInfo);
				vEvents.push_back({Event.m_EventId, (int)vEvents.size(), vData});
			}
			Server.m_PredictedEvents.clear();
			if(Rand() % 10 == 0)
				vEvents.push_back({NETEVENTTYPE_SPAWN, (int)vEvents.size(), {(int)Spot.x + (int)(Rand() % 200), (int)Spot.y}});
			if(Rand() % 10 == 0)
				vEvents.push_back({NETEVENTTYPE_MAPSOUNDWORLD, (int)vEvents.size(), {(int)Spot.x, (int)Spot.y - (int)(Rand() % 200), (int)(Rand() % 4)}});
			vItems.insert(vItems.end(), vEvents.begin(), vEvents.end());

			fprintf(f, "snap %d\n", SnapTick);
			for(const SItem &It : vItems)
			{
				fprintf(f, "item %d %d", RsType(It.m_Type), It.m_Id);
				PrintInts(f, It.m_vData.data(), (int)It.m_vData.size());
				fprintf(f, "\n");
			}

			// C++ OnNewSnapshot (CClient::Update has switched the ticks): ProcessEvents first
			Gc.m_Client.m_PrevGameTick = PrevSnapTick;
			Gc.m_Client.m_GameTick = SnapTick;
			Gc.m_Client.m_vSnapItems.clear();
			for(const SItem &It : vEvents)
				Gc.m_Client.m_vSnapItems.push_back({It.m_Type, It.m_Id, It.m_vData.data(), (int)It.m_vData.size() * 4});
			Gc.ProcessEvents();
			Gc.m_GameInfo = Info;

			// what UpdatePrediction reads
			static CNetObj_GameInfo s_GameInfo;
			s_GameInfo = GameInfo;
			Gc.m_Snap.m_pGameInfoObj = &s_GameInfo;
			Gc.m_Snap.m_pLocalCharacter = nullptr;
			Gc.m_Snap.m_pLocalPrevCharacter = nullptr;
			for(int Id = 0; Id < MAX_CLIENTS; Id++)
			{
				SCharacterSnap &c = Gc.m_Snap.m_aCharacters[Id];
				c = SCharacterSnap();
				if(!aIn[Id])
					continue;
				// every DDNetCharacter (gameclient.cpp:1887)
				c.m_HasExtendedData = true;
				c.m_ExtendedData = aExt[Id];
				Gc.m_aClients[Id].m_Solo = aExt[Id].m_Flags & CHARACTERFLAG_SOLO;
				Gc.m_aClients[Id].m_Predicted.ReadDDNet(&aExt[Id]);
				Gc.m_Teams.SetSolo(Id, Gc.m_aClients[Id].m_Solo);
				if(!aPrevIn[Id])
					continue;
				c.m_Active = true;
				c.m_Cur = aObj[Id];
				c.m_Prev = aPrevObj[Id];
				if(Id == Local)
				{
					Gc.m_Snap.m_pLocalCharacter = &c.m_Cur;
					Gc.m_Snap.m_pLocalPrevCharacter = &c.m_Prev;
				}
			}
			std::vector<std::vector<int>> vEntData;
			Gc.m_vSnapEntities.clear();
			for(SItem &It : vEntities)
				It.m_vData.resize(16, 0);
			for(const SItem &It : vEntities)
			{
				CSnapEntities Ent;
				Ent.m_Item.m_Type = It.m_Type;
				Ent.m_Item.m_Id = It.m_Id;
				Ent.m_Item.m_pData = It.m_vData.data();
				Ent.m_Item.m_DataSize = (int)It.m_vData.size() * 4;
				Ent.m_pDataEx = nullptr;
				Gc.m_vSnapEntities.push_back(Ent);
			}
			Gc.m_Client.m_PrevGameTick = PrevSnapTick;
			Gc.m_Client.m_GameTick = SnapTick;
			Gc.m_NewTick = true;
			Gc.DetectOtherAirJumps();
			Gc.UpdatePrediction();
			fprintf(f, "update");
			PrintState(f, Gc, aSeen, false);
			mem_copy(aPrevObj, aObj, sizeof(aObj));
			mem_copy(aPrevIn, aIn, sizeof(aIn));
			PrevSnapTick = SnapTick;

			// a Sv_TeamsState between the snapshot and the prediction: the split tee joins
			// the local team for a while (its projectiles enter the world), then leaves
			if(TeamSplit && Snap % 10 == 5)
			{
				Teams.Team(aIds[2], (Snap / 10) % 2 == 0 ? 0 : 1);
				Server.m_Teams = Teams;
				fprintf(f, "teams");
				for(int i = 0; i < MAX_CLIENTS; i++)
				{
					Gc.m_Teams.Team(i, Teams.Team(i));
					fprintf(f, " %d", Teams.Team(i));
				}
				fprintf(f, "\n");
			}

			// frames until the next snapshot: predict ahead
			const int Frames = 1 + Rand() % 3;
			for(int Frame = 0; Frame < Frames; Frame++)
			{
				const int PredTick = SnapTick + Latency + Frame;
				for(int Tick = SentUpTo + 1; Tick <= PredTick; Tick++)
				{
					const CNetObj_PlayerInput &In = aInputs[Local].At(Tick);
					Gc.m_Client.m_Inputs[Tick] = In;
					fprintf(f, "input %d", Tick);
					PrintInts(f, &In, sizeof(In) / sizeof(int));
					fprintf(f, "\n");
				}
				SentUpTo = std::max(SentUpTo, PredTick);
				// some frames come fast (high frame rates: smoothing windows last ~1 ms)
				RunTime += Rand() % 3 == 0 ? 1 + Rand64() % (TickTime / 10) : TickTime / 3 + Rand64() % TickTime;
				s_Now = RunTime;
				CClient &Client = Gc.m_Client;
				Client.m_PredGameTick = PredTick;
				Client.m_aGameTime[0].m_Value = (SnapTick - 1) * TickTime + Rand64() % TickTime;
				Client.m_PredictedTime.m_Value = (PredTick - 1) * TickTime + Rand64() % TickTime;
				Client.m_IntraGameTick = (Rand() % 1001) / 1000.0f;
				Client.m_PredIntraGameTick = (Rand() % 1001) / 1000.0f;
				fprintf(f, "frame %d %d %d %lld %lld %lld %08x %08x %d", Client.m_GameTick, Client.m_PrevGameTick, Client.m_PredGameTick,
					(long long)Client.m_aGameTime[0].m_Value, (long long)Client.m_PredictedTime.m_Value, (long long)s_Now,
					Bits(Client.m_IntraGameTick), Bits(Client.m_PredIntraGameTick), Gc.m_NewTick);
				// CClient::Update: only sane predictions
				const int MaxLatencyTicks = SERVER_TICK_SPEED + (10 * SERVER_TICK_SPEED) / 1000;
				if(PredTick > SnapTick && PredTick < SnapTick + MaxLatencyTicks)
					Gc.OnPredict();
				PrintState(f, Gc, aSeen, true);
				Gc.m_NewTick = false;
			}
		}
	}
	return 0;
}

} // namespace rs_antiping

int AntiPingMode(CCollision *pCollision, const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	if(!pCollision)
		rs_antiping::TimingVectors(f);
	else
		rs_antiping::LoopMode(pCollision, f);
	fclose(f);
	return 0;
}
