// "scoreboard" mode of rs_parity: the scoreboard, name plate and freeze bar logic
// (PLAN §13.2), for rust-client/client/tests/scoreboard.rs.
//
// These are CGameClient, CScoreboard, CNamePlates and CFreezeBars members, which need the
// whole client, so they are copied here word for word into stand-in classes of the same
// names (in their own namespace): the player sorts of OnNewSnapshot, GetScoreComparator,
// GetDDTeamColor, CScoreboard::IsActive and GetTeamName, the row and DDRace team walk of
// RenderScoreboard (its drawing left out between the two copies), RenderNamePlateGame and
// OnRender of the name plates, RenderFreezeBar, IsPlayerInfoAvailable and OnRender of the
// freeze bars. Each copy is marked "C++ copy: <file>:<lines>" ... "end of copy"; the Rust
// test `cpp_copies_match_source` checks that every copy is still identical (whitespace
// aside) to those source lines. What the copies call is the real code: CTeamsCore,
// CCharacterCore, the str_* functions, the colour conversions, Localize, g_Config.

#include <base/color.h>
#include <base/dbg.h>
#include <base/math.h>
#include <base/mem.h>
#include <base/str.h>
#include <base/vmath.h>

#include <engine/client.h>
#include <engine/graphics.h>
#include <engine/shared/config.h>
#include <engine/shared/protocol.h>

#include <game/gamecore.h>
#include <game/localization.h>
#include <game/teamscore.h>

#include <generated/protocol.h>
#include <generated/protocol7.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <functional>
#include <limits>
#include <memory>
#include <vector>

namespace rs_scoreboard {

static unsigned s_Seed = 1357;
static unsigned Rand()
{
	s_Seed = s_Seed * 1103515245u + 12345u;
	return (s_Seed >> 8) & 0xffffff;
}

static FILE *s_pOut = nullptr;

static void Hex(const char *pStr)
{
	if(!*pStr)
		fprintf(s_pOut, "-");
	for(const char *p = pStr; *p; p++)
		fprintf(s_pOut, "%02x", (unsigned char)*p);
}

static unsigned Bits(float f)
{
	unsigned u;
	mem_copy(&u, &f, sizeof(u));
	return u;
}

struct STranslationClient
{
	int m_PlayerFlags7 = 0;
};
struct STranslationContext
{
	STranslationClient m_aClients[MAX_CLIENTS];
};

struct SFakeClient
{
	int m_GameTick = 0;
	int m_PredGameTick = 0;
	int m_PredictionTick = 0;
	STranslationContext m_TranslationContext;
	bool IsSixup() const { return false; }
	int State() const { return IClient::STATE_ONLINE; }
	int GameTick(int Conn) const { return m_GameTick; }
	int PredGameTick(int Conn) const { return m_PredGameTick; }
	int GetPredictionTick() const { return m_PredictionTick; }
};

struct SFakeGraphics
{
	CScreenRect m_Screen = CScreenRect(0, 0, 1, 1);
	CScreenRect GetScreen() const { return m_Screen; }
};

struct SFakeTextRender
{
	void TextColor(ColorRGBA Color) {}
	ColorRGBA DefaultTextColor() const { return ColorRGBA(1, 1, 1, 1); }
	void Text(float x, float y, float Size, const char *pText)
	{
		fprintf(s_pOut, " L:");
		Hex(pText);
	}
	float TextWidth(float Size, const char *pText) const { return str_length(pText); }
};

// The rect operations the row walk uses; drawing a row prints its corners.
class CUIRect
{
public:
	float x = 0, y = 0, w = 0, h = 0;
	void HSplitTop(float Cut, CUIRect *pTop, CUIRect *pBottom) const
	{
		CUIRect r = *this;
		if(pBottom)
			fprintf(s_pOut, " R"); // a row (the row's own split has no bottom part)
		if(pTop)
			*pTop = {r.x, r.y, r.w, Cut};
		if(pBottom)
			*pBottom = {r.x, r.y + Cut, r.w, r.h - Cut};
	}
	void Draw(ColorRGBA Color, int Corners, float Rounding) const { fprintf(s_pOut, " C:%d", Corners); }
};

// no video is being rendered (the tool is built with CONF_VIDEORECORDER)
struct IVideo
{
	static IVideo *Current() { return nullptr; }
};

struct SMultiView
{
	bool IsActivated() const { return false; }
};
struct SStatboard
{
	bool m_Active = false;
	bool IsActive() const { return m_Active; }
};

// The members of the real CGameClient that the copies read.
class CGameClient
{
public:
	class CClientData
	{
	public:
		char m_aName[MAX_NAME_LENGTH] = "";
		char m_aClan[MAX_CLAN_LENGTH] = "";
		int m_Team = 0;
		bool m_Friend = false;
		int m_FinishTimeSeconds = FinishTime::UNSET;
		int m_FinishTimeMillis = 0;
		bool m_SpecCharPresent = false;
		vec2 m_SpecChar = vec2(0, 0);
		vec2 m_RenderPos = vec2(0, 0);
		CCharacterCore m_Predicted;
		bool m_IsPredicted = false;
		bool m_IsPredictedLocal = false;
		bool m_OtherTeam = false;
	};
	struct SGameInfo
	{
		bool m_TimeScore = false;
	};
	class CSnapState
	{
	public:
		class CCharacterInfo
		{
		public:
			bool m_Active = false;
			CNetObj_Character m_Cur = {};
			bool m_HasExtendedData = false;
			bool m_HasExtendedDisplayInfo = false;
			CNetObj_DDNetCharacter m_ExtendedData = {};
		};
		struct SSpecInfo
		{
			bool m_Active = false;
			int m_SpectatorId = SPEC_FREEVIEW;
		};
		const CNetObj_PlayerInfo *m_apPlayerInfos[MAX_CLIENTS] = {};
		const CNetObj_PlayerInfo *m_apPrevPlayerInfos[MAX_CLIENTS] = {};
		const CNetObj_PlayerInfo *m_apInfoByName[MAX_CLIENTS] = {};
		const CNetObj_PlayerInfo *m_apInfoByScore[MAX_CLIENTS] = {};
		const CNetObj_PlayerInfo *m_apInfoByDDTeamScore[MAX_CLIENTS] = {};
		const CNetObj_PlayerInfo *m_apInfoByDDTeamName[MAX_CLIENTS] = {};
		const CNetObj_PlayerInfo *m_pLocalInfo = nullptr;
		const CNetObj_Character *m_pLocalCharacter = nullptr;
		const CNetObj_GameInfo *m_pGameInfoObj = nullptr;
		int m_LocalClientId = -1;
		SSpecInfo m_SpecInfo;
		CCharacterInfo m_aCharacters[MAX_CLIENTS];
	};
	struct SControls
	{
		vec2 m_aTargetPos[NUM_DUMMIES] = {};
		CNetObj_PlayerInput m_aInputData[NUM_DUMMIES] = {};
	};

	CClientData m_aClients[MAX_CLIENTS];
	SGameInfo m_GameInfo;
	CSnapState m_Snap;
	CTeamsCore m_Teams;
	SControls m_Controls;
	SMultiView m_MultiView;
	SStatboard m_Statboard;
	int m_aLocalIds[NUM_DUMMIES] = {-1, -1};
	bool m_ReceivedDDNetPlayerFinishTimes = false;
	bool m_TeamPlay = false;
	int m_MaxTeamSize = 4;
	SFakeClient m_Client;

	SFakeClient *Client() { return &m_Client; }
	bool IsTeamPlay() const { return m_TeamPlay; }
	bool IsOtherTeam(int ClientId) const { return m_aClients[ClientId].m_OtherTeam; }
	int MaxTeamSize() const { return m_MaxTeamSize; }
	ColorRGBA GetDDTeamColor(int DDTeam, float Lightness = 0.5f) const;
	static std::function<bool(int, int, int, int)> GetScoreComparator(bool TimeScore, bool ReceivedMillisecondFinishTimes, bool Race7);
	void SortPlayers();
};

// C++ copy: game/client/gameclient.cpp:1008-1014
ColorRGBA CGameClient::GetDDTeamColor(int DDTeam, float Lightness) const
{
	// Use golden angle to generate unique colors with distinct adjacent colors.
	// The first DDTeam (team 1) gets angle 0°, i.e. red hue.
	const float Hue = std::fmod((DDTeam - 1) * normalized_golden_angle, 1.0f);
	return color_cast<ColorRGBA>(ColorHSLA(Hue, 1.0f, Lightness));
}
// end of copy

void CGameClient::SortPlayers()
{
// C++ copy: game/client/gameclient.cpp:2174-2222
	// sort player infos by name
	mem_copy(m_Snap.m_apInfoByName, m_Snap.m_apPlayerInfos, sizeof(m_Snap.m_apInfoByName));
	std::stable_sort(m_Snap.m_apInfoByName, m_Snap.m_apInfoByName + MAX_CLIENTS,
		[this](const CNetObj_PlayerInfo *pPlayer1, const CNetObj_PlayerInfo *pPlayer2) -> bool {
			if(!pPlayer2)
				return static_cast<bool>(pPlayer1);
			if(!pPlayer1)
				return false;
			return str_comp_nocase(m_aClients[pPlayer1->m_ClientId].m_aName, m_aClients[pPlayer2->m_ClientId].m_aName) < 0;
		});

	bool TimeScore = m_GameInfo.m_TimeScore;
	bool Race7 = Client()->IsSixup() && m_Snap.m_pGameInfoObj && m_Snap.m_pGameInfoObj->m_GameFlags & protocol7::GAMEFLAG_RACE;

	// sort player infos by score
	mem_copy(m_Snap.m_apInfoByScore, m_Snap.m_apInfoByName, sizeof(m_Snap.m_apInfoByScore));
	auto TimeComparator = CGameClient::GetScoreComparator(TimeScore, m_ReceivedDDNetPlayerFinishTimes, Race7);
	auto SortByTimeScore = [TimeComparator, this](const CNetObj_PlayerInfo *pPlayer1, const CNetObj_PlayerInfo *pPlayer2) -> bool {
		if(!pPlayer2)
			return static_cast<bool>(pPlayer1);
		if(!pPlayer1)
			return false;
		if(m_ReceivedDDNetPlayerFinishTimes)
			return TimeComparator(m_aClients[pPlayer1->m_ClientId].m_FinishTimeSeconds, m_aClients[pPlayer2->m_ClientId].m_FinishTimeSeconds, m_aClients[pPlayer1->m_ClientId].m_FinishTimeMillis, m_aClients[pPlayer2->m_ClientId].m_FinishTimeMillis);
		return TimeComparator(pPlayer1->m_Score, pPlayer2->m_Score, 0, 0);
	};
	std::stable_sort(m_Snap.m_apInfoByScore, m_Snap.m_apInfoByScore + MAX_CLIENTS, SortByTimeScore);

	// sort player infos by DDRace Team (and score between)
	int Index = 0;
	for(int Team = TEAM_FLOCK; Team < NUM_DDRACE_TEAMS; ++Team)
	{
		for(int i = 0; i < MAX_CLIENTS && Index < MAX_CLIENTS; ++i)
		{
			if(m_Snap.m_apInfoByScore[i] && m_Teams.Team(m_Snap.m_apInfoByScore[i]->m_ClientId) == Team)
				m_Snap.m_apInfoByDDTeamScore[Index++] = m_Snap.m_apInfoByScore[i];
		}
	}

	// sort player infos by DDRace Team (and name between)
	Index = 0;
	for(int Team = TEAM_FLOCK; Team < NUM_DDRACE_TEAMS; ++Team)
	{
		for(int i = 0; i < MAX_CLIENTS && Index < MAX_CLIENTS; ++i)
		{
			if(m_Snap.m_apInfoByName[i] && m_Teams.Team(m_Snap.m_apInfoByName[i]->m_ClientId) == Team)
				m_Snap.m_apInfoByDDTeamName[Index++] = m_Snap.m_apInfoByName[i];
		}
	}
// end of copy
}

// C++ copy: game/client/gameclient.cpp:2479-2521
std::function<bool(int, int, int, int)> CGameClient::GetScoreComparator(bool TimeScore, bool ReceivedMillisecondFinishTimes, bool Race7)
{
	// 0.7 race score
	if(Race7)
	{
		auto CompareTimeMillis07 = [](int TimeMillis1, int TimeMillis2, int, int) {
			TimeMillis1 = TimeMillis1 == protocol7::FinishTime::NOT_FINISHED ? std::numeric_limits<int>::max() : TimeMillis1;
			TimeMillis2 = TimeMillis2 == protocol7::FinishTime::NOT_FINISHED ? std::numeric_limits<int>::max() : TimeMillis2;
			return TimeMillis1 < TimeMillis2;
		};
		return CompareTimeMillis07;
	}

	// normal scores (like points), biggest score is highest in scoreboard
	if(!TimeScore)
	{
		auto CompareScore = [](int Score1, int Score2, int, int) {
			return Score1 > Score2;
		};
		return CompareScore;
	}

	// 'classical' times, times are send negative, so biggest value has shortest time
	if(!ReceivedMillisecondFinishTimes)
	{
		auto CompareTimeScore = [](int TimeScore1, int TimeScore2, int, int) {
			TimeScore1 = TimeScore1 == FinishTime::NOT_FINISHED_TIMESCORE ? std::numeric_limits<int>::min() : TimeScore1;
			TimeScore2 = TimeScore2 == FinishTime::NOT_FINISHED_TIMESCORE ? std::numeric_limits<int>::min() : TimeScore2;
			return TimeScore1 > TimeScore2;
		};
		return CompareTimeScore;
	}

	// long precise times, smallest value first, subsorting by milliseconds
	auto CompareTimeMillis = [](int TimeSeconds1, int TimeSeconds2, int TimeMillis1, int TimeMillis2) {
		TimeSeconds1 = TimeSeconds1 == FinishTime::NOT_FINISHED_MILLIS ? std::numeric_limits<int>::max() : TimeSeconds1;
		TimeSeconds2 = TimeSeconds2 == FinishTime::NOT_FINISHED_MILLIS ? std::numeric_limits<int>::max() : TimeSeconds2;
		if(TimeSeconds1 == TimeSeconds2)
			return TimeMillis1 < TimeMillis2;
		return TimeSeconds1 < TimeSeconds2;
	};
	return CompareTimeMillis;
}
// end of copy

// The members of CComponent the copies use.
class CComponentStandIn
{
public:
	CGameClient *m_pGameClient = nullptr;
	SFakeGraphics m_Graphics;
	SFakeTextRender m_TextRender;
	CGameClient *GameClient() const { return m_pGameClient; }
	SFakeClient *Client() const { return &m_pGameClient->m_Client; }
	SFakeGraphics *Graphics() { return &m_Graphics; }
	SFakeTextRender *TextRender() { return &m_TextRender; }
};

class CScoreboard : public CComponentStandIn
{
public:
	struct CScoreboardRenderState
	{
		float m_TeamStartX;
		float m_TeamStartY;
		int m_CurrentDDTeamSize;

		CScoreboardRenderState() :
			m_TeamStartX(0), m_TeamStartY(0), m_CurrentDDTeamSize(0) {}
	};
	bool m_Active = false;
	bool IsActive() const;
	const char *GetTeamName(int Team) const;
	void RenderScoreboardRows(CUIRect Scoreboard, int Team, int CountStart, int CountEnd, CScoreboardRenderState &State, int NumPlayers);
};

// C++ copy: game/client/components/scoreboard.cpp:1206-1260
bool CScoreboard::IsActive() const
{
	// if statboard is active don't show scoreboard
	if(GameClient()->m_Statboard.IsActive())
		return false;

	if(m_Active)
		return true;

	const CNetObj_GameInfo *pGameInfoObj = GameClient()->m_Snap.m_pGameInfoObj;
	if(GameClient()->m_Snap.m_pLocalInfo && !GameClient()->m_Snap.m_SpecInfo.m_Active)
	{
		// we are not a spectator, check if we are dead and the game isn't paused
		if(!GameClient()->m_Snap.m_pLocalCharacter && g_Config.m_ClScoreboardOnDeath &&
			!(pGameInfoObj && pGameInfoObj->m_GameStateFlags & GAMESTATEFLAG_PAUSED))
			return true;
	}

	// if the game is over
	if(pGameInfoObj && pGameInfoObj->m_GameStateFlags & GAMESTATEFLAG_GAMEOVER)
		return true;

	return false;
}

const char *CScoreboard::GetTeamName(int Team) const
{
	dbg_assert(Team == TEAM_RED || Team == TEAM_BLUE, "Team invalid");

	int ClanPlayers = 0;
	const char *pClanName = nullptr;
	for(const CNetObj_PlayerInfo *pInfo : GameClient()->m_Snap.m_apInfoByScore)
	{
		if(!pInfo || pInfo->m_Team != Team)
			continue;

		if(!pClanName)
		{
			pClanName = GameClient()->m_aClients[pInfo->m_ClientId].m_aClan;
			ClanPlayers++;
		}
		else
		{
			if(str_comp(GameClient()->m_aClients[pInfo->m_ClientId].m_aClan, pClanName) == 0)
				ClanPlayers++;
			else
				return nullptr;
		}
	}

	if(ClanPlayers > 1 && pClanName[0] != '\0')
		return pClanName;
	else
		return nullptr;
}
// end of copy

// RenderScoreboard (scoreboard.cpp:489) without its measurements and drawing.
void CScoreboard::RenderScoreboardRows(CUIRect Scoreboard, int Team, int CountStart, int CountEnd, CScoreboardRenderState &State, int NumPlayers)
{
	const float LineHeight = 20.0f, Spacing = 0.0f, RoundRadius = 2.5f, FontSize = 12.0f;
// C++ copy: game/client/components/scoreboard.cpp:606-710
	// render player entries
	int CountRendered = 0;
	int PrevDDTeam = -1;
	int &CurrentDDTeamSize = State.m_CurrentDDTeamSize;

	char aBuf[64];
	int MaxTeamSize = GameClient()->MaxTeamSize();

	for(int RenderDead = 0; RenderDead < 2; RenderDead++)
	{
		for(int i = 0; i < MAX_CLIENTS; i++)
		{
			// make sure that we render the correct team
			const CNetObj_PlayerInfo *pInfo = GameClient()->m_Snap.m_apInfoByDDTeamScore[i];
			if(!pInfo || pInfo->m_Team != Team)
				continue;
			bool IsDead = Client()->m_TranslationContext.m_aClients[pInfo->m_ClientId].m_PlayerFlags7 & protocol7::PLAYERFLAG_DEAD;
			if(!RenderDead && IsDead)
				continue;
			if(RenderDead && !IsDead)
				continue;
			if(CountRendered++ < CountStart)
				continue;

			int DDTeam = GameClient()->m_Teams.Team(pInfo->m_ClientId);
			int NextDDTeam = 0;

			ColorRGBA TextColor = TextRender()->DefaultTextColor();
			TextColor.a = RenderDead ? 0.5f : 1.0f;
			TextRender()->TextColor(TextColor);

			for(int j = i + 1; j < MAX_CLIENTS; j++)
			{
				const CNetObj_PlayerInfo *pInfoNext = GameClient()->m_Snap.m_apInfoByDDTeamScore[j];
				if(!pInfoNext || pInfoNext->m_Team != Team)
					continue;

				NextDDTeam = GameClient()->m_Teams.Team(pInfoNext->m_ClientId);
				break;
			}

			if(PrevDDTeam == -1)
			{
				for(int j = i - 1; j >= 0; j--)
				{
					const CNetObj_PlayerInfo *pInfoPrev = GameClient()->m_Snap.m_apInfoByDDTeamScore[j];
					if(!pInfoPrev || pInfoPrev->m_Team != Team)
						continue;

					PrevDDTeam = GameClient()->m_Teams.Team(pInfoPrev->m_ClientId);
					break;
				}
			}

			CUIRect RowAndSpacing, Row;
			Scoreboard.HSplitTop(LineHeight + Spacing, &RowAndSpacing, &Scoreboard);
			RowAndSpacing.HSplitTop(LineHeight, &Row, nullptr);
			const float TextY = Row.y + (Row.h - FontSize) / 2.0f;

			// team background
			if(DDTeam != TEAM_FLOCK)
			{
				const ColorRGBA Color = GameClient()->GetDDTeamColor(DDTeam).WithAlpha(0.5f);
				int TeamRectCorners = 0;
				if(PrevDDTeam != DDTeam)
				{
					TeamRectCorners |= IGraphics::CORNER_T;
					State.m_TeamStartX = Row.x;
					State.m_TeamStartY = Row.y;
				}
				if(NextDDTeam != DDTeam)
					TeamRectCorners |= IGraphics::CORNER_B;
				RowAndSpacing.Draw(Color, TeamRectCorners, RoundRadius);

				CurrentDDTeamSize++;

				if(NextDDTeam != DDTeam)
				{
					const float TeamFontSize = FontSize / 1.5f;

					if(NumPlayers > 8)
					{
						if(DDTeam == GameClient()->m_Teams.TeamSuper())
							str_copy(aBuf, Localize("Super"));
						else if(CurrentDDTeamSize <= 1)
							str_format(aBuf, sizeof(aBuf), "%d", DDTeam);
						else
							str_format(aBuf, sizeof(aBuf), Localize("%d\n(%d/%d)", "Team and size"), DDTeam, CurrentDDTeamSize, MaxTeamSize);
						TextRender()->Text(State.m_TeamStartX, std::max(State.m_TeamStartY + Row.h / 2.0f - TeamFontSize, State.m_TeamStartY + 1.5f /* padding top */), TeamFontSize, aBuf);
					}
					else
					{
						if(DDTeam == GameClient()->m_Teams.TeamSuper())
							str_copy(aBuf, Localize("Super"));
						else if(CurrentDDTeamSize > 1)
							str_format(aBuf, sizeof(aBuf), Localize("Team %d (%d/%d)"), DDTeam, CurrentDDTeamSize, MaxTeamSize);
						else
							str_format(aBuf, sizeof(aBuf), Localize("Team %d"), DDTeam);
						TextRender()->Text(Row.x + Row.w / 2.0f - TextRender()->TextWidth(TeamFontSize, aBuf) / 2.0f + 5.0f, Row.y + Row.h, TeamFontSize, aBuf);
					}

					CurrentDDTeamSize = 0;
				}
			}
			PrevDDTeam = DDTeam;
// end of copy
// C++ copy: game/client/components/scoreboard.cpp:890-896
			if(CountRendered == CountEnd)
				break;
		}
		if(CountRendered == CountEnd)
			break;
	}
}
// end of copy

// C++ copy: game/client/components/nameplates.cpp:18-51
enum class EHookStrongWeakState
{
	WEAK,
	NEUTRAL,
	STRONG
};

class CNamePlateData
{
public:
	bool m_InGame;
	ColorRGBA m_Color;
	bool m_ShowName;
	char m_aName[std::max((size_t)MAX_NAME_LENGTH, (size_t)protocol7::MAX_NAME_ARRAY_SIZE)];
	bool m_ShowFriendMark;
	bool m_ShowClientId;
	int m_ClientId;
	float m_FontSizeClientId;
	bool m_ClientIdSeparateLine;
	float m_FontSize;
	bool m_ShowClan;
	char m_aClan[std::max((size_t)MAX_CLAN_LENGTH, (size_t)protocol7::MAX_CLAN_ARRAY_SIZE)];
	float m_FontSizeClan;
	bool m_ShowDirection;
	bool m_DirLeft;
	bool m_DirJump;
	bool m_DirRight;
	float m_FontSizeDirection;
	bool m_ShowHookStrongWeak;
	EHookStrongWeakState m_HookStrongWeakState;
	bool m_ShowHookStrongWeakId;
	int m_HookStrongWeakId;
	float m_FontSizeHookStrongWeak;
};
// end of copy

// The name plate drawn: its data and position are printed.
class CNamePlate
{
public:
	CNamePlateData m_Data;
	void Update(CGameClient &This, const CNamePlateData &Data) { m_Data = Data; }
	void Render(CGameClient &This, vec2 Position)
	{
		const CNamePlateData &D = m_Data;
		fprintf(s_pOut, " plate %d %08x %08x %08x %08x %08x %08x %d ", D.m_ClientId, Bits(Position.x), Bits(Position.y),
			Bits(D.m_Color.r), Bits(D.m_Color.g), Bits(D.m_Color.b), Bits(D.m_Color.a), D.m_ShowName);
		Hex(D.m_aName);
		fprintf(s_pOut, " %d %d %08x %d %08x %d ", D.m_ShowFriendMark, D.m_ShowClientId, Bits(D.m_FontSizeClientId), D.m_ClientIdSeparateLine,
			Bits(D.m_FontSize), D.m_ShowClan);
		Hex(D.m_aClan);
		fprintf(s_pOut, " %08x %d %d %d %d %08x %d %d %d %d %08x\n", Bits(D.m_FontSizeClan), D.m_ShowDirection, D.m_DirLeft, D.m_DirJump,
			D.m_DirRight, Bits(D.m_FontSizeDirection), D.m_ShowHookStrongWeak, (int)D.m_HookStrongWeakState, D.m_ShowHookStrongWeakId,
			D.m_HookStrongWeakId, Bits(D.m_FontSizeHookStrongWeak));
	}
};

class CNamePlates : public CComponentStandIn
{
public:
	class CNamePlatesData
	{
	public:
		CNamePlate m_aNamePlates[MAX_CLIENTS];
	};
	std::unique_ptr<CNamePlatesData> m_pData = std::make_unique<CNamePlatesData>();
	void RenderNamePlateGame(vec2 Position, const CNetObj_PlayerInfo *pPlayerInfo, float Alpha);
	void OnRender();
};

// C++ copy: game/client/components/nameplates.cpp:625-767
void CNamePlates::RenderNamePlateGame(vec2 Position, const CNetObj_PlayerInfo *pPlayerInfo, float Alpha)
{
	// Get screen edges to avoid rendering offscreen
	CScreenRect ScreenRect = Graphics()->GetScreen();

	// Assume that the name plate fits into a 800x800 box placed directly above the tee
	ScreenRect.m_TopLeft.x -= 400;
	ScreenRect.m_BottomRight.x += 400;
	ScreenRect.m_BottomRight.y += 800;
	if(!ScreenRect.Inside(Position))
		return;

	CNamePlateData Data;

	const auto &ClientData = GameClient()->m_aClients[pPlayerInfo->m_ClientId];
	const bool OtherTeam = GameClient()->IsOtherTeam(pPlayerInfo->m_ClientId);

	Data.m_InGame = true;

	Data.m_ShowName = pPlayerInfo->m_Local ? g_Config.m_ClNamePlatesOwn : g_Config.m_ClNamePlates;
	str_copy(Data.m_aName, GameClient()->m_aClients[pPlayerInfo->m_ClientId].m_aName);
	Data.m_ShowFriendMark = Data.m_ShowName && g_Config.m_ClNamePlatesFriendMark && GameClient()->m_aClients[pPlayerInfo->m_ClientId].m_Friend;
	Data.m_ShowClientId = Data.m_ShowName && (g_Config.m_Debug || g_Config.m_ClNamePlatesIds);
	Data.m_FontSize = 18.0f + 20.0f * g_Config.m_ClNamePlatesSize / 100.0f;

	Data.m_ClientId = pPlayerInfo->m_ClientId;
	Data.m_ClientIdSeparateLine = g_Config.m_ClNamePlatesIdsSeparateLine;
	Data.m_FontSizeClientId = Data.m_ClientIdSeparateLine ? (18.0f + 20.0f * g_Config.m_ClNamePlatesIdsSize / 100.0f) : Data.m_FontSize;

	Data.m_ShowClan = Data.m_ShowName && g_Config.m_ClNamePlatesClan;
	str_copy(Data.m_aClan, GameClient()->m_aClients[pPlayerInfo->m_ClientId].m_aClan);
	Data.m_FontSizeClan = 18.0f + 20.0f * g_Config.m_ClNamePlatesClanSize / 100.0f;

	Data.m_FontSizeHookStrongWeak = 18.0f + 20.0f * g_Config.m_ClNamePlatesStrongSize / 100.0f;
	Data.m_FontSizeDirection = 18.0f + 20.0f * g_Config.m_ClDirectionSize / 100.0f;

	if(g_Config.m_ClNamePlatesAlways == 0)
		Alpha *= std::clamp(1.0f - std::pow(distance(GameClient()->m_Controls.m_aTargetPos[g_Config.m_ClDummy], Position) / 200.0f, 16.0f), 0.0f, 1.0f);
	if(OtherTeam)
		Alpha *= (float)g_Config.m_ClShowOthersAlpha / 100.0f;

	Data.m_Color = ColorRGBA(1.0f, 1.0f, 1.0f);
	if(g_Config.m_ClNamePlatesTeamcolors)
	{
		if(GameClient()->IsTeamPlay())
		{
			if(ClientData.m_Team == TEAM_RED)
				Data.m_Color = ColorRGBA(1.0f, 0.5f, 0.5f);
			else if(ClientData.m_Team == TEAM_BLUE)
				Data.m_Color = ColorRGBA(0.7f, 0.7f, 1.0f);
		}
		else
		{
			const int Team = GameClient()->m_Teams.Team(pPlayerInfo->m_ClientId);
			if(Team)
				Data.m_Color = GameClient()->GetDDTeamColor(Team, 0.75f);
		}
	}
	Data.m_Color.a = Alpha;

	int ShowDirectionConfig = g_Config.m_ClShowDirection;
#if defined(CONF_VIDEORECORDER)
	if(IVideo::Current())
		ShowDirectionConfig = g_Config.m_ClVideoShowDirection;
#endif
	Data.m_DirLeft = Data.m_DirJump = Data.m_DirRight = false;
	switch(ShowDirectionConfig)
	{
	case 0: // Off
		Data.m_ShowDirection = false;
		break;
	case 1: // Others
		Data.m_ShowDirection = !pPlayerInfo->m_Local;
		break;
	case 2: // Everyone
		Data.m_ShowDirection = true;
		break;
	case 3: // Only self
		Data.m_ShowDirection = pPlayerInfo->m_Local;
		break;
	default:
		dbg_assert_failed("ShowDirectionConfig invalid");
	}
	if(Data.m_ShowDirection)
	{
		if(Client()->State() != IClient::STATE_DEMOPLAYBACK &&
			pPlayerInfo->m_ClientId == GameClient()->m_aLocalIds[!g_Config.m_ClDummy])
		{
			const auto &InputData = GameClient()->m_Controls.m_aInputData[!g_Config.m_ClDummy];
			Data.m_DirLeft = InputData.m_Direction == -1;
			Data.m_DirJump = InputData.m_Jump == 1;
			Data.m_DirRight = InputData.m_Direction == 1;
		}
		else if(Client()->State() != IClient::STATE_DEMOPLAYBACK && pPlayerInfo->m_Local) // Always render local input when not in demo playback
		{
			const auto &InputData = GameClient()->m_Controls.m_aInputData[g_Config.m_ClDummy];
			Data.m_DirLeft = InputData.m_Direction == -1;
			Data.m_DirJump = InputData.m_Jump == 1;
			Data.m_DirRight = InputData.m_Direction == 1;
		}
		else
		{
			const auto &Character = GameClient()->m_Snap.m_aCharacters[pPlayerInfo->m_ClientId];
			Data.m_DirLeft = Character.m_Cur.m_Direction == -1;
			Data.m_DirJump = Character.m_Cur.m_Jumped & 1;
			Data.m_DirRight = Character.m_Cur.m_Direction == 1;
		}
	}

	Data.m_ShowHookStrongWeak = false;
	Data.m_HookStrongWeakState = EHookStrongWeakState::NEUTRAL;
	Data.m_ShowHookStrongWeakId = false;
	Data.m_HookStrongWeakId = 0;

	const bool Following = (GameClient()->m_Snap.m_SpecInfo.m_Active && !GameClient()->m_MultiView.IsActivated() && GameClient()->m_Snap.m_SpecInfo.m_SpectatorId != SPEC_FREEVIEW);
	if(GameClient()->m_Snap.m_LocalClientId != -1 || Following)
	{
		const int SelectedId = Following ? GameClient()->m_Snap.m_SpecInfo.m_SpectatorId : GameClient()->m_Snap.m_LocalClientId;
		const CGameClient::CSnapState::CCharacterInfo &Selected = GameClient()->m_Snap.m_aCharacters[SelectedId];
		const CGameClient::CSnapState::CCharacterInfo &Other = GameClient()->m_Snap.m_aCharacters[pPlayerInfo->m_ClientId];

		if((Selected.m_HasExtendedData || GameClient()->m_aClients[SelectedId].m_SpecCharPresent) && Other.m_HasExtendedData)
		{
			int SelectedStrongWeakId = Selected.m_HasExtendedData ? Selected.m_ExtendedData.m_StrongWeakId : 0;
			Data.m_HookStrongWeakId = Other.m_ExtendedData.m_StrongWeakId;
			Data.m_ShowHookStrongWeakId = g_Config.m_Debug || g_Config.m_ClNamePlatesStrong == 2;
			if(SelectedId == pPlayerInfo->m_ClientId)
			{
				Data.m_ShowHookStrongWeak = Data.m_ShowHookStrongWeakId;
			}
			else
			{
				Data.m_HookStrongWeakState = SelectedStrongWeakId > Other.m_ExtendedData.m_StrongWeakId ? EHookStrongWeakState::STRONG : EHookStrongWeakState::WEAK;
				Data.m_ShowHookStrongWeak = g_Config.m_Debug || g_Config.m_ClNamePlatesStrong > 0;
			}
		}
	}

	// Check if the nameplate is actually on screen
	CNamePlate &NamePlate = m_pData->m_aNamePlates[pPlayerInfo->m_ClientId];
	NamePlate.Update(*GameClient(), Data);
	NamePlate.Render(*GameClient(), Position - vec2(0.0f, (float)g_Config.m_ClNamePlatesOffset));
}
// end of copy

// C++ copy: game/client/components/nameplates.cpp:856-888
void CNamePlates::OnRender()
{
	if(Client()->State() != IClient::STATE_ONLINE && Client()->State() != IClient::STATE_DEMOPLAYBACK)
		return;

	int ShowDirection = g_Config.m_ClShowDirection;
#if defined(CONF_VIDEORECORDER)
	if(IVideo::Current())
		ShowDirection = g_Config.m_ClVideoShowDirection;
#endif
	if(!g_Config.m_ClNamePlates && !g_Config.m_ClNamePlatesOwn && ShowDirection == 0)
		return;

	for(int i = 0; i < MAX_CLIENTS; i++)
	{
		const CNetObj_PlayerInfo *pInfo = GameClient()->m_Snap.m_apPlayerInfos[i];
		if(!pInfo)
			continue;

		// Each player can also have a spectator char whose name plate is displayed independently
		if(GameClient()->m_aClients[i].m_SpecCharPresent)
		{
			const vec2 RenderPos = GameClient()->m_aClients[i].m_SpecChar;
			RenderNamePlateGame(RenderPos, pInfo, 0.4f);
		}
		// Only render name plates for active characters
		if(GameClient()->m_Snap.m_aCharacters[i].m_Active)
		{
			const vec2 RenderPos = GameClient()->m_aClients[i].m_RenderPos;
			RenderNamePlateGame(RenderPos, pInfo, 1.0f);
		}
	}
}
// end of copy

class CFreezeBars : public CComponentStandIn
{
public:
	void RenderFreezeBar(const int ClientId);
	void RenderFreezeBarPos(float x, const float y, const float Width, const float Height, float Progress, const float Alpha)
	{
		fprintf(s_pOut, " bar %08x %08x %08x %08x %08x %08x", Bits(x), Bits(y), Bits(Width), Bits(Height), Bits(Progress), Bits(Alpha));
	}
	bool IsPlayerInfoAvailable(int ClientId) const;
	void OnRender();
};

// C++ copy: game/client/components/freezebars.cpp:5-54
void CFreezeBars::RenderFreezeBar(const int ClientId)
{
	const float FreezeBarWidth = 64.0f;
	const float FreezeBarHalfWidth = 32.0f;
	const float FreezeBarHeight = 16.0f;

	// pCharacter contains the predicted character for local players or the last snap for players who are spectated
	CCharacterCore *pCharacter = &GameClient()->m_aClients[ClientId].m_Predicted;

	if(pCharacter->m_FreezeEnd <= 0 || pCharacter->m_FreezeStart == 0 || pCharacter->m_FreezeEnd <= pCharacter->m_FreezeStart || !GameClient()->m_Snap.m_aCharacters[ClientId].m_HasExtendedDisplayInfo || (pCharacter->m_IsInFreeze && g_Config.m_ClFreezeBarsAlphaInsideFreeze == 0))
	{
		return;
	}

	// only show the freeze bar once the server confirmed the freeze, so mispredicted freezes are not shown
	if(GameClient()->m_Snap.m_aCharacters[ClientId].m_ExtendedData.m_FreezeEnd <= 0)
	{
		return;
	}

	// the freeze ticks of predicted characters are in predicted time, so compare them against the tick they were predicted to
	int Tick = Client()->GameTick(g_Config.m_ClDummy);
	if(GameClient()->m_aClients[ClientId].m_IsPredictedLocal)
	{
		Tick = Client()->PredGameTick(g_Config.m_ClDummy);
	}
	else if(GameClient()->m_aClients[ClientId].m_IsPredicted)
	{
		Tick = Client()->GetPredictionTick();
	}

	const int64_t Max = (int64_t)pCharacter->m_FreezeEnd - pCharacter->m_FreezeStart;
	float FreezeProgress = std::clamp<int64_t>(Max - ((int64_t)Tick - pCharacter->m_FreezeStart), 0, Max) / (float)Max;
	if(FreezeProgress <= 0.0f)
	{
		return;
	}

	vec2 Position = GameClient()->m_aClients[ClientId].m_RenderPos;
	Position.x -= FreezeBarHalfWidth;
	Position.y += 32;

	float Alpha = GameClient()->IsOtherTeam(ClientId) ? g_Config.m_ClShowOthersAlpha / 100.0f : 1.0f;
	if(pCharacter->m_IsInFreeze)
	{
		Alpha *= g_Config.m_ClFreezeBarsAlphaInsideFreeze / 100.0f;
	}

	RenderFreezeBarPos(Position.x, Position.y, FreezeBarWidth, FreezeBarHeight, FreezeProgress, Alpha);
}
// end of copy

// C++ copy: game/client/components/freezebars.cpp:207-254
inline bool CFreezeBars::IsPlayerInfoAvailable(int ClientId) const
{
	return GameClient()->m_Snap.m_aCharacters[ClientId].m_Active &&
	       GameClient()->m_Snap.m_apPrevPlayerInfos[ClientId] != nullptr &&
	       GameClient()->m_Snap.m_apPlayerInfos[ClientId] != nullptr;
}

void CFreezeBars::OnRender()
{
	if(Client()->State() != IClient::STATE_ONLINE && Client()->State() != IClient::STATE_DEMOPLAYBACK)
		return;

	if(!g_Config.m_ClShowFreezeBars)
	{
		return;
	}
	// get screen edges to avoid rendering offscreen
	CScreenRect ScreenRect = Graphics()->GetScreen();
	// expand the edges to prevent popping in/out onscreen
	//
	// it is assumed that the tee with the freeze bar fit into a 240x240 box centered on the tee
	// this may need to be changed or calculated differently in the future
	constexpr float BorderBuffer = 120.0f;
	ScreenRect.Expand(BorderBuffer);

	int LocalClientId = GameClient()->m_Snap.m_LocalClientId;

	// render everyone else's freeze bar, then our own
	for(int ClientId = 0; ClientId < MAX_CLIENTS; ClientId++)
	{
		if(ClientId == LocalClientId || !IsPlayerInfoAvailable(ClientId))
		{
			continue;
		}

		//don't render if the tee is offscreen
		if(!ScreenRect.Inside(GameClient()->m_aClients[ClientId].m_RenderPos))
		{
			continue;
		}

		RenderFreezeBar(ClientId);
	}
	if(LocalClientId != -1 && IsPlayerInfoAvailable(LocalClientId))
	{
		RenderFreezeBar(LocalClientId);
	}
}
// end of copy

static const char *const s_apNames[] = {"nameless tee", "Tee", "tee", "Жук", "a b", "brainless tee", "ab", "A", "Ölf", "x.y", "ninja", "Gruß", "zz"};
static const char *const s_apClans[] = {"", "", "DDNet", "ddnet", "Clan", "Жук"};

static void DumpList(const char *pName, const CNetObj_PlayerInfo *const *ppList)
{
	fprintf(s_pOut, " %s", pName);
	for(int i = 0; i < MAX_CLIENTS; i++)
		if(ppList[i])
			fprintf(s_pOut, " %d", ppList[i]->m_ClientId);
	fprintf(s_pOut, " |");
}

static int Run(FILE *f)
{
	s_pOut = f;
	const int Runs = 300;
	fprintf(f, "scoreboard %d\n", Runs);
	for(int r = 0; r < Runs; r++)
	{
		auto pGameClient = std::make_unique<CGameClient>();
		CGameClient &G = *pGameClient;
		static CNetObj_PlayerInfo s_aInfos[MAX_CLIENTS], s_aPrevInfos[MAX_CLIENTS];
		static CNetObj_GameInfo s_GameInfo;
		static CNetObj_Character s_LocalChar;

		G.m_GameInfo.m_TimeScore = Rand() % 2;
		G.m_ReceivedDDNetPlayerFinishTimes = Rand() % 2;
		G.m_TeamPlay = Rand() % 4 == 0;
		G.m_MaxTeamSize = 2 + Rand() % 6;
		const int NumPlayers = Rand() % 4 == 0 ? 20 + Rand() % 50 : Rand() % 12;
		fprintf(f, "run %d %d %d %d %d\n", r, G.m_GameInfo.m_TimeScore, G.m_ReceivedDDNetPlayerFinishTimes, G.m_TeamPlay, G.m_MaxTeamSize);
		std::vector<int> vIds;
		for(int i = 0; i < NumPlayers; i++)
		{
			int Id;
			do
				Id = Rand() % MAX_CLIENTS;
			while(std::find(vIds.begin(), vIds.end(), Id) != vIds.end());
			vIds.push_back(Id);
			CGameClient::CClientData &C = G.m_aClients[Id];
			str_copy(C.m_aName, s_apNames[Rand() % std::size(s_apNames)]);
			str_copy(C.m_aClan, s_apClans[Rand() % std::size(s_apClans)]);
			C.m_Friend = Rand() % 4 == 0;
			C.m_OtherTeam = Rand() % 3 == 0;
			const int Kind = Rand() % 6;
			C.m_FinishTimeSeconds = Kind == 0 ? FinishTime::NOT_FINISHED_MILLIS : Kind == 1 ? (int)(Rand() % 4000) : Kind == 2 ? 3500 + (int)(Rand() % 400) : (int)(Rand() % 300);
			C.m_FinishTimeMillis = Rand() % 1000;
			CNetObj_PlayerInfo &Info = s_aInfos[Id];
			Info.m_Local = 0;
			Info.m_ClientId = Id;
			Info.m_Team = Rand() % 5 == 0 ? TEAM_SPECTATORS : (G.m_TeamPlay ? (int)(Rand() % 2) : TEAM_RED);
			C.m_Team = Info.m_Team;
			const int ScoreKind = Rand() % 4;
			Info.m_Score = ScoreKind == 0 ? FinishTime::NOT_FINISHED_TIMESCORE : ScoreKind == 1 ? -(int)(Rand() % 5000) : (int)(Rand() % 200) - 50;
			if(Rand() % 5 == 0 && !vIds.empty())
				Info.m_Score = s_aInfos[vIds[0]].m_Score; // ties
			Info.m_Latency = Rand() % 1200;
			G.m_Snap.m_apPlayerInfos[Id] = &Info;
			s_aPrevInfos[Id] = Info;
			G.m_Snap.m_apPrevPlayerInfos[Id] = Rand() % 8 ? &s_aPrevInfos[Id] : nullptr;
			const int DDTeam = Rand() % 3 ? 0 : (Rand() % 8 == 0 ? (int)TEAM_SUPER : 1 + (int)(Rand() % 4));
			G.m_Teams.Team(Id, DDTeam);
			fprintf(f, "player %d %d %d %d ", Id, Info.m_Team, Info.m_Score, Info.m_Latency);
			Hex(C.m_aName);
			fprintf(f, " ");
			Hex(C.m_aClan);
			fprintf(f, " %d %d %d %d %d\n", C.m_FinishTimeSeconds, C.m_FinishTimeMillis, DDTeam, C.m_Friend, C.m_OtherTeam);
		}
		const int Local = vIds.empty() || Rand() % 6 == 0 ? -1 : vIds[Rand() % vIds.size()];
		G.m_Snap.m_LocalClientId = Local;
		G.m_aLocalIds[0] = Local;
		if(Local >= 0)
		{
			s_aInfos[Local].m_Local = 1;
			G.m_Snap.m_pLocalInfo = &s_aInfos[Local];
		}

		fprintf(f, "local %d\n", Local);
		G.SortPlayers();
		fprintf(f, "sort");
		DumpList("name", G.m_Snap.m_apInfoByName);
		DumpList("score", G.m_Snap.m_apInfoByScore);
		DumpList("ddscore", G.m_Snap.m_apInfoByDDTeamScore);
		DumpList("ddname", G.m_Snap.m_apInfoByDDTeamName);
		fprintf(f, "\n");

		CScoreboard Board;
		Board.m_pGameClient = &G;
		for(int k = 0; k < 4; k++)
		{
			Board.m_Active = Rand() % 2;
			G.m_Statboard.m_Active = Rand() % 6 == 0;
			G.m_Snap.m_SpecInfo.m_Active = Rand() % 4 == 0;
			G.m_Snap.m_pLocalCharacter = Rand() % 2 ? &s_LocalChar : nullptr;
			g_Config.m_ClScoreboardOnDeath = Rand() % 3 != 0;
			s_GameInfo.m_GameStateFlags = (Rand() % 3 == 0 ? GAMESTATEFLAG_GAMEOVER : 0) | (Rand() % 3 == 0 ? GAMESTATEFLAG_PAUSED : 0);
			G.m_Snap.m_pGameInfoObj = Rand() % 5 ? &s_GameInfo : nullptr;
			fprintf(f, "active %d %d %d %d %d %d %d %d = %d\n", Board.m_Active, G.m_Statboard.m_Active, G.m_Snap.m_pLocalInfo != nullptr,
				G.m_Snap.m_SpecInfo.m_Active, G.m_Snap.m_pLocalCharacter != nullptr, g_Config.m_ClScoreboardOnDeath,
				G.m_Snap.m_pGameInfoObj != nullptr, s_GameInfo.m_GameStateFlags, Board.IsActive());
		}
		for(int Team = TEAM_RED; Team <= TEAM_BLUE; Team++)
		{
			fprintf(f, "teamname %d ", Team);
			const char *pName = Board.GetTeamName(Team);
			if(pName)
				Hex(pName);
			else
				fprintf(f, "null");
			fprintf(f, "\n");
		}
		// rows, in columns like OnRender splits them
		for(int Team = TEAM_RED; Team <= (G.m_TeamPlay ? TEAM_BLUE : TEAM_RED); Team++)
		{
			const int Start = Rand() % 3 == 0 ? (int)(Rand() % 16) : 0;
			const int End = Start + 1 + Rand() % 40;
			const int Size = Rand() % 2 ? 6 : 20;
			CScoreboard::CScoreboardRenderState State;
			CUIRect Rect = {0, 0, 300, 400};
			fprintf(f, "rows %d %d %d %d =", Team, Start, End, Size);
			Board.RenderScoreboardRows(Rect, Team, Start, End, State, Size);
			fprintf(f, "\n");
		}

		// name plates
		CNamePlates Plates;
		Plates.m_pGameClient = &G;
		for(int k = 0; k < 3; k++)
		{
			g_Config.m_ClNamePlates = Rand() % 4 != 0;
			g_Config.m_ClNamePlatesOwn = Rand() % 2;
			g_Config.m_ClNamePlatesFriendMark = Rand() % 2;
			g_Config.m_ClNamePlatesIds = Rand() % 3 == 0;
			g_Config.m_ClNamePlatesSize = Rand() % 101 - 50;
			g_Config.m_ClNamePlatesIdsSeparateLine = Rand() % 2;
			g_Config.m_ClNamePlatesIdsSize = Rand() % 101 - 50;
			g_Config.m_ClNamePlatesClan = Rand() % 2;
			g_Config.m_ClNamePlatesClanSize = Rand() % 101 - 50;
			g_Config.m_ClNamePlatesStrong = Rand() % 3;
			g_Config.m_ClNamePlatesStrongSize = Rand() % 101 - 50;
			g_Config.m_ClDirectionSize = Rand() % 101 - 50;
			g_Config.m_ClNamePlatesAlways = Rand() % 2;
			g_Config.m_ClNamePlatesOffset = Rand() % 100;
			g_Config.m_ClShowOthersAlpha = Rand() % 101;
			g_Config.m_ClNamePlatesTeamcolors = Rand() % 2;
			g_Config.m_ClShowDirection = Rand() % 4;
			g_Config.m_Debug = Rand() % 10 == 0;
			const float ScreenX = (float)(Rand() % 3000), ScreenY = (float)(Rand() % 2000);
			Plates.m_Graphics.m_Screen = CScreenRect(ScreenX, ScreenY, 1200.0f, 700.0f);
			G.m_Controls.m_aTargetPos[0] = vec2(ScreenX + (Rand() % 1200), ScreenY + (Rand() % 700));
			G.m_Controls.m_aInputData[0].m_Direction = (int)(Rand() % 3) - 1;
			G.m_Controls.m_aInputData[0].m_Jump = Rand() % 2;
			G.m_Controls.m_aInputData[1].m_Direction = (int)(Rand() % 3) - 1;
			G.m_Controls.m_aInputData[1].m_Jump = Rand() % 2;
			G.m_aLocalIds[1] = Rand() % 4 == 0 && !vIds.empty() ? vIds[Rand() % vIds.size()] : -1;
			G.m_Snap.m_SpecInfo.m_Active = Rand() % 4 == 0;
			G.m_Snap.m_SpecInfo.m_SpectatorId = Rand() % 2 || vIds.empty() ? SPEC_FREEVIEW : vIds[Rand() % vIds.size()];
			fprintf(f, "plates %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %08x %08x %08x %08x %d %d %d %d %d %d %d\n",
				g_Config.m_ClNamePlates, g_Config.m_ClNamePlatesOwn, g_Config.m_ClNamePlatesFriendMark, g_Config.m_ClNamePlatesIds,
				g_Config.m_ClNamePlatesSize, g_Config.m_ClNamePlatesIdsSeparateLine, g_Config.m_ClNamePlatesIdsSize, g_Config.m_ClNamePlatesClan,
				g_Config.m_ClNamePlatesClanSize, g_Config.m_ClNamePlatesStrong, g_Config.m_ClNamePlatesStrongSize, g_Config.m_ClDirectionSize,
				g_Config.m_ClNamePlatesAlways, g_Config.m_ClNamePlatesOffset, g_Config.m_ClShowOthersAlpha, g_Config.m_ClNamePlatesTeamcolors,
				g_Config.m_ClShowDirection, g_Config.m_Debug, Bits(ScreenX), Bits(ScreenY), Bits(G.m_Controls.m_aTargetPos[0].x),
				Bits(G.m_Controls.m_aTargetPos[0].y), G.m_Controls.m_aInputData[0].m_Direction, G.m_Controls.m_aInputData[0].m_Jump,
				G.m_Controls.m_aInputData[1].m_Direction, G.m_Controls.m_aInputData[1].m_Jump, G.m_aLocalIds[1],
				G.m_Snap.m_SpecInfo.m_Active, G.m_Snap.m_SpecInfo.m_SpectatorId);
			for(int Id : vIds)
			{
				CGameClient::CClientData &C = G.m_aClients[Id];
				CGameClient::CSnapState::CCharacterInfo &Char = G.m_Snap.m_aCharacters[Id];
				Char.m_Active = Rand() % 5 != 0;
				Char.m_Cur.m_Direction = (int)(Rand() % 3) - 1;
				Char.m_Cur.m_Jumped = Rand() % 4;
				Char.m_HasExtendedData = Rand() % 4 != 0;
				Char.m_ExtendedData.m_StrongWeakId = Rand() % 64;
				C.m_SpecCharPresent = Rand() % 8 == 0;
				C.m_SpecChar = vec2(ScreenX + (float)(Rand() % 1600) - 200.0f, ScreenY + (float)(Rand() % 1600) - 200.0f);
				C.m_RenderPos = vec2(ScreenX + (float)(Rand() % 2400) - 600.0f, ScreenY + (float)(Rand() % 1800) - 400.0f) + vec2(Rand() % 100 / 64.0f, Rand() % 100 / 64.0f);
				fprintf(f, "char %d %d %d %d %d %d %d %08x %08x %08x %08x\n", Id, Char.m_Active, Char.m_Cur.m_Direction, Char.m_Cur.m_Jumped,
					Char.m_HasExtendedData, Char.m_ExtendedData.m_StrongWeakId, C.m_SpecCharPresent, Bits(C.m_SpecChar.x), Bits(C.m_SpecChar.y),
					Bits(C.m_RenderPos.x), Bits(C.m_RenderPos.y));
			}
			fprintf(f, "platesout\n");
			Plates.OnRender();
			fprintf(f, "end\n");
		}

		// freeze bars
		CFreezeBars Bars;
		Bars.m_pGameClient = &G;
		for(int k = 0; k < 3; k++)
		{
			g_Config.m_ClShowFreezeBars = 1;
			g_Config.m_ClFreezeBarsAlphaInsideFreeze = Rand() % 3 == 0 ? 0 : (int)(Rand() % 101);
			g_Config.m_ClShowOthersAlpha = Rand() % 101;
			G.m_Client.m_GameTick = 1000 + Rand() % 500;
			G.m_Client.m_PredGameTick = G.m_Client.m_GameTick + Rand() % 20;
			G.m_Client.m_PredictionTick = G.m_Client.m_GameTick + Rand() % 20;
			const float ScreenX = (float)(Rand() % 3000), ScreenY = (float)(Rand() % 2000);
			Bars.m_Graphics.m_Screen = CScreenRect(ScreenX, ScreenY, 1200.0f, 700.0f);
			fprintf(f, "bars %d %d %d %d %d %08x %08x\n", g_Config.m_ClFreezeBarsAlphaInsideFreeze, g_Config.m_ClShowOthersAlpha,
				G.m_Client.m_GameTick, G.m_Client.m_PredGameTick, G.m_Client.m_PredictionTick, Bits(ScreenX), Bits(ScreenY));
			for(int Id : vIds)
			{
				CGameClient::CClientData &C = G.m_aClients[Id];
				CGameClient::CSnapState::CCharacterInfo &Char = G.m_Snap.m_aCharacters[Id];
				Char.m_Active = Rand() % 5 != 0;
				Char.m_HasExtendedDisplayInfo = Rand() % 6 != 0;
				Char.m_ExtendedData.m_FreezeEnd = Rand() % 5 == 0 ? 0 : 1;
				C.m_Predicted.m_FreezeStart = Rand() % 6 == 0 ? 0 : (int)(900 + Rand() % 600);
				C.m_Predicted.m_FreezeEnd = Rand() % 6 == 0 ? (int)(Rand() % 3) - 1 : C.m_Predicted.m_FreezeStart + (int)(Rand() % 300) - 20;
				C.m_Predicted.m_IsInFreeze = Rand() % 2;
				C.m_IsPredicted = Rand() % 2;
				C.m_IsPredictedLocal = Rand() % 4 == 0;
				C.m_RenderPos = vec2(ScreenX + (float)(Rand() % 1600) - 200.0f, ScreenY + (float)(Rand() % 1200) - 200.0f);
				fprintf(f, "frozen %d %d %d %d %d %d %d %d %d %d %08x %08x\n", Id, Char.m_Active, G.m_Snap.m_apPrevPlayerInfos[Id] != nullptr,
					Char.m_HasExtendedDisplayInfo, Char.m_ExtendedData.m_FreezeEnd, C.m_Predicted.m_FreezeStart, C.m_Predicted.m_FreezeEnd,
					C.m_Predicted.m_IsInFreeze, C.m_IsPredicted, C.m_IsPredictedLocal, Bits(C.m_RenderPos.x), Bits(C.m_RenderPos.y));
			}
			fprintf(f, "barsout");
			Bars.OnRender();
			fprintf(f, "\n");
		}
		for(int Id : vIds)
			G.m_Teams.Team(Id, TEAM_FLOCK);
	}
	return 0;
}

} // namespace rs_scoreboard

int ScoreboardMode(const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	const int Result = rs_scoreboard::Run(f);
	fclose(f);
	return Result;
}
