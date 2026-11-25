// "msgs" mode of rs_parity: the game messages that change the client prediction world
// (Sv_TuneParams, Sv_TeamsState, Sv_KillMsg, Sv_KillMsgTeam, Sv_PreInput), the tune zone
// bookkeeping (UpdateLocalTuning) and the map's tune settings (LoadMapSettings), for
// rust-client/ddnet/tests/parity.rs.
//
// Those are CGameClient members, and CGameClient cannot be linked into this tool. They
// are copied here word for word into a stand-in class of the same name (in its own
// namespace), with the members they use. Each copy is marked "C++ copy: <file>:<lines>"
// ... "end of copy"; the Rust test `cpp_copies_match_source` checks that every marked
// copy is still identical (whitespace aside) to those source lines. What the copies
// call is the real code: CUnpacker, CTuningParams, CTeamsCore, CCharOrder, CGameWorld,
// CCharacter, CCollision, CConsole.

#include <base/hash.h>
#include <base/log.h>
#include <base/str.h>

#include <engine/console.h>
#include <engine/map.h>
#include <engine/message.h>
#include <engine/shared/config.h>
#include <engine/shared/packer.h>
#include <engine/shared/protocol.h>

#include <game/client/prediction/entities/character.h>
#include <game/client/prediction/gameworld.h>
#include <game/collision.h>
#include <game/gamecore.h>
#include <game/mapbugs.h>
#include <game/mapitems.h>
#include <game/teamscore.h>

#include <generated/protocol.h>

#include <zlib.h>

#include <algorithm>
#include <bitset>
#include <cmath>
#include <cstdio>
#include <memory>
#include <utility>
#include <vector>

unsigned CharacterStateCrc(CCharacter *pChar); // rs_parity.cpp: WorldState

namespace rs_msgs {

static unsigned s_Seed = 777;
static unsigned Rand()
{
	s_Seed = s_Seed * 1103515245u + 12345u;
	return (s_Seed >> 8) & 0xffffff;
}

static unsigned Crc(const void *pData, size_t Size)
{
	return crc32(0, (const unsigned char *)pData, (unsigned)Size);
}

struct SFakeClient
{
	bool IsSixup() const { return false; }
	void SendMsgActive(CMsgPacker *pMsg, int Flags) {}
};

struct SAllowRestart
{
	bool m_AllowRestart = false;
};

struct SClientData
{
	CNetMsg_Sv_PreInput m_aPreInputs[200];
};

struct SCharacterInfo
{
	bool m_HasExtendedData = false;
	CNetObj_DDNetCharacter m_ExtendedData = {};
};

struct SSnapState
{
	const CNetObj_Character *m_pLocalCharacter = nullptr;
	const CNetObj_SpectatorInfo *m_pSpectatorInfo = nullptr;
	int m_LocalClientId = -1;
	SCharacterInfo m_aCharacters[MAX_CLIENTS];
};

struct SGameInfo
{
	bool m_PredictDDRaceTiles = false;
};

// The members of the real CGameClient that the copied code reads and writes.
class CGameClient
{
public:
	enum
	{
		SERVERMODE_PURE = 0,
	};
	int m_ServerMode = 0;
	CTuningParams m_aTuning[2];
	bool m_aReceivedTuning[2] = {};
	int m_aLocalTuneZone[2] = {-1, -1};
	int m_aExpectingTuningForZone[2] = {-1, -1};
	int m_aExpectingTuningSince[2] = {};
	CTuningParams m_aTuningList[TuneZone::NUM];
	CMapBugs m_MapBugs;
	CGameWorld m_GameWorld;
	CTeamsCore m_Teams;
	CCharOrder m_CharOrder;
	SAllowRestart m_Ghost;
	SAllowRestart m_RaceDemo;
	SClientData m_aClients[MAX_CLIENTS];
	SSnapState m_Snap;
	SGameInfo m_GameInfo;
	std::bitset<4> m_ActiveRecordings;
	int m_IsDummySwapping = 0;
	CCollision *m_pCollision = nullptr;
	IMap *m_pMap = nullptr;
	IConsole *m_pConsole = nullptr;
	SFakeClient m_Client;

	CTuningParams *TuningList() { return m_aTuningList; }
	CCollision *Collision() { return m_pCollision; }
	IMap *Map() { return m_pMap; }
	IConsole *Console() { return m_pConsole; }
	SFakeClient *Client() { return &m_Client; }

	CGameClient()
	{
		for(auto &Client : m_aClients)
			for(auto &PreInput : Client.m_aPreInputs)
				PreInput.m_IntendedTick = -1;
	}

	void OnTuneParams(CUnpacker *pUnpacker, int Conn);
	void OnTeamsState(CUnpacker *pUnpacker);
	void OnKillMsg(void *pRawMsg);
	void OnKillMsgTeam(void *pRawMsg);
	void OnPreInput(void *pRawMsg);
	void UpdateLocalTuning();
	void LoadMapSettings();
	static void ConTuneParam(IConsole::IResult *pResult, void *pUserData);
	static void ConTuneZone(IConsole::IResult *pResult, void *pUserData);
};

void CGameClient::OnTuneParams(CUnpacker *pUnpacker, int Conn)
{
	// C++ copy: game/client/gameclient.cpp:1055-1085
	// unpack the new tuning
	CTuningParams NewTuning;

	// No jetpack on DDNet incompatible servers,
	// jetpack strength will be received by tune params
	NewTuning.m_JetpackStrength = 0;

	int *pParams = NewTuning.NetworkArray();
	for(int i = 0; i < CTuningParams::Num(); i++)
	{
		static_assert(offsetof(CTuningParams, m_LaserDamage) / sizeof(CTuneParam) == 30);
		if(i == 30 && Client()->IsSixup()) // laser_damage was removed in 0.7
		{
			continue;
		}

		const int Value = pUnpacker->GetInt();

		// check for unpacking errors
		if(pUnpacker->Error())
			break;

		pParams[i] = Value;
	}

	m_ServerMode = SERVERMODE_PURE;

	m_aReceivedTuning[Conn] = true;
	// apply new tuning
	m_aTuning[Conn] = NewTuning;
	return;
	// end of copy
}

void CGameClient::OnTeamsState(CUnpacker *pUnpacker)
{
	// C++ copy: game/client/gameclient.cpp:1170-1190
	unsigned int i;

	for(i = 0; i < MAX_CLIENTS; i++)
	{
		const int Team = pUnpacker->GetInt();
		if(!pUnpacker->Error() && Team >= TEAM_FLOCK && Team < NUM_DDRACE_TEAMS)
		{
			m_Teams.Team(i, Team);
		}
		else
		{
			m_Teams.Team(i, 0);
			break;
		}
	}

	if(i <= VANILLA_MAX_CLIENTS)
		m_Teams.m_NumDDRaceTeams = VANILLA_MAX_CLIENTS + 1;

	m_Ghost.m_AllowRestart = true;
	m_RaceDemo.m_AllowRestart = true;
	// end of copy
}

void CGameClient::OnKillMsg(void *pRawMsg)
{
	// C++ copy: game/client/gameclient.cpp:1194-1202
	CNetMsg_Sv_KillMsg *pMsg = (CNetMsg_Sv_KillMsg *)pRawMsg;
	// reset character prediction
	if(!(m_GameWorld.m_WorldConfig.m_IsFNG && pMsg->m_Weapon == WEAPON_LASER))
	{
		m_CharOrder.GiveWeak(pMsg->m_Victim);
		if(CCharacter *pChar = m_GameWorld.GetCharacterById(pMsg->m_Victim))
			pChar->ResetPrediction();
		m_GameWorld.ReleaseHooked(pMsg->m_Victim);
	}
	// end of copy
}

void CGameClient::OnKillMsgTeam(void *pRawMsg)
{
	// C++ copy: game/client/gameclient.cpp:1206-1226
	CNetMsg_Sv_KillMsgTeam *pMsg = (CNetMsg_Sv_KillMsgTeam *)pRawMsg;

	// reset prediction
	std::vector<std::pair<int, int>> vStrongWeakSorted;
	for(int i = 0; i < MAX_CLIENTS; i++)
	{
		if(m_Teams.Team(i) == pMsg->m_Team)
		{
			if(CCharacter *pChar = m_GameWorld.GetCharacterById(i))
			{
				pChar->ResetPrediction();
				vStrongWeakSorted.emplace_back(i, pMsg->m_First == i ? MAX_CLIENTS : (pChar ? pChar->GetStrongWeakId() : 0));
			}
			m_GameWorld.ReleaseHooked(i);
		}
	}
	std::stable_sort(vStrongWeakSorted.begin(), vStrongWeakSorted.end(), [](auto &Left, auto &Right) { return Left.second > Right.second; });
	for(auto Id : vStrongWeakSorted)
	{
		m_CharOrder.GiveWeak(Id.first);
	}
	// end of copy
}

void CGameClient::OnPreInput(void *pRawMsg)
{
	// C++ copy: game/client/gameclient.cpp:1238-1239
	CNetMsg_Sv_PreInput *pMsg = (CNetMsg_Sv_PreInput *)pRawMsg;
	m_aClients[pMsg->m_Owner].m_aPreInputs[pMsg->m_IntendedTick % 200] = *pMsg;
	// end of copy
}

// C++ copy: game/client/gameclient.cpp:3409-3485
void CGameClient::UpdateLocalTuning()
{
	m_GameWorld.m_WorldConfig.m_UseTuneZones = m_GameInfo.m_PredictDDRaceTiles;

	// always update default tune zone, even without character
	if(!m_GameWorld.m_WorldConfig.m_UseTuneZones)
		m_GameWorld.TuningList()[0] = m_aTuning[g_Config.m_ClDummy];

	if(!m_Snap.m_pLocalCharacter && !m_Snap.m_pSpectatorInfo)
		return;

	vec2 LocalPos = m_Snap.m_pLocalCharacter ? vec2(m_Snap.m_pLocalCharacter->m_X, m_Snap.m_pLocalCharacter->m_Y) : vec2(m_Snap.m_pSpectatorInfo->m_X, m_Snap.m_pSpectatorInfo->m_Y);

	// update the tuning at the local position with the latest tunings received before the new snapshot
	if(m_GameWorld.m_WorldConfig.m_UseTuneZones)
	{
		int TuneZone =
			m_Snap.m_aCharacters[m_Snap.m_LocalClientId].m_HasExtendedData &&
					m_Snap.m_aCharacters[m_Snap.m_LocalClientId].m_ExtendedData.m_TuneZoneOverride != TuneZone::OVERRIDE_NONE ?
				m_Snap.m_aCharacters[m_Snap.m_LocalClientId].m_ExtendedData.m_TuneZoneOverride :
				Collision()->IsTune(Collision()->GetMapIndex(LocalPos));

		if(TuneZone != m_aLocalTuneZone[g_Config.m_ClDummy])
		{
			// our tunezone changed, expecting tuning message
			m_aLocalTuneZone[g_Config.m_ClDummy] = m_aExpectingTuningForZone[g_Config.m_ClDummy] = TuneZone;
			m_aExpectingTuningSince[g_Config.m_ClDummy] = 0;
		}

		// tunezone could have changed, send dummy tuning to demo
		if(m_ActiveRecordings.any() && m_IsDummySwapping && m_aLocalTuneZone[0] != m_aLocalTuneZone[1])
		{
			CMsgPacker Msg(NETMSGTYPE_SV_TUNEPARAMS);
			int *pParams = (int *)&m_aTuning[g_Config.m_ClDummy];
			for(unsigned i = 0; i < sizeof(m_aTuning[0]) / sizeof(int); i++)
				Msg.AddInt(pParams[i]);
			Client()->SendMsgActive(&Msg, MSGFLAG_RECORD | MSGFLAG_NOSEND);
		}

		if(m_aExpectingTuningForZone[g_Config.m_ClDummy] >= 0)
		{
			if(m_aReceivedTuning[g_Config.m_ClDummy])
			{
				TuningList()[m_aExpectingTuningForZone[g_Config.m_ClDummy]] = m_aTuning[g_Config.m_ClDummy];
				m_GameWorld.TuningList()[m_aExpectingTuningForZone[g_Config.m_ClDummy]] = m_aTuning[g_Config.m_ClDummy];
				m_aReceivedTuning[g_Config.m_ClDummy] = false;
				m_aExpectingTuningForZone[g_Config.m_ClDummy] = -1;
			}
			else if(m_aExpectingTuningSince[g_Config.m_ClDummy] >= 5)
			{
				// if we are expecting tuning for more than 10 snaps (less than a quarter of a second)
				// it is probably dropped or it was received out of order
				// or applied to another tunezone.
				// we need to fallback to current tuning to fix ourselves.
				m_aExpectingTuningForZone[g_Config.m_ClDummy] = -1;
				m_aExpectingTuningSince[g_Config.m_ClDummy] = 0;
				m_aReceivedTuning[g_Config.m_ClDummy] = false;
				log_debug("tunezone", "the tuning was missed");
			}
			else
			{
				// if we are expecting tuning and have not received one yet.
				// do not update any tuning, so we don't apply it to the wrong tunezone.
				log_debug("tunezone", "waiting for tuning for zone %d", m_aExpectingTuningForZone[g_Config.m_ClDummy]);
				m_aExpectingTuningSince[g_Config.m_ClDummy]++;
			}
		}
		else
		{
			// if we have processed what we need, and the tuning is still wrong due to out of order message
			// fix our tuning by using the current one
			m_GameWorld.TuningList()[TuneZone] = m_aTuning[g_Config.m_ClDummy];
			m_aExpectingTuningSince[g_Config.m_ClDummy] = 0;
			m_aReceivedTuning[g_Config.m_ClDummy] = false;
		}
	}
}
// end of copy

static bool UnknownMapSettingCallback(const char *pCommand, void *pUser)
{
	return true;
}

// C++ copy: game/client/gameclient.cpp:4208-4253
void CGameClient::LoadMapSettings()
{
	m_MapBugs = CMapBugs::Create(Map()->BaseName(), Map()->Size(), Map()->Sha256());

	// Reset Tunezones
	for(int TuneZone = 0; TuneZone < TuneZone::NUM; TuneZone++)
	{
		TuningList()[TuneZone] = CTuningParams::DEFAULT;
		TuningList()[TuneZone].Set("gun_curvature", 0);
		TuningList()[TuneZone].Set("gun_speed", 1400);
		TuningList()[TuneZone].Set("shotgun_curvature", 0);
		TuningList()[TuneZone].Set("shotgun_speed", 500);
		TuningList()[TuneZone].Set("shotgun_speeddiff", 0);
	}

	// Load map tunings
	int Start, Num;
	Map()->GetType(MAPITEMTYPE_INFO, &Start, &Num);
	for(int i = Start; i < Start + Num; i++)
	{
		int ItemId;
		CMapItemInfoSettings *pItem = (CMapItemInfoSettings *)Map()->GetItem(i, nullptr, &ItemId);
		int ItemSize = Map()->GetItemSize(i);
		if(!pItem || ItemId != 0)
			continue;

		if(ItemSize < (int)sizeof(CMapItemInfoSettings))
			break;
		if(!(pItem->m_Settings > -1))
			break;

		int Size = Map()->GetDataSize(pItem->m_Settings);
		char *pSettings = (char *)Map()->GetData(pItem->m_Settings);
		char *pNext = pSettings;
		Console()->SetUnknownCommandCallback(UnknownMapSettingCallback, nullptr);
		while(pNext < pSettings + Size)
		{
			int StrSize = str_length(pNext) + 1;
			Console()->ExecuteLine(pNext, IConsole::CLIENT_ID_GAME);
			pNext += StrSize;
		}
		Console()->SetUnknownCommandCallback(IConsole::EmptyUnknownCommandCallback, nullptr);
		Map()->UnloadData(pItem->m_Settings);
		break;
	}
}
// end of copy

// C++ copy: game/client/gameclient.cpp:4255-4275
void CGameClient::ConTuneParam(IConsole::IResult *pResult, void *pUserData)
{
	CGameClient *pSelf = (CGameClient *)pUserData;
	const char *pParamName = pResult->GetString(0);
	if(pResult->NumArguments() == 2)
	{
		float NewValue = pResult->GetFloat(1);
		pSelf->TuningList()[0].Set(pParamName, NewValue);
	}
}

void CGameClient::ConTuneZone(IConsole::IResult *pResult, void *pUserData)
{
	CGameClient *pSelf = (CGameClient *)pUserData;
	int List = pResult->GetInteger(0);
	const char *pParamName = pResult->GetString(1);
	float NewValue = pResult->GetFloat(2);

	if(List >= 0 && List < TuneZone::NUM)
		pSelf->TuningList()[List].Set(pParamName, NewValue);
}
// end of copy

static std::unique_ptr<IConsole> CreateGameConsole(CGameClient *pClient)
{
	// the registrations of gameclient.cpp:199-200
	std::unique_ptr<IConsole> pConsole = CreateConsole(CFGFLAG_CLIENT);
	pConsole->Register("tune", "s[tuning] ?f[value]", CFGFLAG_GAME, CGameClient::ConTuneParam, pClient, "Tune variable to value");
	pConsole->Register("tune_zone", "i[zone] s[tuning] f[value]", CFGFLAG_GAME, CGameClient::ConTuneZone, pClient, "Tune in zone a variable to value");
	return pConsole;
}

static void Hex(FILE *f, const unsigned char *pData, int Size)
{
	if(Size == 0)
		fprintf(f, "-");
	for(int i = 0; i < Size; i++)
		fprintf(f, "%02x", pData[i]);
}

static unsigned TuningCrc(const CTuningParams *pList, int Num)
{
	return Crc(pList, sizeof(CTuningParams) * Num);
}

// A payload of `Num` ints (values from small to huge), sometimes cut inside the last
// int or followed by garbage.
static std::vector<unsigned char> IntPayload(int Num, int Lo, int Hi)
{
	CPacker Packer;
	Packer.Reset();
	for(int i = 0; i < Num; i++)
	{
		const int Kind = Rand() % 10;
		int v = Lo + (int)(Rand() % (unsigned)(Hi - Lo + 1));
		if(Kind == 0)
			v = (int)(Rand() << 8 | Rand());
		else if(Kind == 1)
			v = -(int)(Rand() % 100000);
		Packer.AddInt(v);
	}
	std::vector<unsigned char> v(Packer.Data(), Packer.Data() + Packer.Size());
	const int Mutate = Rand() % 6;
	if(Mutate == 0 && !v.empty())
		v.resize(v.size() - 1 - Rand() % std::min<size_t>(v.size(), 3)); // truncated
	else if(Mutate == 1)
		v.push_back(0x80 | (Rand() & 0x7f)); // a varint that never ends
	return v;
}

static const char *const s_apNames[] = {"gun_speed", "GUN_CURVATURE", "gravity", "player_collision", "player_hooking",
	"jetpack_strength", "laser_reach", "Shotgun_Speed", "explosion_strength", "hook_length", "ground_elasticity_y",
	"velramp_curvature", "laser_bounce_delay", "no_such_param", "", "gun speed"};
static const char *const s_apValues[] = {"0", "1", "-1", "1400", "0.5", "-0.25", ".5", "5.", "1e3", "2.5E-2", "3000000",
	"1e30", "-1e30", "inf", "abc", "1.5x", "+7", "  12", "0x10", "99999999999", "1e-40", "\"3\"", "-0", "\"\"",
	"0x1.8p3", "-0X.4", "nan", "NAN(12_a)", "0x", "1e"};
static const char *const s_apZones[] = {"0", "1", "3", "17", "255", "256", "-1", "abc", "2147483647", "99999999999", "\"4\"", "0x3"};

static std::string RandomSettingLine()
{
	auto Pick = [](const char *const *ppList, int Num) { return std::string(ppList[Rand() % Num]); };
	const std::string Name = Pick(s_apNames, std::size(s_apNames));
	const std::string Value = Pick(s_apValues, std::size(s_apValues));
	const std::string Zone = Pick(s_apZones, std::size(s_apZones));
	switch(Rand() % 12)
	{
	case 0: return "tune " + Name + " " + Value;
	case 1: return "TUNE " + Name + " " + Value;
	case 2: return "tune_zone " + Zone + " " + Name + " " + Value;
	case 3: return "Tune_Zone " + Zone + " \"" + Name + "\" " + Value;
	case 4: return "tune " + Name;
	case 5: return "tune_zone " + Zone + " " + Name;
	case 6: return "tune " + Name + " " + Value + "; tune_zone " + Zone + " " + Name + " " + Value;
	case 7: return "  tune_zone " + Zone + " " + Name + " " + Value + " # comment";
	case 8: return "sv_hit 0";
	case 9: return "echo hi; tune " + Name + " " + Value + " extra";
	case 10: return "tune \"" + Name + "; tune gravity 9\" " + Value;
	default: return "";
	}
}

static constexpr int MSG_RUNS = 30;
static constexpr int MSG_EVENTS = 300;
static constexpr int MSG_CHARS = 6;
static constexpr int SETTINGS_BLOBS = 400;

static int Run(IMap *pMap, CCollision *pCollision, FILE *f)
{
	// the tool's g_Config is zeroed, set what the prediction reads to the defaults
	g_Config.m_SvFreezeDelay = 3;
	g_Config.m_ClAntiPingPlayers = 1;
	g_Config.m_SvDeepfly = 1;
	g_Config.m_ClDummy = 0;

	// the map's own settings
	{
		auto pClient = std::make_unique<CGameClient>();
		std::unique_ptr<IConsole> pConsole = CreateGameConsole(pClient.get());
		pClient->m_pMap = pMap;
		pClient->m_pConsole = pConsole.get();
		pClient->LoadMapSettings();
		fprintf(f, "mapsettings = %08x\n", TuningCrc(pClient->m_aTuningList, TuneZone::NUM));
	}

	// synthetic settings blobs: NUL-separated lines run like LoadMapSettings' loop
	for(int b = 0; b < SETTINGS_BLOBS; b++)
	{
		auto pClient = std::make_unique<CGameClient>();
		std::unique_ptr<IConsole> pConsole = CreateGameConsole(pClient.get());
		for(auto &Tuning : pClient->m_aTuningList)
			Tuning = CTuningParams::DEFAULT;
		std::vector<char> vBlob;
		const int Lines = 1 + Rand() % 6;
		for(int l = 0; l < Lines; l++)
		{
			const std::string Line = RandomSettingLine();
			vBlob.insert(vBlob.end(), Line.begin(), Line.end());
			vBlob.push_back('\0');
		}
		const int Size = (int)vBlob.size();
		vBlob.push_back('\0');
		pConsole->SetUnknownCommandCallback(UnknownMapSettingCallback, nullptr);
		for(char *pNext = vBlob.data(); pNext < vBlob.data() + Size; pNext += str_length(pNext) + 1)
			pConsole->ExecuteLine(pNext, IConsole::CLIENT_ID_GAME);
		fprintf(f, "settings ");
		Hex(f, (const unsigned char *)vBlob.data(), Size);
		fprintf(f, " = %08x\n", TuningCrc(pClient->m_aTuningList, TuneZone::NUM));
	}

	const int Width = pCollision->GetWidth(), Height = pCollision->GetHeight();
	for(int r = 0; r < MSG_RUNS; r++)
	{
		auto pClient = std::make_unique<CGameClient>();
		CGameClient &C = *pClient;
		C.m_pCollision = pCollision;
		for(auto &Tuning : C.m_aTuningList)
			Tuning = CTuningParams::DEFAULT;
		C.m_GameWorld.Init(pCollision, C.m_aTuningList, nullptr);
		C.m_GameWorld.m_Core.InitSwitchers(pCollision->m_HighestSwitchNumber);
		C.m_GameWorld.m_GameTick = 1000;
		C.m_GameWorld.m_WorldConfig = {};
		C.m_GameWorld.m_WorldConfig.m_InfiniteAmmo = true;
		C.m_GameWorld.m_WorldConfig.m_PredictFreeze = 1;
		C.m_GameWorld.m_WorldConfig.m_IsDDRace = true;
		C.m_GameWorld.m_WorldConfig.m_PredictDDRace = true;
		C.m_GameWorld.m_WorldConfig.m_PredictTiles = true;
		C.m_GameWorld.m_WorldConfig.m_IsFNG = r % 3 == 2;
		fprintf(f, "run %d %d\n", r, C.m_GameWorld.m_WorldConfig.m_IsFNG);

		int aIds[MSG_CHARS];
		for(int i = 0; i < MSG_CHARS; i++)
		{
			int Id;
			do
				Id = Rand() % MAX_CLIENTS;
			while(std::find(aIds, aIds + i, Id) != aIds + i);
			aIds[i] = Id;
		}
		for(int i = 0; i < MSG_CHARS; i++)
		{
			CNetObj_Character Obj = {};
			Obj.m_Tick = 990;
			Obj.m_X = 32 + Rand() % std::max(1, Width * 32 - 64);
			Obj.m_Y = 32 + Rand() % std::max(1, Height * 32 - 64);
			Obj.m_HookX = Obj.m_X + 50;
			Obj.m_HookY = Obj.m_Y;
			Obj.m_HookState = (int)(Rand() % 7) - 1;
			Obj.m_HookedPlayer = Rand() % 2 ? aIds[Rand() % MSG_CHARS] : -1;
			Obj.m_Weapon = WEAPON_GUN;
			Obj.m_AmmoCount = 10;
			CNetObj_DDNetCharacter Ext = {};
			Ext.m_Flags = CHARACTERFLAG_WEAPON_HAMMER | CHARACTERFLAG_WEAPON_GUN;
			const unsigned Extra = Rand();
			if(Extra % 4 == 0)
				Ext.m_Flags |= CHARACTERFLAG_SUPER;
			if(Extra % 5 == 0)
				Ext.m_Flags |= CHARACTERFLAG_SOLO;
			if(Extra % 3 == 0)
				Ext.m_Flags |= CHARACTERFLAG_ENDLESS_HOOK | CHARACTERFLAG_JETPACK | CHARACTERFLAG_WEAPON_GRENADE;
			if(Extra % 7 == 0)
				Ext.m_Flags |= CHARACTERFLAG_HAMMER_HIT_DISABLED | CHARACTERFLAG_COLLISION_DISABLED;
			Ext.m_Jumps = 1 + Rand() % 3;
			Ext.m_JumpedTotal = -1;
			Ext.m_NinjaActivationTick = -1;
			Ext.m_FreezeStart = Extra % 6 == 0 ? 995 : -1;
			Ext.m_FreezeEnd = Extra % 6 == 0 ? 1100 : 0;
			Ext.m_StrongWeakId = Rand() % MAX_CLIENTS;
			Ext.m_TuneZoneOverride = TuneZone::OVERRIDE_NONE;
			fprintf(f, "char %d", aIds[i]);
			for(int k = 0; k < (int)(sizeof(Obj) / sizeof(int)); k++)
				fprintf(f, " %d", ((const int *)&Obj)[k]);
			fprintf(f, " |");
			for(int k = 0; k < (int)(sizeof(Ext) / sizeof(int)); k++)
				fprintf(f, " %d", ((const int *)&Ext)[k]);
			fprintf(f, "\n");
			CCharacter *pChar = new CCharacter(&C.m_GameWorld, aIds[i], &Obj, &Ext);
			C.m_GameWorld.InsertEntity(pChar);
		}
		C.m_Snap.m_LocalClientId = aIds[0];
		int SnapX = 0, SnapY = 0, SnapOverride = TuneZone::OVERRIDE_NONE;
		bool SnapHasExt = false;

		auto DumpWorld = [&]() {
			fprintf(f, " =");
			for(int Id : C.m_CharOrder.m_Ids)
				fprintf(f, " %d", Id);
			fprintf(f, " |");
			for(int Id = 0; Id < MAX_CLIENTS; Id++)
				if(CCharacter *pChar = C.m_GameWorld.GetCharacterById(Id))
					fprintf(f, " %d:%08x", Id, CharacterStateCrc(pChar));
			fprintf(f, "\n");
		};

		for(int e = 0; e < MSG_EVENTS; e++)
		{
			const int Kind = Rand() % 10;
			if(Kind == 0)
			{
				const std::vector<unsigned char> v = IntPayload(Rand() % (CTuningParams::Num() + 3), 0, 5000);
				fprintf(f, "tune ");
				Hex(f, v.data(), (int)v.size());
				CUnpacker Unpacker;
				Unpacker.Reset(v.data(), (int)v.size());
				C.OnTuneParams(&Unpacker, 0);
				fprintf(f, " = %08x %d\n", TuningCrc(&C.m_aTuning[0], 1), C.m_aReceivedTuning[0]);
			}
			else if(Kind == 1)
			{
				const int Num = Rand() % 2 ? MAX_CLIENTS : Rand() % 3 ? (int)(Rand() % (MAX_CLIENTS + 2)) : VANILLA_MAX_CLIENTS + (int)(Rand() % 3) - 1;
				const std::vector<unsigned char> v = IntPayload(Num, 0, Rand() % 4 ? 3 : 70);
				fprintf(f, "teams ");
				Hex(f, v.data(), (int)v.size());
				CUnpacker Unpacker;
				Unpacker.Reset(v.data(), (int)v.size());
				C.m_Teams.m_NumDDRaceTeams = NUM_DDRACE_TEAMS; // as each snapshot sets it
				C.OnTeamsState(&Unpacker);
				fprintf(f, " =");
				for(int i = 0; i < MAX_CLIENTS; i++)
					fprintf(f, " %d", C.m_Teams.Team(i));
				fprintf(f, " %d\n", C.m_Teams.m_NumDDRaceTeams);
			}
			else if(Kind == 2)
			{
				CNetMsg_Sv_KillMsg Msg = {};
				Msg.m_Victim = Rand() % 4 ? aIds[Rand() % MSG_CHARS] : (int)(Rand() % MAX_CLIENTS);
				Msg.m_Weapon = (int)(Rand() % 8) - 1;
				fprintf(f, "kill %d %d", Msg.m_Victim, Msg.m_Weapon);
				C.OnKillMsg(&Msg);
				DumpWorld();
			}
			else if(Kind == 3)
			{
				CNetMsg_Sv_KillMsgTeam Msg = {};
				Msg.m_Team = Rand() % 4;
				Msg.m_First = Rand() % 3 ? aIds[Rand() % MSG_CHARS] : -1;
				fprintf(f, "killteam %d %d", Msg.m_Team, Msg.m_First);
				C.OnKillMsgTeam(&Msg);
				DumpWorld();
			}
			else if(Kind == 4)
			{
				CNetMsg_Sv_PreInput Msg = {};
				int *pInts = (int *)&Msg;
				for(int k = 0; k < (int)(sizeof(Msg) / sizeof(int)); k++)
					pInts[k] = (int)(Rand() % 2000) - 1000;
				Msg.m_Owner = Rand() % MAX_CLIENTS;
				Msg.m_IntendedTick = Rand() % 100000;
				fprintf(f, "pre");
				for(int k = 0; k < (int)(sizeof(Msg) / sizeof(int)); k++)
					fprintf(f, " %d", pInts[k]);
				C.OnPreInput(&Msg);
				const int Tick = Rand() % 2 ? Msg.m_IntendedTick : (int)(Rand() % 100000);
				const int Owner = Rand() % 2 ? Msg.m_Owner : (int)(Rand() % MAX_CLIENTS);
				const CNetMsg_Sv_PreInput &Slot = C.m_aClients[Owner].m_aPreInputs[Tick % 200];
				fprintf(f, " ? %d %d =", Owner, Tick);
				if(Slot.m_IntendedTick == Tick)
					for(int k = 0; k < (int)(sizeof(Slot) / sizeof(int)); k++)
						fprintf(f, " %d", ((const int *)&Slot)[k]);
				else
					fprintf(f, " none");
				fprintf(f, "\n");
			}
			else
			{
				CNetObj_Character Local = {};
				CNetObj_SpectatorInfo Spec = {};
				const int Where = Rand() % 6;
				// the local tee often stays in its zone for a few snapshots
				if(Rand() % 3 == 0)
				{
					SnapX = Rand() % (Width * 32);
					SnapY = Rand() % (Height * 32);
					SnapHasExt = Rand() % 3 != 0;
					const int Pick = Rand() % 4;
					SnapOverride = Pick < 2 ? TuneZone::OVERRIDE_NONE : Pick == 2 ? 0 : (int)(Rand() % TuneZone::NUM);
				}
				const int x = SnapX, y = SnapY;
				Local.m_X = Spec.m_X = x;
				Local.m_Y = Spec.m_Y = y;
				C.m_Snap.m_pLocalCharacter = Where < 4 ? &Local : nullptr;
				C.m_Snap.m_pSpectatorInfo = Where == 4 || (Where < 4 && Rand() % 2) ? &Spec : nullptr;
				SCharacterInfo &Info = C.m_Snap.m_aCharacters[C.m_Snap.m_LocalClientId];
				Info.m_HasExtendedData = SnapHasExt;
				Info.m_ExtendedData.m_TuneZoneOverride = SnapOverride;
				C.m_GameInfo.m_PredictDDRaceTiles = Rand() % 10 != 0;
				fprintf(f, "snap %d %d %d %d %d %d", C.m_Snap.m_pLocalCharacter != nullptr, C.m_Snap.m_pSpectatorInfo != nullptr,
					x, y, Info.m_HasExtendedData ? Info.m_ExtendedData.m_TuneZoneOverride : -2, C.m_GameInfo.m_PredictDDRaceTiles);
				C.UpdateLocalTuning();
				C.m_Snap.m_pLocalCharacter = nullptr;
				C.m_Snap.m_pSpectatorInfo = nullptr;
				fprintf(f, " = %08x %d %d %d %d\n", TuningCrc(C.m_aTuningList, TuneZone::NUM), C.m_aReceivedTuning[0],
					C.m_aLocalTuneZone[0], C.m_aExpectingTuningForZone[0], C.m_aExpectingTuningSince[0]);
			}
		}
	}
	return 0;
}

} // namespace rs_msgs

int MsgsMode(IMap *pMap, CCollision *pCollision, const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	const int Result = rs_msgs::Run(pMap, pCollision, f);
	fclose(f);
	return Result;
}
