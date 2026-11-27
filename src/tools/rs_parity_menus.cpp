// "menus" mode of rs_parity: which buttons the in-game menu's Game page shows (PLAN
// §13.5), for rust-client/client/tests/esc_menu.rs.
//
// CMenus::RenderGame needs the whole client, so its state-dependent part (spectate, join
// red/blue/game, kill, pause, auto camera) is copied here word for word into a stand-in
// CMenus (in its own namespace) whose DoButton_Menu/DoButton_FontIcon record which
// buttons were drawn. The copy is marked "C++ copy: <file>:<lines>" ... "end of copy";
// the Rust test `cpp_copies_match_source` checks it is still identical (whitespace
// aside) to those source lines. Localize, g_Config and the team constants are the real
// ones. Every combination of the state the copy reads is run.

#include <base/str.h>

#include <engine/console.h>
#include <engine/graphics.h>
#include <engine/shared/config.h>
#include <engine/shared/protocol.h>

#include <game/localization.h>

#include <generated/protocol.h>

#include <cstdio>
#include <string>
#include <vector>

namespace menus_copy {

static std::vector<std::string> s_vButtons;

struct CButtonContainer
{
};

struct CUIRect
{
	void VSplitLeft(float, CUIRect *, CUIRect *) const {}
};

enum
{
	BUTTONFLAG_LEFT = 1,
};

namespace FontIcon {
static const char *CAMERA = "camera";
}

struct CUi
{
	int DoButton_FontIcon(CButtonContainer *, const char *, int, const CUIRect *, int, int, bool)
	{
		s_vButtons.emplace_back("<auto camera>");
		return 0;
	}
	void DoToolTip(const void *, const CUIRect *, const char *) {}
};

struct CConsole
{
	void ExecuteLine(const char *, int) {}
};

struct CClientStub
{
	bool m_DummyConnecting = false;
	bool DummyConnecting() const { return m_DummyConnecting; }
	bool DummyConnected() const { return false; }
};

struct CCameraStub
{
	bool m_AutoSpecCamera = false;
	bool SpectatingPlayer() const { return false; }
	bool CanUseAutoSpecCamera() const { return false; }
	void ToggleAutoSpecCamera() {}
	void UpdateAutoSpecCameraTooltip() {}
	const char *AutoSpecCameraTooltip() { return ""; }
};

struct CClientData
{
	bool m_Paused = false;
	bool m_Spec = false;
};

struct CGameClientStub
{
	struct
	{
		int m_LocalClientId = -1;
		const CNetObj_PlayerInfo *m_pLocalInfo = nullptr;
		const CNetObj_GameInfo *m_pGameInfoObj = nullptr;
	} m_Snap;
	CClientData m_aClients[MAX_CLIENTS];
	bool m_TeamPlay = false;
	bool m_ReceivedDDNetPlayer = false;
	CCameraStub m_Camera;
	bool IsTeamPlay() const { return m_TeamPlay; }
	void SendSwitchTeam(int) const {}
	void SendKill() const {}
};

class CMenus
{
public:
	CGameClientStub m_GameClient;
	CClientStub m_Client;
	CUi m_Ui;
	CConsole m_Console;
	CGameClientStub *GameClient() { return &m_GameClient; }
	CClientStub *Client() { return &m_Client; }
	CUi *Ui() { return &m_Ui; }
	CConsole *Console() { return &m_Console; }
	void SetActive(bool) {}
	int DoButton_Menu(CButtonContainer *, const char *pText, int, const CUIRect *)
	{
		s_vButtons.emplace_back(pText);
		return 0;
	}

	void RenderGameButtons(bool ShowDDRaceButtons)
	{
		CUIRect Button, ButtonBar;
// C++ copy: game/client/components/menus_ingame.cpp:140-251
	bool Paused = false;
	bool Spec = false;
	if(GameClient()->m_Snap.m_LocalClientId >= 0)
	{
		Paused = GameClient()->m_aClients[GameClient()->m_Snap.m_LocalClientId].m_Paused;
		Spec = GameClient()->m_aClients[GameClient()->m_Snap.m_LocalClientId].m_Spec;
	}

	if(GameClient()->m_Snap.m_pLocalInfo && GameClient()->m_Snap.m_pGameInfoObj && !Paused && !Spec)
	{
		if(GameClient()->m_Snap.m_pLocalInfo->m_Team != TEAM_SPECTATORS)
		{
			ButtonBar.VSplitLeft(120.0f, &Button, &ButtonBar);
			ButtonBar.VSplitLeft(5.0f, nullptr, &ButtonBar);
			static CButtonContainer s_SpectateButton;
			if(!Client()->DummyConnecting() && DoButton_Menu(&s_SpectateButton, Localize("Spectate"), 0, &Button))
			{
				if(g_Config.m_ClDummy == 0 || Client()->DummyConnected())
				{
					GameClient()->SendSwitchTeam(TEAM_SPECTATORS);
					SetActive(false);
				}
			}
		}

		if(GameClient()->IsTeamPlay())
		{
			if(GameClient()->m_Snap.m_pLocalInfo->m_Team != TEAM_RED)
			{
				ButtonBar.VSplitLeft(100.0f, &Button, &ButtonBar);
				ButtonBar.VSplitLeft(5.0f, nullptr, &ButtonBar);
				static CButtonContainer s_JoinRedButton;
				if(!Client()->DummyConnecting() && DoButton_Menu(&s_JoinRedButton, Localize("Join red"), 0, &Button))
				{
					GameClient()->SendSwitchTeam(TEAM_RED);
					SetActive(false);
				}
			}

			if(GameClient()->m_Snap.m_pLocalInfo->m_Team != TEAM_BLUE)
			{
				ButtonBar.VSplitLeft(100.0f, &Button, &ButtonBar);
				ButtonBar.VSplitLeft(5.0f, nullptr, &ButtonBar);
				static CButtonContainer s_JoinBlueButton;
				if(!Client()->DummyConnecting() && DoButton_Menu(&s_JoinBlueButton, Localize("Join blue"), 0, &Button))
				{
					GameClient()->SendSwitchTeam(TEAM_BLUE);
					SetActive(false);
				}
			}
		}
		else
		{
			if(GameClient()->m_Snap.m_pLocalInfo->m_Team != TEAM_GAME)
			{
				ButtonBar.VSplitLeft(120.0f, &Button, &ButtonBar);
				ButtonBar.VSplitLeft(5.0f, nullptr, &ButtonBar);
				static CButtonContainer s_JoinGameButton;
				if(!Client()->DummyConnecting() && DoButton_Menu(&s_JoinGameButton, Localize("Join game"), 0, &Button))
				{
					GameClient()->SendSwitchTeam(TEAM_GAME);
					SetActive(false);
				}
			}
		}

		if(GameClient()->m_Snap.m_pLocalInfo->m_Team != TEAM_SPECTATORS && (ShowDDRaceButtons || !GameClient()->IsTeamPlay()))
		{
			ButtonBar.VSplitLeft(65.0f, &Button, &ButtonBar);
			ButtonBar.VSplitLeft(5.0f, nullptr, &ButtonBar);

			static CButtonContainer s_KillButton;
			if(DoButton_Menu(&s_KillButton, Localize("Kill"), 0, &Button))
			{
				GameClient()->SendKill();
				SetActive(false);
			}
		}
	}

	if(GameClient()->m_ReceivedDDNetPlayer && GameClient()->m_Snap.m_pLocalInfo && (ShowDDRaceButtons || !GameClient()->IsTeamPlay()))
	{
		if(GameClient()->m_Snap.m_pLocalInfo->m_Team != TEAM_SPECTATORS || Paused || Spec)
		{
			ButtonBar.VSplitLeft((!Paused && !Spec) ? 65.0f : 120.0f, &Button, &ButtonBar);
			ButtonBar.VSplitLeft(5.0f, nullptr, &ButtonBar);

			static CButtonContainer s_PauseButton;
			if(DoButton_Menu(&s_PauseButton, (!Paused && !Spec) ? Localize("Pause") : Localize("Join game"), 0, &Button))
			{
				Console()->ExecuteLine("say /pause", IConsole::CLIENT_ID_UNSPECIFIED);
				SetActive(false);
			}
		}
	}

	if(GameClient()->m_Snap.m_pLocalInfo && (GameClient()->m_Snap.m_pLocalInfo->m_Team == TEAM_SPECTATORS || Paused || Spec))
	{
		ButtonBar.VSplitLeft(32.0f, &Button, &ButtonBar);
		ButtonBar.VSplitLeft(5.0f, nullptr, &ButtonBar);

		static CButtonContainer s_AutoCameraButton;

		bool Active = GameClient()->m_Camera.m_AutoSpecCamera && GameClient()->m_Camera.SpectatingPlayer() && GameClient()->m_Camera.CanUseAutoSpecCamera();
		bool Enabled = g_Config.m_ClSpecAutoSync;
		if(Ui()->DoButton_FontIcon(&s_AutoCameraButton, FontIcon::CAMERA, !Active, &Button, BUTTONFLAG_LEFT, IGraphics::CORNER_ALL, Enabled))
		{
			GameClient()->m_Camera.ToggleAutoSpecCamera();
		}
		GameClient()->m_Camera.UpdateAutoSpecCameraTooltip();
		Ui()->DoToolTip(&s_AutoCameraButton, &Button, GameClient()->m_Camera.AutoSpecCameraTooltip());
	}
// end of copy
	}
};

} // namespace menus_copy

int MenusMode(const char *pOutFile)
{
	using namespace menus_copy;
	FILE *pFile = fopen(pOutFile, "wb");
	if(!pFile)
		return -1;
	// local id, local info, game info, local team (spectators, red/game, blue), paused,
	// spec, team play, received DDNet player, room for the DDRace buttons, dummy connecting
	const int aTeams[] = {TEAM_SPECTATORS, TEAM_RED, TEAM_BLUE};
	int Cases = 0;
	for(int Bits = 0; Bits < (1 << 9); Bits++)
	{
		for(int Team : aTeams)
		{
			CMenus Menus;
			CNetObj_PlayerInfo LocalInfo = {};
			CNetObj_GameInfo GameInfo = {};
			LocalInfo.m_Team = Team;
			const bool HasLocalId = Bits & 1;
			auto &Snap = Menus.m_GameClient.m_Snap;
			Snap.m_LocalClientId = HasLocalId ? 3 : -1;
			Snap.m_pLocalInfo = (Bits & 2) ? &LocalInfo : nullptr;
			Snap.m_pGameInfoObj = (Bits & 4) ? &GameInfo : nullptr;
			Menus.m_GameClient.m_aClients[3].m_Paused = Bits & 8;
			Menus.m_GameClient.m_aClients[3].m_Spec = Bits & 16;
			Menus.m_GameClient.m_TeamPlay = Bits & 32;
			Menus.m_GameClient.m_ReceivedDDNetPlayer = Bits & 64;
			const bool ShowDDRaceButtons = Bits & 128;
			Menus.m_Client.m_DummyConnecting = Bits & 256;
			s_vButtons.clear();
			Menus.RenderGameButtons(ShowDDRaceButtons);
			fprintf(pFile, "%d %d %d %d %d %d %d %d %d %d", HasLocalId, Snap.m_pLocalInfo != nullptr,
				Snap.m_pGameInfoObj != nullptr, Team, (Bits & 8) != 0, (Bits & 16) != 0, (Bits & 32) != 0,
				(Bits & 64) != 0, ShowDDRaceButtons, (Bits & 256) != 0);
			for(const std::string &Label : s_vButtons)
				fprintf(pFile, "|%s", Label.c_str());
			fprintf(pFile, "\n");
			Cases++;
		}
	}
	fclose(pFile);
	printf("%d cases\n", Cases);
	return 0;
}
