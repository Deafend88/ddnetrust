// "dummy" mode of rs_parity: the dummy's inputs (PLAN §13.6), for
// rust-client/client/tests/input.rs.
//
// CControls::SnapInput/ResetInput, CGameClient::OnDummySwap/OnSnapInput and
// CClient::SendInput need the whole client, so they are copied here word for word
// into stand-in classes (in their own namespace); the copies are marked "C++ copy:
// <file>:<lines>" ... "end of copy" and `cpp_copies_match_source` checks them against
// the source. g_Config, CMsgPacker, time_get and the protocol constants are the real
// ones. Random events (keys, mouse, dummy settings, swaps) run between SendInput calls;
// every event and, per SendInput, what SnapInput and OnSnapInput read and produced are
// written out. ServerCapAnyPlayerFlag is false (DDNet-rs sends no AIM/SPEC_CAM flags
// yet) and there is no local character (the ninja send rule is not exercised).

#include <base/math.h>
#include <base/mem.h>
#include <base/time.h>
#include <base/vmath.h>

#include <engine/client/enums.h>
#include <engine/message.h>
#include <engine/shared/config.h>
#include <engine/shared/protocol.h>
#include <engine/shared/protocolglue.h>

#include <generated/protocol.h>

#include <cstddef>
#include <cstdio>

namespace dummy_copy {

static FILE *s_pOut = nullptr;

struct CChat
{
	bool m_Active = false;
	bool IsActive() const { return m_Active; }
};
struct CMenus
{
	bool m_Active = false;
	bool IsActive() const { return m_Active; }
};
struct CScoreboard
{
	bool m_Active = false;
	bool IsActive() const { return m_Active; }
};
struct CCamera
{
	enum
	{
		CAMTYPE_UNDEFINED = -1,
		CAMTYPE_SPEC,
		CAMTYPE_PLAYER,
	};
	int CamType() const { return CAMTYPE_PLAYER; }
};
struct CGameInfo
{
	bool m_BugDDRaceInput = false;
};
struct CSnapState
{
	struct
	{
		bool m_Active = false;
		int m_SpectatorId = -1;
	} m_SpecInfo;
	const CNetObj_Character *m_pLocalCharacter = nullptr;
};
struct CStubClient
{
	bool ServerCapAnyPlayerFlag() const { return false; }
	float LocalTime() const { return 0.0f; }
};
struct CClientData
{
	struct
	{
		vec2 m_Pos;
	} m_Predicted;
};

class CGameClient;

class CControls
{
public:
	enum class EMouseInputType
	{
		ABSOLUTE,
		RELATIVE,
		AUTOMATED,
	};
	vec2 m_aMousePos[NUM_DUMMIES] = {};
	vec2 m_aMousePosOnAction[NUM_DUMMIES] = {};
	EMouseInputType m_aMouseInputType[NUM_DUMMIES] = {EMouseInputType::ABSOLUTE, EMouseInputType::ABSOLUTE};
	int64_t m_LastSendTime = 0;
	CNetObj_PlayerInput m_aInputData[NUM_DUMMIES] = {};
	CNetObj_PlayerInput m_aLastData[NUM_DUMMIES] = {};
	int m_aInputDirectionLeft[NUM_DUMMIES] = {};
	int m_aInputDirectionRight[NUM_DUMMIES] = {};
	int m_aShowHookColl[NUM_DUMMIES] = {};
	CGameClient *m_pGameClient = nullptr;
	CGameClient *GameClient() const { return m_pGameClient; }
	CStubClient *Client() const;
	int SnapInput(int *pData);
	void ResetInput(int Dummy);
};

// records what SnapInput produced, then the state of both tees and m_DummyInput
struct CControlsRec : CControls
{
	int SnapInput(int *pData);
};

class CGameClient
{
public:
	CStubClient m_StubClient;
	CControlsRec m_Controls;
	CChat m_Chat;
	CMenus m_Menus;
	CScoreboard m_Scoreboard;
	CCamera m_Camera;
	CGameInfo m_GameInfo;
	CSnapState m_Snap;
	CNetObj_PlayerInput m_DummyInput = {};
	CNetObj_PlayerInput m_HammerInput = {};
	int m_DummyFire = 0;
	int m_IsDummySwapping = 0;
	int m_aLocalIds[NUM_DUMMIES] = {-1, -1};
	vec2 m_LocalCharacterPos = vec2(0, 0);
	CClientData m_aClients[MAX_CLIENTS] = {};
	CStubClient *Client() { return &m_StubClient; }
	void OnDummySwap();
	int OnSnapInput(int *pData, bool Dummy, bool Force);
};

CStubClient *CControls::Client() const { return &m_pGameClient->m_StubClient; }

// records OnSnapInput for the other tee: what it read and what it produced
struct CGameRec : CGameClient
{
	int OnSnapInput(int *pData, bool Dummy, bool Force);
};

struct CSmoothTimeStub
{
	int64_t Get(int64_t Now) const { return Now; }
};

class CClient
{
public:
	int m_aPredTick[NUM_DUMMIES] = {100, 100};
	int m_aAckGameTick[NUM_DUMMIES] = {90, 91};
	int m_aCurrentInput[NUM_DUMMIES] = {0, 0};
	CSmoothTimeStub m_PredictedTime;
	bool m_DummyConnected = false;
	struct
	{
		int m_aData[MAX_INPUT_SIZE];
		int m_Tick;
		int64_t m_PredictedTime;
		int64_t m_PredictionMargin;
		int64_t m_Time;
	} m_aInputs[NUM_DUMMIES][200];
	CGameRec *m_pGameClient = nullptr;
	CGameRec *GameClient() { return m_pGameClient; }
	bool DummyConnected() const { return m_DummyConnected; }
	bool IsSixup() const { return false; }
	int PredictionMargin() const { return 10; }
	int SendMsg(int Conn, CMsgPacker *pMsg, int Flags);
	void SendInput();
};

static unsigned Bits(float f)
{
	unsigned u;
	mem_copy(&u, &f, sizeof(u));
	return u;
}

static void PrintInput(const CNetObj_PlayerInput &In)
{
	const int *p = reinterpret_cast<const int *>(&In);
	for(int i = 0; i < (int)(sizeof(In) / sizeof(int)); i++)
		fprintf(s_pOut, "%s%d", i ? "," : " ", p[i]);
}

int CControlsRec::SnapInput(int *pData)
{
	mem_zero(pData, sizeof(CNetObj_PlayerInput));
	const int Size = CControls::SnapInput(pData);
	fprintf(s_pOut, "snap %d", Size);
	PrintInput(*reinterpret_cast<CNetObj_PlayerInput *>(pData));
	for(int d = 0; d < NUM_DUMMIES; d++)
	{
		PrintInput(m_aInputData[d]);
		PrintInput(m_aLastData[d]);
		fprintf(s_pOut, " %08x %08x", Bits(m_aMousePos[d].x), Bits(m_aMousePos[d].y));
	}
	PrintInput(GameClient()->m_DummyInput);
	fprintf(s_pOut, "\n");
	return Size;
}

int CGameRec::OnSnapInput(int *pData, bool Dummy, bool Force)
{
	if(!Dummy)
		return CGameClient::OnSnapInput(pData, Dummy, Force);
	fprintf(s_pOut, "other %d %d", Force, m_aLocalIds[!g_Config.m_ClDummy] >= 0);
	PrintInput(m_DummyInput);
	PrintInput(m_HammerInput);
	fprintf(s_pOut, " %d |", m_DummyFire);
	mem_zero(pData, sizeof(CNetObj_PlayerInput));
	const int Size = CGameClient::OnSnapInput(pData, Dummy, Force);
	fprintf(s_pOut, " %d", Size);
	PrintInput(*reinterpret_cast<CNetObj_PlayerInput *>(pData));
	PrintInput(m_DummyInput);
	PrintInput(m_HammerInput);
	fprintf(s_pOut, " %d\n", m_DummyFire);
	return Size;
}

int CClient::SendMsg(int Conn, CMsgPacker *pMsg, int Flags)
{
	fprintf(s_pOut, "sent %d %d\n", Conn, m_aCurrentInput[Conn]);
	return 0;
}

// C++ copy: game/client/components/controls.cpp:45-57
void CControls::ResetInput(int Dummy)
{
	m_aLastData[Dummy].m_Direction = 0;
	// simulate releasing the fire button
	if((m_aLastData[Dummy].m_Fire & 1) != 0)
		m_aLastData[Dummy].m_Fire++;
	m_aLastData[Dummy].m_Fire &= INPUT_STATE_MASK;
	m_aLastData[Dummy].m_Jump = 0;
	m_aInputData[Dummy] = m_aLastData[Dummy];

	m_aInputDirectionLeft[Dummy] = 0;
	m_aInputDirectionRight[Dummy] = 0;
}
// end of copy

// C++ copy: game/client/components/controls.cpp:241-394
int CControls::SnapInput(int *pData)
{
	// update player state
	if(GameClient()->m_Chat.IsActive())
		m_aInputData[g_Config.m_ClDummy].m_PlayerFlags = PLAYERFLAG_CHATTING;
	else if(GameClient()->m_Menus.IsActive())
		m_aInputData[g_Config.m_ClDummy].m_PlayerFlags = PLAYERFLAG_IN_MENU;
	else
		m_aInputData[g_Config.m_ClDummy].m_PlayerFlags = PLAYERFLAG_PLAYING;

	if(GameClient()->m_Scoreboard.IsActive())
		m_aInputData[g_Config.m_ClDummy].m_PlayerFlags |= PLAYERFLAG_SCOREBOARD;

	if(Client()->ServerCapAnyPlayerFlag() && GameClient()->m_Controls.m_aShowHookColl[g_Config.m_ClDummy])
		m_aInputData[g_Config.m_ClDummy].m_PlayerFlags |= PLAYERFLAG_AIM;

	if(Client()->ServerCapAnyPlayerFlag() && GameClient()->m_Camera.CamType() == CCamera::CAMTYPE_SPEC)
		m_aInputData[g_Config.m_ClDummy].m_PlayerFlags |= PLAYERFLAG_SPEC_CAM;

	switch(m_aMouseInputType[g_Config.m_ClDummy])
	{
	case CControls::EMouseInputType::AUTOMATED:
		m_aInputData[g_Config.m_ClDummy].m_PlayerFlags |= PLAYERFLAG_INPUT_ABSOLUTE;
		break;
	case CControls::EMouseInputType::ABSOLUTE:
		m_aInputData[g_Config.m_ClDummy].m_PlayerFlags |= PLAYERFLAG_INPUT_ABSOLUTE | PLAYERFLAG_INPUT_MANUAL;
		break;
	case CControls::EMouseInputType::RELATIVE:
		m_aInputData[g_Config.m_ClDummy].m_PlayerFlags |= PLAYERFLAG_INPUT_MANUAL;
		break;
	}

	bool Send = m_aLastData[g_Config.m_ClDummy].m_PlayerFlags != m_aInputData[g_Config.m_ClDummy].m_PlayerFlags;

	m_aLastData[g_Config.m_ClDummy].m_PlayerFlags = m_aInputData[g_Config.m_ClDummy].m_PlayerFlags;

	// we freeze the input if chat or menu is activated
	if(!(m_aInputData[g_Config.m_ClDummy].m_PlayerFlags & PLAYERFLAG_PLAYING))
	{
		if(!GameClient()->m_GameInfo.m_BugDDRaceInput)
			ResetInput(g_Config.m_ClDummy);

		mem_copy(pData, &m_aInputData[g_Config.m_ClDummy], sizeof(m_aInputData[0]));

		// set the target anyway though so that we can keep seeing our surroundings,
		// even if chat or menu are activated
		m_aInputData[g_Config.m_ClDummy].m_TargetX = (int)m_aMousePos[g_Config.m_ClDummy].x;
		m_aInputData[g_Config.m_ClDummy].m_TargetY = (int)m_aMousePos[g_Config.m_ClDummy].y;

		// send once a second just to be sure
		Send = Send || time_get() > m_LastSendTime + time_freq();
	}
	else
	{
		m_aInputData[g_Config.m_ClDummy].m_TargetX = (int)m_aMousePos[g_Config.m_ClDummy].x;
		m_aInputData[g_Config.m_ClDummy].m_TargetY = (int)m_aMousePos[g_Config.m_ClDummy].y;

		if(g_Config.m_ClSubTickAiming && m_aMousePosOnAction[g_Config.m_ClDummy] != vec2(0.0f, 0.0f))
		{
			m_aInputData[g_Config.m_ClDummy].m_TargetX = (int)m_aMousePosOnAction[g_Config.m_ClDummy].x;
			m_aInputData[g_Config.m_ClDummy].m_TargetY = (int)m_aMousePosOnAction[g_Config.m_ClDummy].y;
			m_aMousePosOnAction[g_Config.m_ClDummy] = vec2(0.0f, 0.0f);
		}

		if(!m_aInputData[g_Config.m_ClDummy].m_TargetX && !m_aInputData[g_Config.m_ClDummy].m_TargetY)
		{
			m_aInputData[g_Config.m_ClDummy].m_TargetX = 1;
			m_aMousePos[g_Config.m_ClDummy].x = 1;
		}

		// set direction
		m_aInputData[g_Config.m_ClDummy].m_Direction = 0;
		if(m_aInputDirectionLeft[g_Config.m_ClDummy] && !m_aInputDirectionRight[g_Config.m_ClDummy])
			m_aInputData[g_Config.m_ClDummy].m_Direction = -1;
		if(!m_aInputDirectionLeft[g_Config.m_ClDummy] && m_aInputDirectionRight[g_Config.m_ClDummy])
			m_aInputData[g_Config.m_ClDummy].m_Direction = 1;

		// dummy copy moves
		if(g_Config.m_ClDummyCopyMoves)
		{
			CNetObj_PlayerInput *pDummyInput = &GameClient()->m_DummyInput;

			// Don't copy any input to dummy when spectating others
			if(!GameClient()->m_Snap.m_SpecInfo.m_Active || GameClient()->m_Snap.m_SpecInfo.m_SpectatorId < 0)
			{
				pDummyInput->m_Direction = m_aInputData[g_Config.m_ClDummy].m_Direction;
				pDummyInput->m_Hook = m_aInputData[g_Config.m_ClDummy].m_Hook;
				pDummyInput->m_Jump = m_aInputData[g_Config.m_ClDummy].m_Jump;
				pDummyInput->m_PlayerFlags = m_aInputData[g_Config.m_ClDummy].m_PlayerFlags;
				pDummyInput->m_TargetX = m_aInputData[g_Config.m_ClDummy].m_TargetX;
				pDummyInput->m_TargetY = m_aInputData[g_Config.m_ClDummy].m_TargetY;
				pDummyInput->m_WantedWeapon = m_aInputData[g_Config.m_ClDummy].m_WantedWeapon;

				if(!g_Config.m_ClDummyControl)
					pDummyInput->m_Fire += m_aInputData[g_Config.m_ClDummy].m_Fire - m_aLastData[g_Config.m_ClDummy].m_Fire;

				pDummyInput->m_NextWeapon += m_aInputData[g_Config.m_ClDummy].m_NextWeapon - m_aLastData[g_Config.m_ClDummy].m_NextWeapon;
				pDummyInput->m_PrevWeapon += m_aInputData[g_Config.m_ClDummy].m_PrevWeapon - m_aLastData[g_Config.m_ClDummy].m_PrevWeapon;
			}

			m_aInputData[!g_Config.m_ClDummy] = *pDummyInput;
		}

		if(g_Config.m_ClDummyControl)
		{
			CNetObj_PlayerInput *pDummyInput = &GameClient()->m_DummyInput;
			pDummyInput->m_Jump = g_Config.m_ClDummyJump;

			if(g_Config.m_ClDummyFire)
				pDummyInput->m_Fire = g_Config.m_ClDummyFire;
			else if((pDummyInput->m_Fire & 1) != 0)
				pDummyInput->m_Fire++;

			pDummyInput->m_Hook = g_Config.m_ClDummyHook;
			m_aInputData[!g_Config.m_ClDummy] = *pDummyInput;
		}

		// stress testing
		if(g_Config.m_DbgStress)
		{
			float t = Client()->LocalTime();
			mem_zero(&m_aInputData[g_Config.m_ClDummy], sizeof(m_aInputData[0]));

			m_aInputData[g_Config.m_ClDummy].m_Direction = ((int)t / 2) & 1;
			m_aInputData[g_Config.m_ClDummy].m_Jump = ((int)t);
			m_aInputData[g_Config.m_ClDummy].m_Fire = ((int)(t * 10));
			m_aInputData[g_Config.m_ClDummy].m_Hook = ((int)(t * 2)) & 1;
			m_aInputData[g_Config.m_ClDummy].m_WantedWeapon = ((int)t) % NUM_WEAPONS;
			m_aInputData[g_Config.m_ClDummy].m_TargetX = (int)(std::sin(t * 3) * 100.0f);
			m_aInputData[g_Config.m_ClDummy].m_TargetY = (int)(std::cos(t * 3) * 100.0f);
		}

		// check if we need to send input
		Send = Send || m_aInputData[g_Config.m_ClDummy].m_Direction != m_aLastData[g_Config.m_ClDummy].m_Direction;
		Send = Send || m_aInputData[g_Config.m_ClDummy].m_Jump != m_aLastData[g_Config.m_ClDummy].m_Jump;
		Send = Send || m_aInputData[g_Config.m_ClDummy].m_Fire != m_aLastData[g_Config.m_ClDummy].m_Fire;
		Send = Send || m_aInputData[g_Config.m_ClDummy].m_Hook != m_aLastData[g_Config.m_ClDummy].m_Hook;
		Send = Send || m_aInputData[g_Config.m_ClDummy].m_WantedWeapon != m_aLastData[g_Config.m_ClDummy].m_WantedWeapon;
		Send = Send || m_aInputData[g_Config.m_ClDummy].m_NextWeapon != m_aLastData[g_Config.m_ClDummy].m_NextWeapon;
		Send = Send || m_aInputData[g_Config.m_ClDummy].m_PrevWeapon != m_aLastData[g_Config.m_ClDummy].m_PrevWeapon;
		Send = Send || time_get() > m_LastSendTime + time_freq() / 25; // send at least 25 Hz
		Send = Send || (GameClient()->m_Snap.m_pLocalCharacter && GameClient()->m_Snap.m_pLocalCharacter->m_Weapon == WEAPON_NINJA && (m_aInputData[g_Config.m_ClDummy].m_Direction || m_aInputData[g_Config.m_ClDummy].m_Jump || m_aInputData[g_Config.m_ClDummy].m_Hook));
	}

	// copy and return size
	m_aLastData[g_Config.m_ClDummy] = m_aInputData[g_Config.m_ClDummy];

	if(!Send)
		return 0;

	m_LastSendTime = time_get();
	mem_copy(pData, &m_aInputData[g_Config.m_ClDummy], sizeof(m_aInputData[0]));
	return sizeof(m_aInputData[0]);
}
// end of copy

// C++ copy: game/client/gameclient.cpp:513-525
void CGameClient::OnDummySwap()
{
	if(g_Config.m_ClDummyResetOnSwitch)
	{
		int PlayerOrDummy = (g_Config.m_ClDummyResetOnSwitch == 2) ? g_Config.m_ClDummy : (!g_Config.m_ClDummy);
		m_Controls.ResetInput(PlayerOrDummy);
		m_Controls.m_aInputData[PlayerOrDummy].m_Hook = 0;
	}
	const int PrevDummyFire = m_DummyInput.m_Fire;
	m_DummyInput = m_Controls.m_aInputData[!g_Config.m_ClDummy];
	m_Controls.m_aInputData[g_Config.m_ClDummy].m_Fire = PrevDummyFire;
	m_IsDummySwapping = 1;
}
// end of copy

// C++ copy: game/client/gameclient.cpp:527-577
int CGameClient::OnSnapInput(int *pData, bool Dummy, bool Force)
{
	if(!Dummy)
	{
		return m_Controls.SnapInput(pData);
	}
	if(m_aLocalIds[!g_Config.m_ClDummy] < 0)
	{
		return 0;
	}

	if(!g_Config.m_ClDummyHammer)
	{
		if(m_DummyFire != 0)
		{
			m_DummyInput.m_Fire = (m_HammerInput.m_Fire + 1) & ~1;
			m_DummyFire = 0;
		}

		if(!Force && (!m_DummyInput.m_Direction && !m_DummyInput.m_Jump && !m_DummyInput.m_Hook))
		{
			return 0;
		}

		mem_copy(pData, &m_DummyInput, sizeof(m_DummyInput));
		return sizeof(m_DummyInput);
	}
	else
	{
		if(m_DummyFire % 25 != 0)
		{
			m_DummyFire++;
			return 0;
		}
		m_DummyFire++;

		m_HammerInput.m_Fire = (m_HammerInput.m_Fire + 1) | 1;
		m_HammerInput.m_WantedWeapon = WEAPON_HAMMER + 1;
		if(!g_Config.m_ClDummyRestoreWeapon)
		{
			m_DummyInput.m_WantedWeapon = WEAPON_HAMMER + 1;
		}

		const vec2 Dir = m_LocalCharacterPos - m_aClients[m_aLocalIds[!g_Config.m_ClDummy]].m_Predicted.m_Pos;
		m_HammerInput.m_TargetX = (int)Dir.x;
		m_HammerInput.m_TargetY = (int)Dir.y;

		mem_copy(pData, &m_HammerInput, sizeof(m_HammerInput));
		return sizeof(m_HammerInput);
	}
}
// end of copy

// C++ copy: engine/client/client.cpp:350-407
void CClient::SendInput()
{
	int64_t Now = time_get();

	if(m_aPredTick[g_Config.m_ClDummy] <= 0)
		return;

	bool Force = false;
	// fetch input
	for(int Dummy = 0; Dummy < NUM_DUMMIES; Dummy++)
	{
		if(!DummyConnected() && Dummy != 0)
		{
			break;
		}
		int i = g_Config.m_ClDummy ^ Dummy;
		int Size = GameClient()->OnSnapInput(m_aInputs[i][m_aCurrentInput[i]].m_aData, Dummy, Force);

		if(Size)
		{
			// pack input
			CMsgPacker Msg(NETMSG_INPUT, true);
			Msg.AddInt(m_aAckGameTick[i]);
			Msg.AddInt(m_aPredTick[g_Config.m_ClDummy]);
			Msg.AddInt(Size);

			m_aInputs[i][m_aCurrentInput[i]].m_Tick = m_aPredTick[g_Config.m_ClDummy];
			m_aInputs[i][m_aCurrentInput[i]].m_PredictedTime = m_PredictedTime.Get(Now);
			m_aInputs[i][m_aCurrentInput[i]].m_PredictionMargin = PredictionMargin() * time_freq() / 1000;
			m_aInputs[i][m_aCurrentInput[i]].m_Time = Now;

			// pack it
			for(int k = 0; k < Size / 4; k++)
			{
				static const int FlagsOffset = offsetof(CNetObj_PlayerInput, m_PlayerFlags) / sizeof(int);
				if(k == FlagsOffset && IsSixup())
				{
					int PlayerFlags = m_aInputs[i][m_aCurrentInput[i]].m_aData[k];
					Msg.AddInt(PlayerFlags_SixToSeven(PlayerFlags));
				}
				else
				{
					Msg.AddInt(m_aInputs[i][m_aCurrentInput[i]].m_aData[k]);
				}
			}

			m_aCurrentInput[i]++;
			m_aCurrentInput[i] %= 200;

			SendMsg(i, &Msg, MSGFLAG_FLUSH);
			// ugly workaround for dummy. we need to send input with dummy to prevent
			// prediction time resets. but if we do it too often, then it's
			// impossible to use grenade with frozen dummy that gets hammered...
			if(g_Config.m_ClDummyCopyMoves || m_aCurrentInput[i] % 2)
				Force = true;
		}
	}
}
// end of copy

static unsigned s_Seed = 4242;
static unsigned Rand()
{
	s_Seed = s_Seed * 1103515245u + 12345u;
	return s_Seed >> 8;
}

} // namespace dummy_copy

int DummyMode(const char *pOutFile)
{
	using namespace dummy_copy;
	s_pOut = fopen(pOutFile, "wb");
	if(!s_pOut)
		return -1;
	static CGameRec s_Game;
	static CClient s_Client;
	s_Game.m_Controls.m_pGameClient = &s_Game;
	s_Client.m_pGameClient = &s_Game;
	g_Config.m_ClDummy = 0;
	g_Config.m_DbgStress = 0;
	g_Config.m_ClSubTickAiming = 0;
	bool LastDummy = false;
	for(int Step = 0; Step < 20000; Step++)
	{
		const int Kind = Rand() % 100;
		CControls &c = s_Game.m_Controls;
		const int d = g_Config.m_ClDummy;
		if(Kind < 3)
		{
			// the dummy settings and what the game state looks like
			g_Config.m_ClDummyCopyMoves = Rand() % 2;
			g_Config.m_ClDummyControl = Rand() % 3 == 0;
			g_Config.m_ClDummyJump = Rand() % 2;
			g_Config.m_ClDummyFire = Rand() % 2;
			g_Config.m_ClDummyHook = Rand() % 2;
			g_Config.m_ClDummyHammer = Rand() % 4 == 0;
			g_Config.m_ClDummyRestoreWeapon = Rand() % 2;
			g_Config.m_ClDummyResetOnSwitch = Rand() % 3;
			s_Client.m_DummyConnected = Rand() % 6 != 0;
			s_Game.m_aLocalIds[0] = Rand() % 8 == 0 ? -1 : 3;
			s_Game.m_aLocalIds[1] = Rand() % 8 == 0 ? -1 : 5;
			s_Game.m_Snap.m_SpecInfo.m_Active = Rand() % 6 == 0;
			s_Game.m_Snap.m_SpecInfo.m_SpectatorId = Rand() % 2 ? -1 : 7;
			s_Game.m_Chat.m_Active = Rand() % 8 == 0;
			s_Game.m_Menus.m_Active = Rand() % 8 == 0;
			s_Game.m_Scoreboard.m_Active = Rand() % 6 == 0;
			s_Game.m_GameInfo.m_BugDDRaceInput = Rand() % 4 == 0;
			s_Game.m_LocalCharacterPos = vec2((int)(Rand() % 20001) - 10000, (int)(Rand() % 20001) - 10000) / 7.0f;
			for(int Id : {3, 5})
				s_Game.m_aClients[Id].m_Predicted.m_Pos = vec2((int)(Rand() % 20001) - 10000, (int)(Rand() % 20001) - 10000) / 7.0f;
			fprintf(s_pOut, "cfg %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %08x %08x %08x %08x %08x %08x\n",
				g_Config.m_ClDummyCopyMoves, g_Config.m_ClDummyControl, g_Config.m_ClDummyJump, g_Config.m_ClDummyFire,
				g_Config.m_ClDummyHook, g_Config.m_ClDummyHammer, g_Config.m_ClDummyRestoreWeapon, g_Config.m_ClDummyResetOnSwitch,
				s_Client.m_DummyConnected, s_Game.m_aLocalIds[0], s_Game.m_aLocalIds[1], s_Game.m_Snap.m_SpecInfo.m_Active,
				s_Game.m_Snap.m_SpecInfo.m_SpectatorId, s_Game.m_Chat.m_Active, s_Game.m_Menus.m_Active, s_Game.m_Scoreboard.m_Active,
				s_Game.m_GameInfo.m_BugDDRaceInput, Bits(s_Game.m_LocalCharacterPos.x), Bits(s_Game.m_LocalCharacterPos.y),
				Bits(s_Game.m_aClients[3].m_Predicted.m_Pos.x), Bits(s_Game.m_aClients[3].m_Predicted.m_Pos.y),
				Bits(s_Game.m_aClients[5].m_Predicted.m_Pos.x), Bits(s_Game.m_aClients[5].m_Predicted.m_Pos.y));
		}
		else if(Kind < 8)
		{
			// cl_dummy toggled; CClient::Update calls OnDummySwap when it changed
			if(s_Client.m_DummyConnected)
				g_Config.m_ClDummy ^= 1;
			fprintf(s_pOut, "dummy %d\n", g_Config.m_ClDummy);
			if(LastDummy != (bool)g_Config.m_ClDummy)
				s_Game.OnDummySwap();
			LastDummy = g_Config.m_ClDummy;
		}
		else if(Kind < 50)
		{
			// a bind's command for the active tee, as ConKeyInputState/Counter/Set and
			// ConKeyInputNextPrevWeapon do
			const int Key = Rand() % 9;
			const int Stroke = Rand() % 2;
			int Value = Stroke;
			CNetObj_PlayerInput &In = c.m_aInputData[d];
			auto Counter = [&](int *v) {
				if(((*v) & 1) != Stroke)
					(*v)++;
				*v &= INPUT_STATE_MASK;
			};
			switch(Key)
			{
			case 0: c.m_aInputDirectionLeft[d] = Stroke; break;
			case 1: c.m_aInputDirectionRight[d] = Stroke; break;
			case 2: In.m_Jump = Stroke; break;
			case 3: In.m_Hook = Stroke; break;
			case 4: Counter(&In.m_Fire); break;
			case 5:
				Counter(&In.m_NextWeapon);
				In.m_WantedWeapon = 0;
				break;
			case 6:
				Counter(&In.m_PrevWeapon);
				In.m_WantedWeapon = 0;
				break;
			default:
				Value = 1 + Rand() % 5;
				In.m_WantedWeapon = Value;
			}
			fprintf(s_pOut, "key %d %d %d\n", d, Key, Value);
		}
		else if(Kind < 60)
		{
			const int Type = Rand() % 3;
			const vec2 Pos = Rand() % 5 == 0 ? vec2(0, 0) : vec2((int)(Rand() % 8001) - 4000, (int)(Rand() % 8001) - 4000) / 3.0f;
			c.m_aMousePos[d] = Pos;
			c.m_aMouseInputType[d] = (CControls::EMouseInputType)Type;
			fprintf(s_pOut, "mouse %d %08x %08x %d\n", d, Bits(Pos.x), Bits(Pos.y), Type);
		}
		else
		{
			// SendInput, with the 25 Hz/1 s resend due or not
			const bool Due = Rand() % 3 == 0;
			c.m_LastSendTime = Due ? 0 : time_get();
			fprintf(s_pOut, "send %d\n", Due);
			s_Client.SendInput();
		}
	}
	fclose(s_pOut);
	return 0;
}
