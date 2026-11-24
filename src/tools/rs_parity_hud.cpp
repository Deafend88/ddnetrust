// "movement" mode of rs_parity: runs the real CHud::GetMovementInformation (hud.cpp:1424)
// and CPlayers::GetPlayerTargetAngle (players.cpp:97) for random characters, and writes
// inputs and results for rust-client/client/tests/hud.rs.
//
// Like rs_parity_camera.cpp, the components are constructed in a zeroed block of
// CGameClient-sized memory; only the members they read are set (the intra tick, m_Snap's
// characters and local id, the tuning, the local mouse position). Spectating paths
// (freeview, m_SpecChar) and the predicted dummy are not exercised.

#include <base/dbg.h>
#include <base/math.h>
#include <base/mem.h>
#include <base/str.h>

#include <engine/client.h>
#include <engine/shared/config.h>

#include <engine/textrender.h>

#include <game/client/components/effects.h>
#include <game/client/components/flow.h>
#include <game/client/components/hud.h>
#include <game/client/components/players.h>
#include <game/client/components/voting.h>
#include <game/client/gameclient.h>
#include <game/client/render.h>

#include <cmath>
#include <cstdio>
#include <new>

// Access to private members: explicit template instantiations may name them. The
// result type is private too, so it is deduced.
auto GetMovementInformationPtr();
template<auto M>
struct SRobMovement
{
	friend auto GetMovementInformationPtr() { return M; }
};
template struct SRobMovement<&CHud::GetMovementInformation>;
template<typename Tag, typename Tag::type M>
struct SRobHud
{
	friend typename Tag::type Get(Tag) { return M; }
};
struct SHudClientPtr
{
	typedef IClient *CGameClient::*type;
	friend type Get(SHudClientPtr);
};
template struct SRobHud<SHudClientPtr, &CGameClient::m_pClient>;

// IClient's protected intra tick, named through a derived class.
struct SHudClientFields : IClient
{
	static float (IClient::*IntraTick())[NUM_DUMMIES] { return &SHudClientFields::m_aGameIntraTick; }
};

// C++: gameclient.cpp:999, copied (gameclient.cpp is not linked); with cl_predict_dummy 0
// it returns before the virtual DummyConnected.
bool CGameClient::PredictDummy() const
{
	return g_Config.m_ClPredictDummy &&
	       Client()->DummyConnected() &&
	       m_Snap.m_LocalClientId >= 0 &&
	       m_aLocalIds[!g_Config.m_ClDummy] >= 0 &&
	       !m_aClients[m_aLocalIds[!g_Config.m_ClDummy]].m_Paused;
}

// Rendering code linked in with hud.cpp and players.cpp; never reached here.
#define NOT_REACHED dbg_assert(false, "rs_parity movement: not reached")
bool CGameClient::IsTeamPlay() const { NOT_REACHED; return false; }
bool CGameClient::AntiPingGunfire() const { NOT_REACHED; return false; }
bool CGameClient::IsDemoPlaybackPaused() const { NOT_REACHED; return false; }
bool CGameClient::IsOtherTeam(int ClientId) const { NOT_REACHED; return false; }
std::shared_ptr<CManagedTeeRenderInfo> CGameClient::CreateManagedTeeRenderInfo(const CTeeRenderInfo &TeeRenderInfo, const CSkinDescriptor &SkinDescriptor) { NOT_REACHED; return nullptr; }
int CGameClient::IntersectCharacter(vec2 HookPos, vec2 NewPos, vec2 &NewPos2, int OwnId, vec2 *pPlayerPosition) { NOT_REACHED; return -1; }
CSkinDescriptor::CSkinDescriptor() { NOT_REACHED; }
void CRenderTools::GetRenderTeeAnimScaleAndBaseSize(const CTeeRenderInfo *pInfo, float &AnimScale, float &BaseSize) { NOT_REACHED; }
void CRenderTools::RenderIdleTeeAtCenter(const CTeeRenderInfo *pInfo, vec2 Pos, int Emote, vec2 Dir, float Alpha) const { NOT_REACHED; }
void CRenderTools::RenderTee(const CAnimState *pAnim, const CTeeRenderInfo *pInfo, int Emote, vec2 Dir, vec2 Pos, float Alpha) const { NOT_REACHED; }
void CEffects::FreezingFlakes(vec2 Pos, vec2 Size, float Alpha) { NOT_REACHED; }
void CEffects::PowerupShine(vec2 Pos, vec2 Size, float Alpha) { NOT_REACHED; }
void CEffects::SkidTrail(vec2 Pos, vec2 Vel, int Direction, float Alpha, float Volume) { NOT_REACHED; }
void CEffects::SparkleTrail(vec2 Pos, float Alpha) { NOT_REACHED; }
void CFlow::Add(vec2 Pos, vec2 Vel, float Size) { NOT_REACHED; }
void CTextCursor::SetPosition(vec2 Position) { NOT_REACHED; }
void CVoting::Render() { NOT_REACHED; }

static unsigned s_HudSeed = 4242;
static unsigned HudRand()
{
	s_HudSeed = s_HudSeed * 1103515245u + 12345u;
	return s_HudSeed >> 8;
}

static unsigned HudBits(float f)
{
	unsigned u;
	mem_copy(&u, &f, sizeof(u));
	return u;
}

// the text CHud::UpdateMovementInformationTextContainer (hud.cpp:1401) shows for a value
static const char *Shown(float Value, char *pBuf, int Size)
{
	Value = std::round(Value * 100.0f) / 100.0f; // Round to 2dp
	str_format(pBuf, Size, "%.2f", Value);
	return pBuf;
}

// a velocity near the thresholds GetMovementInformation tests (|x| <= 1, |y| <= 128) or
// anywhere in the range characters reach
static int RandVel()
{
	switch(HudRand() % 4)
	{
	case 0: return (int)(HudRand() % 5) - 2;
	case 1: return (int)(HudRand() % 5) - 2 + (HudRand() % 2 ? 128 : -128);
	default: return (int)(HudRand() % 200001) - 100000;
	}
}

int MovementMode(const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;

	alignas(CGameClient) static unsigned char s_aGameClient[sizeof(CGameClient)];
	alignas(IClient) static unsigned char s_aClient[sizeof(IClient)];
	CGameClient *pGameClient = reinterpret_cast<CGameClient *>(s_aGameClient);
	IClient *pClient = reinterpret_cast<IClient *>(s_aClient);
	pGameClient->*Get(SHudClientPtr()) = pClient;
	CHud *pHud = new(&pGameClient->m_Hud) CHud();
	CPlayers *pPlayers = new(&pGameClient->m_Players) CPlayers();
	pHud->OnInterfacesInit(pGameClient);
	pPlayers->OnInterfacesInit(pGameClient);
	g_Config.m_ClDummy = 0;
	g_Config.m_ClPredictDummy = 0;
	const auto pfnGet = GetMovementInformationPtr();

	for(int i = 0; i < 20000; i++)
	{
		const int ClientId = HudRand() % MAX_CLIENTS;
		const bool Local = HudRand() % 3 == 0;
		pGameClient->m_Snap.m_LocalClientId = Local ? ClientId : (ClientId + 1) % MAX_CLIENTS;
		const float Intra = HudRand() % 8 == 0 ? (float)(HudRand() % 2) : (HudRand() % 100001) / 100000.0f;
		(pClient->*SHudClientFields::IntraTick())[0] = Intra;

		auto &Char = pGameClient->m_Snap.m_aCharacters[ClientId];
		CNetObj_Character Prev = {}, Cur = {};
		Prev.m_X = (int)(HudRand() % 400000) - 50000;
		Prev.m_Y = (int)(HudRand() % 400000) - 50000;
		Cur.m_X = HudRand() % 4 ? Prev.m_X + (int)(HudRand() % 801) - 400 : (int)(HudRand() % 400000) - 50000;
		Cur.m_Y = HudRand() % 4 ? Prev.m_Y + (int)(HudRand() % 801) - 400 : (int)(HudRand() % 400000) - 50000;
		Prev.m_VelX = RandVel();
		Prev.m_VelY = RandVel();
		Cur.m_VelX = HudRand() % 3 ? Prev.m_VelX + (int)(HudRand() % 41) - 20 : RandVel();
		Cur.m_VelY = HudRand() % 3 ? Prev.m_VelY + (int)(HudRand() % 41) - 20 : RandVel();
		Prev.m_Angle = (int)(HudRand() % 1609) - 402;
		Cur.m_Angle = (int)(HudRand() % 1609) - 402;
		Char.m_Prev = Prev;
		Char.m_Cur = Cur;

		// other players: the DDNet target (mode 1), also the previous one (mode 2)
		static CNetObj_DDNetCharacter s_PrevExt;
		const int Mode = HudRand() % 3;
		Char.m_HasExtendedDisplayInfo = Mode != 0;
		Char.m_ExtendedData.m_TargetX = (int)(HudRand() % 2001) - 1000;
		Char.m_ExtendedData.m_TargetY = (int)(HudRand() % 2001) - 1000;
		s_PrevExt.m_TargetX = (int)(HudRand() % 2001) - 1000;
		s_PrevExt.m_TargetY = (int)(HudRand() % 2001) - 1000;
		Char.m_pPrevExtendedData = Mode == 2 ? &s_PrevExt : nullptr;

		// the local player's aim: the mouse position (CControls::m_aMousePos)
		vec2 Mouse = vec2((int)(HudRand() % 2001) - 1000, (int)(HudRand() % 2001) - 1000) / (float)(1 + HudRand() % 4);
		if(HudRand() % 10 == 0)
			Mouse = vec2((int)(HudRand() % 3) - 1, (int)(HudRand() % 3) - 1) * 0.5f;
		pGameClient->m_Controls.m_aMousePos[0] = Mouse;

		// the velocity ramp tuning (CTuneParam: hundredths), mostly the default
		CTuningParams &Tuning = pGameClient->m_aTuning[0];
		Tuning = CTuningParams();
		if(HudRand() % 4 == 0)
		{
			Tuning.m_VelrampStart = (HudRand() % 200000) / 100.0f;
			Tuning.m_VelrampRange = (1 + HudRand() % 400000) / 100.0f;
			Tuning.m_VelrampCurvature = (101 + HudRand() % 400) / 100.0f;
		}

		const auto Info = (pHud->*pfnGet)(ClientId, 0);
		char a[5][64];
		fprintf(f, "move %d %d %d %d %d %d %d %d %d %d %d %d %d %d %08x %08x %d %d %08x %d %d %d | %08x %08x %08x %08x %08x | %s %s %s %s %s\n",
			Local, Prev.m_X, Prev.m_Y, Prev.m_VelX, Prev.m_VelY, Prev.m_Angle, Cur.m_X, Cur.m_Y, Cur.m_VelX, Cur.m_VelY, Cur.m_Angle,
			Mode, s_PrevExt.m_TargetX, s_PrevExt.m_TargetY, HudBits(Mouse.x), HudBits(Mouse.y),
			Char.m_ExtendedData.m_TargetX, Char.m_ExtendedData.m_TargetY, HudBits(Intra),
			Tuning.m_VelrampStart.Get(), Tuning.m_VelrampRange.Get(), Tuning.m_VelrampCurvature.Get(),
			HudBits(Info.m_Pos.x), HudBits(Info.m_Pos.y), HudBits(Info.m_Speed.x), HudBits(Info.m_Speed.y), HudBits(Info.m_Angle),
			Shown(Info.m_Pos.x, a[0], 64), Shown(Info.m_Pos.y, a[1], 64), Shown(Info.m_Speed.x, a[2], 64), Shown(Info.m_Speed.y, a[3], 64), Shown(Info.m_Angle, a[4], 64));
	}
	fclose(f);
	return 0;
}
