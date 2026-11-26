// "chat" mode of rs_parity: the chat logic (PLAN §13.1), for
// rust-client/client/tests/chat.rs.
//
// CChat (game/client/components/chat.cpp) needs the whole CGameClient, so its logic is
// copied here word for word into a stand-in class of the same name (in its own
// namespace): AddLine, LineShouldHighlight, OnMessage, OnInput, EnableMode,
// DisableMode, RegisterCommand, UnregisterCommand, SendChat, SendChatQueued and the
// queued send of OnRender. Each copy is marked "C++ copy: <file>:<lines>" ...
// "end of copy"; the Rust test `cpp_copies_match_source` checks that every copy is
// still identical (whitespace aside) to those source lines. What the copies call is the
// real code: the str_* functions, CStaticRingBuffer, CTeamsCore, g_Config.
//
// A random game (clients with names, clans, teams, friend/foe/ignore flags, the local
// player) gets random events: chat lines from the server, from players and from unknown
// clients; command infos; key presses (Tab, Shift+Tab, Enter, Escape, Up, Down, others)
// with typed text; opening the chat; time passing. After each event the dump holds the
// chat state the Rust port must reproduce.

#include <base/color.h>
#include <base/log.h>
#include <base/log_color.h>
#include <base/str.h>
#include <base/time.h>

#include <engine/client.h>
#include <engine/console.h>
#include <engine/input.h>
#include <engine/keys.h>
#include <engine/shared/config.h>
#include <engine/shared/protocol.h>
#include <engine/shared/ringbuffer.h>
#include <engine/textrender.h>

#include <game/teamscore.h>

#include <generated/client_data.h>
#include <generated/protocol.h>
#include <generated/protocol7.h>

#include <algorithm>
#include <cstdio>
#include <memory>
#include <optional>
#include <string>
#include <vector>

namespace rs_chat {

static unsigned s_Seed = 4242;
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

// the copies' time(): the dump's "now"
static int64_t s_Now = 0;

enum class EInputPriority
{
	CHAT,
};

// The CLineInput members the copies use (typing is the text field's job).
class CFakeLineInput
{
	char m_aStr[MAX_CHAT_LENGTH] = "";
	size_t m_Cursor = 0;

public:
	void Set(const char *pString)
	{
		str_copy(m_aStr, pString);
		m_Cursor = str_length(m_aStr);
	}
	void Clear() { Set(""); }
	const char *GetString() const { return m_aStr; }
	size_t GetCursorOffset() const { return m_Cursor; }
	void SetCursorOffset(size_t Offset) { m_Cursor = std::min<size_t>(Offset, str_length(m_aStr)); }
	void ProcessInput(const IInput::CEvent &Event) {}
	void Activate(EInputPriority Priority) {}
	void Deactivate() {}
};

struct SFakeTextRender
{
	void DeleteTextContainer(STextContainerIndex &Index) { Index.Reset(); }
};

struct SFakeGraphics
{
	void DeleteQuadContainer(int &Index) { Index = -1; }
};

struct SFakeInput
{
	bool m_Shift = false;
	bool ShiftIsPressed() const { return m_Shift; }
};

struct SFakeEditor
{
	void UpdateMentions() {}
};

// no video is being rendered (the tool is built with CONF_VIDEORECORDER)
struct IVideo
{
	static IVideo *Current() { return nullptr; }
};

class CSounds
{
public:
	enum
	{
		CHN_GUI,
	};
	void Play(int Channel, int SetId, float Volume) { fprintf(s_pOut, " sound%d", SetId); }
};

struct SFakeClient
{
	int m_State = IClient::STATE_ONLINE;
	int State() const { return m_State; }
	bool IsSixup() const { return false; }
	void Notify(const char *pTitle, const char *pMessage) {}
	void SendPackMsgActive(CNetMsg_Cl_Say *pMsg, int Flags)
	{
		fprintf(s_pOut, " send%d:", pMsg->m_Team);
		Hex(pMsg->m_pMessage);
	}
	void SendPackMsgActive(protocol7::CNetMsg_Cl_Say *pMsg, int Flags, bool NoTranslate) {}
};

struct SManagedTee
{
};

// The members of the real CGameClient that the copies read.
class CGameClient
{
public:
	class CClientData
	{
	public:
		char m_aName[MAX_NAME_LENGTH] = "";
		bool m_Active = false;
		bool m_ChatIgnore = false;
		bool m_Friend = false;
		bool m_Foe = false;
		int m_Team = 0;
		bool m_OtherTeam = false;
	};
	struct SSnap
	{
		int m_LocalClientId = -1;
		const CNetObj_PlayerInfo *m_apInfoByName[MAX_CLIENTS] = {};
	};
	CClientData m_aClients[MAX_CLIENTS];
	SSnap m_Snap;
	int m_aLocalIds[2] = {-1, -1};
	CTeamsCore m_Teams;
	bool m_TeamPlay = false;
	bool m_SuppressEvents = false;
	CSounds m_Sounds;
	SFakeClient m_Client;
	SFakeEditor m_Editor;
	bool m_Released = false;

	bool IsOtherTeam(int ClientId) const { return m_aClients[ClientId].m_OtherTeam; }
	bool IsTeamPlay() const { return m_TeamPlay; }
	std::shared_ptr<SManagedTee> CreateManagedTeeRenderInfo(const CClientData &Client) { return std::make_shared<SManagedTee>(); }
	SFakeClient *Client() { return &m_Client; }
	SFakeEditor *Editor() { return &m_Editor; }
	void OnRelease() { m_Released = true; }
};

// The members of the real CChat (chat.h) that the copies use.
class CChat
{
public:
	enum
	{
		MAX_LINES = 64,
	};

	CFakeLineInput m_Input;
	class CLine
	{
	public:
		void Reset(CChat &This);

		bool m_Initialized = false;
		int64_t m_Time = 0;
		float m_aYOffset[2] = {};
		int m_ClientId = 0;
		int m_TeamNumber = 0;
		bool m_Team = false;
		bool m_Whisper = false;
		int m_NameColor = 0;
		char m_aName[64] = "";
		char m_aText[MAX_CHAT_LENGTH] = "";
		bool m_Friend = false;
		bool m_Highlighted = false;
		std::optional<ColorRGBA> m_CustomColor;
		STextContainerIndex m_TextContainerIndex;
		int m_QuadContainerIndex = -1;
		std::shared_ptr<SManagedTee> m_pManagedTeeRenderInfo;
		float m_TextYOffset = 0.0f;
		int m_TimesRepeated = 0;
	};

	CLine m_aLines[MAX_LINES];
	int m_CurrentLine = 0;

	enum
	{
		CLIENT_MSG = -2,
		SERVER_MSG = -1,
	};
	enum
	{
		MODE_NONE = 0,
		MODE_ALL,
		MODE_TEAM,
	};
	enum
	{
		CHAT_SERVER = 0,
		CHAT_HIGHLIGHT,
		CHAT_CLIENT,
		CHAT_NUM,
	};

	int m_Mode = MODE_NONE;
	bool m_Show = false;
	bool m_CompletionUsed = false;
	int m_CompletionChosen = -1;
	char m_aCompletionBuffer[MAX_CHAT_LENGTH] = "";
	int m_PlaceholderOffset = 0;
	int m_PlaceholderLength = 0;
	class CRateablePlayer
	{
	public:
		int m_ClientId;
		int m_Score;
	};
	CRateablePlayer m_aPlayerCompletionList[MAX_CLIENTS];
	int m_PlayerCompletionListLength = 0;

	struct CCommand
	{
		char m_aName[IConsole::TEMPCMD_NAME_LENGTH];
		char m_aParams[IConsole::TEMPCMD_PARAMS_LENGTH];
		char m_aHelpText[IConsole::TEMPCMD_HELP_LENGTH];

		CCommand() = default;
		CCommand(const char *pName, const char *pParams, const char *pHelpText)
		{
			str_copy(m_aName, pName);
			str_copy(m_aParams, pParams);
			str_copy(m_aHelpText, pHelpText);
		}

		bool operator<(const CCommand &Other) const { return str_comp(m_aName, Other.m_aName) < 0; }
		bool operator<=(const CCommand &Other) const { return str_comp(m_aName, Other.m_aName) <= 0; }
		bool operator==(const CCommand &Other) const { return str_comp(m_aName, Other.m_aName) == 0; }
	};

	std::vector<CCommand> m_vServerCommands;
	bool m_ServerCommandsNeedSorting = false;

	struct CHistoryEntry
	{
		int m_Team;
		char m_aText[1];
	};
	CHistoryEntry *m_pHistoryEntry = nullptr;
	CStaticRingBuffer<CHistoryEntry, 64 * 1024, CRingBufferBase::FLAG_RECYCLE> m_History;
	int m_PendingChatCounter = 0;
	int64_t m_LastChatSend = 0;
	int64_t m_aLastSoundPlayed[CHAT_NUM] = {};
	char m_aCurrentInputText[MAX_CHAT_LENGTH] = "";
	bool m_EditingNewLine = true;
	bool m_ServerSupportsCommandInfo = false;

	CGameClient m_GameClient;
	SFakeTextRender m_TextRender;
	SFakeGraphics m_Graphics;
	SFakeInput m_InputState;

	CGameClient *GameClient() { return &m_GameClient; }
	SFakeClient *Client() { return &m_GameClient.m_Client; }
	SFakeTextRender *TextRender() { return &m_TextRender; }
	SFakeGraphics *Graphics() { return &m_Graphics; }
	SFakeInput *Input() { return &m_InputState; }
	int64_t time() const { return s_Now; }
	void StoreSave(const char *pText)
	{
		fprintf(s_pOut, " save:");
		Hex(pText);
	}

	bool LineShouldHighlight(const char *pLine, const char *pName);
	void AddLine(int ClientId, int Team, const char *pLine);
	void EnableMode(int Team);
	void DisableMode();
	void RegisterCommand(const char *pName, const char *pParams, const char *pHelpText);
	void UnregisterCommand(const char *pName);
	void OnMessage(int MsgType, void *pRawMsg);
	bool OnInput(const IInput::CEvent &Event);
	void SendChat(int Team, const char *pLine);
	void SendChatQueued(const char *pLine);
	void OnRenderPending();
};

// C++ copy: game/client/components/chat.cpp:38-49
void CChat::CLine::Reset(CChat &This)
{
	This.TextRender()->DeleteTextContainer(m_TextContainerIndex);
	This.Graphics()->DeleteQuadContainer(m_QuadContainerIndex);
	m_Initialized = false;
	m_Time = 0;
	m_aText[0] = '\0';
	m_aName[0] = '\0';
	m_Friend = false;
	m_TimesRepeated = 0;
	m_pManagedTeeRenderInfo = nullptr;
}
// end of copy

// C++ copy: game/client/components/chat.cpp:88-102
void CChat::RegisterCommand(const char *pName, const char *pParams, const char *pHelpText)
{
	// Don't allow duplicate commands.
	for(const auto &Command : m_vServerCommands)
		if(str_comp(Command.m_aName, pName) == 0)
			return;

	m_vServerCommands.emplace_back(pName, pParams, pHelpText);
	m_ServerCommandsNeedSorting = true;
}

void CChat::UnregisterCommand(const char *pName)
{
	m_vServerCommands.erase(std::remove_if(m_vServerCommands.begin(), m_vServerCommands.end(), [pName](const CCommand &Command) { return str_comp(Command.m_aName, pName) == 0; }), m_vServerCommands.end());
}
// end of copy

// C++ copy: game/client/components/chat.cpp:271-629
bool CChat::OnInput(const IInput::CEvent &Event)
{
	if(m_Mode == MODE_NONE)
		return false;

	if(Event.m_Flags & IInput::FLAG_PRESS && Event.m_Key == KEY_ESCAPE)
	{
		DisableMode();
		GameClient()->OnRelease();
		if(g_Config.m_ClChatReset)
		{
			m_Input.Clear();
			m_pHistoryEntry = nullptr;
			m_EditingNewLine = true;
		}
	}
	else if(Event.m_Flags & IInput::FLAG_PRESS && (Event.m_Key == KEY_RETURN || Event.m_Key == KEY_KP_ENTER))
	{
		if(m_ServerCommandsNeedSorting)
		{
			std::sort(m_vServerCommands.begin(), m_vServerCommands.end());
			m_ServerCommandsNeedSorting = false;
		}

		SendChatQueued(m_Input.GetString());
		m_pHistoryEntry = nullptr;
		m_EditingNewLine = true;
		DisableMode();
		GameClient()->OnRelease();
		m_Input.Clear();
	}
	if(Event.m_Flags & IInput::FLAG_PRESS && Event.m_Key == KEY_TAB)
	{
		const bool ShiftPressed = Input()->ShiftIsPressed();

		// fill the completion buffer
		if(!m_CompletionUsed)
		{
			const char *pCursor = m_Input.GetString() + m_Input.GetCursorOffset();
			for(size_t Count = 0; Count < m_Input.GetCursorOffset() && *(pCursor - 1) != ' '; --pCursor, ++Count)
				;
			m_PlaceholderOffset = pCursor - m_Input.GetString();

			for(m_PlaceholderLength = 0; *pCursor && *pCursor != ' '; ++pCursor)
				++m_PlaceholderLength;

			str_truncate(m_aCompletionBuffer, sizeof(m_aCompletionBuffer), m_Input.GetString() + m_PlaceholderOffset, m_PlaceholderLength);
		}

		if(!m_CompletionUsed && m_aCompletionBuffer[0] != '/')
		{
			// Create the completion list of player names through which the player can iterate
			const char *PlayerName, *FoundInput;
			m_PlayerCompletionListLength = 0;
			for(auto &PlayerInfo : GameClient()->m_Snap.m_apInfoByName)
			{
				if(PlayerInfo)
				{
					PlayerName = GameClient()->m_aClients[PlayerInfo->m_ClientId].m_aName;
					FoundInput = str_utf8_find_nocase(PlayerName, m_aCompletionBuffer);
					if(FoundInput != nullptr)
					{
						m_aPlayerCompletionList[m_PlayerCompletionListLength].m_ClientId = PlayerInfo->m_ClientId;
						// The score for suggesting a player name is determined by the distance of the search input to the beginning of the player name
						m_aPlayerCompletionList[m_PlayerCompletionListLength].m_Score = (int)(FoundInput - PlayerName);
						m_PlayerCompletionListLength++;
					}
				}
			}
			std::stable_sort(m_aPlayerCompletionList, m_aPlayerCompletionList + m_PlayerCompletionListLength,
				[](const CRateablePlayer &Player1, const CRateablePlayer &Player2) -> bool {
					return Player1.m_Score < Player2.m_Score;
				});
		}

		if(m_aCompletionBuffer[0] == '/' && !m_vServerCommands.empty())
		{
			CCommand *pCompletionCommand = nullptr;

			const size_t NumCommands = m_vServerCommands.size();

			if(ShiftPressed && m_CompletionUsed)
				m_CompletionChosen--;
			else if(!ShiftPressed)
				m_CompletionChosen++;
			m_CompletionChosen = (m_CompletionChosen + 2 * NumCommands) % (2 * NumCommands);

			m_CompletionUsed = true;

			const char *pCommandStart = m_aCompletionBuffer + 1;
			for(size_t i = 0; i < 2 * NumCommands; ++i)
			{
				int SearchType;
				int Index;

				if(ShiftPressed)
				{
					SearchType = ((m_CompletionChosen - i + 2 * NumCommands) % (2 * NumCommands)) / NumCommands;
					Index = (m_CompletionChosen - i + NumCommands) % NumCommands;
				}
				else
				{
					SearchType = ((m_CompletionChosen + i) % (2 * NumCommands)) / NumCommands;
					Index = (m_CompletionChosen + i) % NumCommands;
				}

				auto &Command = m_vServerCommands[Index];

				if(str_startswith_nocase(Command.m_aName, pCommandStart))
				{
					pCompletionCommand = &Command;
					m_CompletionChosen = Index + SearchType * NumCommands;
					break;
				}
			}

			// insert the command
			if(pCompletionCommand)
			{
				char aBuf[MAX_CHAT_LENGTH];
				// add part before the name
				str_truncate(aBuf, sizeof(aBuf), m_Input.GetString(), m_PlaceholderOffset);

				// add the command
				str_append(aBuf, "/");
				str_append(aBuf, pCompletionCommand->m_aName);

				// add separator
				const char *pSeparator = pCompletionCommand->m_aParams[0] == '\0' ? "" : " ";
				str_append(aBuf, pSeparator);

				// add part after the name
				str_append(aBuf, m_Input.GetString() + m_PlaceholderOffset + m_PlaceholderLength);

				m_PlaceholderLength = str_length(pSeparator) + str_length(pCompletionCommand->m_aName) + 1;
				m_Input.Set(aBuf);
				m_Input.SetCursorOffset(m_PlaceholderOffset + m_PlaceholderLength);
			}
		}
		else
		{
			// find next possible name
			const char *pCompletionString = nullptr;
			if(m_PlayerCompletionListLength > 0)
			{
				// We do this in a loop, if a player left the game during the repeated pressing of Tab, they are skipped
				CGameClient::CClientData *pCompletionClientData;
				for(int i = 0; i < m_PlayerCompletionListLength; ++i)
				{
					if(ShiftPressed && m_CompletionUsed)
					{
						m_CompletionChosen--;
					}
					else if(!ShiftPressed)
					{
						m_CompletionChosen++;
					}
					if(m_CompletionChosen < 0)
					{
						m_CompletionChosen += m_PlayerCompletionListLength;
					}
					m_CompletionChosen %= m_PlayerCompletionListLength;
					m_CompletionUsed = true;

					pCompletionClientData = &GameClient()->m_aClients[m_aPlayerCompletionList[m_CompletionChosen].m_ClientId];
					if(!pCompletionClientData->m_Active)
					{
						continue;
					}

					pCompletionString = pCompletionClientData->m_aName;
					break;
				}
			}

			// insert the name
			if(pCompletionString)
			{
				char aBuf[MAX_CHAT_LENGTH];
				// add part before the name
				str_truncate(aBuf, sizeof(aBuf), m_Input.GetString(), m_PlaceholderOffset);

				// quote the name
				char aQuoted[128];
				if(m_Input.GetString()[0] == '/' && (str_find(pCompletionString, " ") || str_find(pCompletionString, "\"")))
				{
					// escape the name
					str_copy(aQuoted, "\"");
					char *pDst = aQuoted + str_length(aQuoted);
					str_escape(&pDst, pCompletionString, aQuoted + sizeof(aQuoted));
					str_append(aQuoted, "\"");

					pCompletionString = aQuoted;
				}

				// add the name
				str_append(aBuf, pCompletionString);

				// add separator
				const char *pSeparator = "";
				if(*(m_Input.GetString() + m_PlaceholderOffset + m_PlaceholderLength) != ' ')
					pSeparator = m_PlaceholderOffset == 0 ? ": " : " ";
				else if(m_PlaceholderOffset == 0)
					pSeparator = ":";
				if(*pSeparator)
					str_append(aBuf, pSeparator);

				// add part after the name
				str_append(aBuf, m_Input.GetString() + m_PlaceholderOffset + m_PlaceholderLength);

				m_PlaceholderLength = str_length(pSeparator) + str_length(pCompletionString);
				m_Input.Set(aBuf);
				m_Input.SetCursorOffset(m_PlaceholderOffset + m_PlaceholderLength);
			}
		}
	}
	else
	{
		// reset name completion process
		if(Event.m_Flags & IInput::FLAG_PRESS && Event.m_Key != KEY_TAB && Event.m_Key != KEY_LSHIFT && Event.m_Key != KEY_RSHIFT)
		{
			m_CompletionChosen = -1;
			m_CompletionUsed = false;
		}

		m_Input.ProcessInput(Event);
	}

	if(Event.m_Flags & IInput::FLAG_PRESS && Event.m_Key == KEY_UP)
	{
		if(m_EditingNewLine)
		{
			str_copy(m_aCurrentInputText, m_Input.GetString());
			m_EditingNewLine = false;
		}

		if(m_pHistoryEntry)
		{
			CHistoryEntry *pTest = m_History.Prev(m_pHistoryEntry);

			if(pTest)
				m_pHistoryEntry = pTest;
		}
		else
		{
			m_pHistoryEntry = m_History.Last();
		}

		if(m_pHistoryEntry)
			m_Input.Set(m_pHistoryEntry->m_aText);
	}
	else if(Event.m_Flags & IInput::FLAG_PRESS && Event.m_Key == KEY_DOWN)
	{
		if(m_pHistoryEntry)
			m_pHistoryEntry = m_History.Next(m_pHistoryEntry);

		if(m_pHistoryEntry)
		{
			m_Input.Set(m_pHistoryEntry->m_aText);
		}
		else if(!m_EditingNewLine)
		{
			m_Input.Set(m_aCurrentInputText);
			m_EditingNewLine = true;
		}
	}

	return true;
}

void CChat::EnableMode(int Team)
{
	if(Client()->State() == IClient::STATE_DEMOPLAYBACK)
		return;

	if(m_Mode == MODE_NONE)
	{
		if(Team)
			m_Mode = MODE_TEAM;
		else
			m_Mode = MODE_ALL;

		m_CompletionChosen = -1;
		m_CompletionUsed = false;
		m_Input.Activate(EInputPriority::CHAT);
	}
}

void CChat::DisableMode()
{
	if(m_Mode != MODE_NONE)
	{
		m_Mode = MODE_NONE;
		m_Input.Deactivate();
	}
}

void CChat::OnMessage(int MsgType, void *pRawMsg)
{
	if(GameClient()->m_SuppressEvents)
		return;

	if(MsgType == NETMSGTYPE_SV_CHAT)
	{
		CNetMsg_Sv_Chat *pMsg = (CNetMsg_Sv_Chat *)pRawMsg;

		/*
		if(g_Config.m_ClCensorChat)
		{
			char aMessage[MAX_CHAT_LENGTH];
			str_copy(aMessage, pMsg->m_pMessage);
			GameClient()->m_Censor.CensorMessage(aMessage);
			AddLine(pMsg->m_ClientId, pMsg->m_Team, aMessage);
		}
		else
			AddLine(pMsg->m_ClientId, pMsg->m_Team, pMsg->m_pMessage);
		*/

		AddLine(pMsg->m_ClientId, pMsg->m_Team, pMsg->m_pMessage);

		if(Client()->State() != IClient::STATE_DEMOPLAYBACK &&
			pMsg->m_ClientId == SERVER_MSG)
		{
			StoreSave(pMsg->m_pMessage);
		}
	}
	else if(MsgType == NETMSGTYPE_SV_COMMANDINFO)
	{
		CNetMsg_Sv_CommandInfo *pMsg = (CNetMsg_Sv_CommandInfo *)pRawMsg;
		if(!m_ServerSupportsCommandInfo)
		{
			m_vServerCommands.clear();
			m_ServerSupportsCommandInfo = true;
		}
		RegisterCommand(pMsg->m_pName, pMsg->m_pArgsFormat, pMsg->m_pHelpText);
	}
	else if(MsgType == NETMSGTYPE_SV_COMMANDINFOREMOVE)
	{
		CNetMsg_Sv_CommandInfoRemove *pMsg = (CNetMsg_Sv_CommandInfoRemove *)pRawMsg;
		UnregisterCommand(pMsg->m_pName);
	}
}

bool CChat::LineShouldHighlight(const char *pLine, const char *pName)
{
	const char *pHit = str_utf8_find_nocase(pLine, pName);

	while(pHit)
	{
		int Length = str_length(pName);

		if(Length > 0 && (pLine == pHit || pHit[-1] == ' ') && (pHit[Length] == 0 || pHit[Length] == ' ' || pHit[Length] == '.' || pHit[Length] == '!' || pHit[Length] == ',' || pHit[Length] == '?' || pHit[Length] == ':'))
			return true;

		pHit = str_utf8_find_nocase(pHit + 1, pName);
	}

	return false;
}
// end of copy

// C++ copy: game/client/components/chat.cpp:680-941
void CChat::AddLine(int ClientId, int Team, const char *pLine)
{
	if(*pLine == 0 ||
		(ClientId == SERVER_MSG && !g_Config.m_ClShowChatSystem) ||
		(ClientId >= 0 && (GameClient()->m_aClients[ClientId].m_aName[0] == '\0' || // unknown client
					  GameClient()->m_aClients[ClientId].m_ChatIgnore ||
					  (GameClient()->m_Snap.m_LocalClientId != ClientId && g_Config.m_ClShowChatFriends && !GameClient()->m_aClients[ClientId].m_Friend) ||
					  (GameClient()->m_Snap.m_LocalClientId != ClientId && g_Config.m_ClShowChatTeamMembersOnly && GameClient()->IsOtherTeam(ClientId) && GameClient()->m_Teams.Team(GameClient()->m_Snap.m_LocalClientId) != TEAM_FLOCK) ||
					  (GameClient()->m_Snap.m_LocalClientId != ClientId && GameClient()->m_aClients[ClientId].m_Foe))))
		return;

	// trim right and set maximum length to 256 utf8-characters
	int Length = 0;
	const char *pStr = pLine;
	const char *pEnd = nullptr;
	while(*pStr)
	{
		const char *pStrOld = pStr;
		int Code = str_utf8_decode(&pStr);

		// check if unicode is not empty
		if(!str_utf8_isspace(Code))
		{
			pEnd = nullptr;
		}
		else if(pEnd == nullptr)
		{
			pEnd = pStrOld;
		}

		if(++Length >= MAX_CHAT_LENGTH)
		{
			*(const_cast<char *>(pStr)) = '\0';
			break;
		}
	}
	if(pEnd != nullptr)
		*(const_cast<char *>(pEnd)) = '\0';

	if(*pLine == 0)
		return;

	bool Highlighted = false;

	auto &&FChatMsgCheckAndPrint = [](const CLine &Line) {
		ColorRGBA ChatLogColor = ColorRGBA(1.0f, 1.0f, 1.0f, 1.0f);
		if(Line.m_Highlighted)
		{
			ChatLogColor = color_cast<ColorRGBA>(ColorHSLA(g_Config.m_ClMessageHighlightColor));
		}
		else
		{
			if(Line.m_Friend && g_Config.m_ClMessageFriend)
				ChatLogColor = color_cast<ColorRGBA>(ColorHSLA(g_Config.m_ClMessageFriendColor));
			else if(Line.m_Team)
				ChatLogColor = color_cast<ColorRGBA>(ColorHSLA(g_Config.m_ClMessageTeamColor));
			else if(Line.m_ClientId == SERVER_MSG)
				ChatLogColor = color_cast<ColorRGBA>(ColorHSLA(g_Config.m_ClMessageSystemColor));
			else if(Line.m_ClientId == CLIENT_MSG)
				ChatLogColor = color_cast<ColorRGBA>(ColorHSLA(g_Config.m_ClMessageClientColor));
			else // regular message
				ChatLogColor = color_cast<ColorRGBA>(ColorHSLA(g_Config.m_ClMessageColor));
		}

		const char *pFrom;
		if(Line.m_Whisper)
			pFrom = "chat/whisper";
		else if(Line.m_Team)
			pFrom = "chat/team";
		else if(Line.m_ClientId == SERVER_MSG)
			pFrom = "chat/server";
		else if(Line.m_ClientId == CLIENT_MSG)
			pFrom = "chat/client";
		else
			pFrom = "chat/all";

		log_info_color(color_cast<LOG_COLOR>(ChatLogColor), pFrom, "%s%s%s", Line.m_aName, Line.m_ClientId >= 0 ? ": " : "", Line.m_aText);
	};

	// Custom color for new line
	std::optional<ColorRGBA> CustomColor = std::nullopt;
	if(ClientId == CLIENT_MSG)
		CustomColor = color_cast<ColorRGBA>(ColorHSLA(g_Config.m_ClMessageClientColor));

	CLine &PreviousLine = m_aLines[m_CurrentLine];

	// Team Number:
	// 0 = global; 1 = team; 2 = sending whisper; 3 = receiving whisper

	// If it's a client message, m_aText will have ": " prepended so we have to work around it.
	if(PreviousLine.m_Initialized &&
		PreviousLine.m_TeamNumber == Team &&
		PreviousLine.m_ClientId == ClientId &&
		str_comp(PreviousLine.m_aText, pLine) == 0 &&
		PreviousLine.m_CustomColor == CustomColor)
	{
		PreviousLine.m_TimesRepeated++;
		TextRender()->DeleteTextContainer(PreviousLine.m_TextContainerIndex);
		Graphics()->DeleteQuadContainer(PreviousLine.m_QuadContainerIndex);
		PreviousLine.m_Time = time();
		PreviousLine.m_aYOffset[0] = -1.0f;
		PreviousLine.m_aYOffset[1] = -1.0f;

		FChatMsgCheckAndPrint(PreviousLine);
		return;
	}

	m_CurrentLine = (m_CurrentLine + 1) % MAX_LINES;

	CLine &CurrentLine = m_aLines[m_CurrentLine];
	CurrentLine.Reset(*this);
	CurrentLine.m_Initialized = true;
	CurrentLine.m_Time = time();
	CurrentLine.m_aYOffset[0] = -1.0f;
	CurrentLine.m_aYOffset[1] = -1.0f;
	CurrentLine.m_ClientId = ClientId;
	CurrentLine.m_TeamNumber = Team;
	CurrentLine.m_Team = Team == 1;
	CurrentLine.m_Whisper = Team >= 2;
	CurrentLine.m_NameColor = -2;
	CurrentLine.m_CustomColor = CustomColor;

	// check for highlighted name
	if(Client()->State() != IClient::STATE_DEMOPLAYBACK)
	{
		if(ClientId >= 0 && ClientId != GameClient()->m_aLocalIds[0] && ClientId != GameClient()->m_aLocalIds[1])
		{
			for(int LocalId : GameClient()->m_aLocalIds)
			{
				Highlighted |= LocalId >= 0 && LineShouldHighlight(pLine, GameClient()->m_aClients[LocalId].m_aName);
			}
		}
	}
	else
	{
		// on demo playback use local id from snap directly,
		// since m_aLocalIds isn't valid there
		Highlighted |= GameClient()->m_Snap.m_LocalClientId >= 0 && LineShouldHighlight(pLine, GameClient()->m_aClients[GameClient()->m_Snap.m_LocalClientId].m_aName);
	}
	CurrentLine.m_Highlighted = Highlighted;

	str_copy(CurrentLine.m_aText, pLine);

	if(CurrentLine.m_ClientId == SERVER_MSG)
	{
		str_copy(CurrentLine.m_aName, "*** ");
	}
	else if(CurrentLine.m_ClientId == CLIENT_MSG)
	{
		str_copy(CurrentLine.m_aName, "— ");
	}
	else
	{
		const auto &LineAuthor = GameClient()->m_aClients[CurrentLine.m_ClientId];

		if(LineAuthor.m_Active)
		{
			if(LineAuthor.m_Team == TEAM_SPECTATORS)
				CurrentLine.m_NameColor = TEAM_SPECTATORS;

			if(GameClient()->IsTeamPlay())
			{
				if(LineAuthor.m_Team == TEAM_RED)
					CurrentLine.m_NameColor = TEAM_RED;
				else if(LineAuthor.m_Team == TEAM_BLUE)
					CurrentLine.m_NameColor = TEAM_BLUE;
			}
		}

		if(Team == TEAM_WHISPER_SEND)
		{
			str_copy(CurrentLine.m_aName, "→");
			if(LineAuthor.m_Active)
			{
				str_append(CurrentLine.m_aName, " ");
				str_append(CurrentLine.m_aName, LineAuthor.m_aName);
			}
			CurrentLine.m_NameColor = TEAM_BLUE;
			CurrentLine.m_Highlighted = false;
			Highlighted = false;
		}
		else if(Team == TEAM_WHISPER_RECV)
		{
			str_copy(CurrentLine.m_aName, "←");
			if(LineAuthor.m_Active)
			{
				str_append(CurrentLine.m_aName, " ");
				str_append(CurrentLine.m_aName, LineAuthor.m_aName);
			}
			CurrentLine.m_NameColor = TEAM_RED;
			CurrentLine.m_Highlighted = true;
			Highlighted = true;
		}
		else
		{
			str_copy(CurrentLine.m_aName, LineAuthor.m_aName);
		}

		if(LineAuthor.m_Active)
		{
			CurrentLine.m_Friend = LineAuthor.m_Friend;
			CurrentLine.m_pManagedTeeRenderInfo = GameClient()->CreateManagedTeeRenderInfo(LineAuthor);
		}
	}

	FChatMsgCheckAndPrint(CurrentLine);

	// play sound
	int64_t Now = time();
	if(ClientId == SERVER_MSG)
	{
		if(Now - m_aLastSoundPlayed[CHAT_SERVER] >= time_freq() * 3 / 10)
		{
			if(g_Config.m_SndServerMessage)
			{
				GameClient()->m_Sounds.Play(CSounds::CHN_GUI, SOUND_CHAT_SERVER, 1.0f);
				m_aLastSoundPlayed[CHAT_SERVER] = Now;
			}
		}
	}
	else if(ClientId == CLIENT_MSG)
	{
		// No sound yet
	}
	else if(Highlighted && Client()->State() != IClient::STATE_DEMOPLAYBACK)
	{
		if(Now - m_aLastSoundPlayed[CHAT_HIGHLIGHT] >= time_freq() * 3 / 10)
		{
			char aBuf[1024];
			str_format(aBuf, sizeof(aBuf), "%s: %s", CurrentLine.m_aName, CurrentLine.m_aText);
			Client()->Notify("DDNet Chat", aBuf);
			if(g_Config.m_SndHighlight)
			{
				GameClient()->m_Sounds.Play(CSounds::CHN_GUI, SOUND_CHAT_HIGHLIGHT, 1.0f);
				m_aLastSoundPlayed[CHAT_HIGHLIGHT] = Now;
			}

			if(g_Config.m_ClEditor)
			{
				GameClient()->Editor()->UpdateMentions();
			}
		}
	}
	else if(Team != TEAM_WHISPER_SEND)
	{
		if(Now - m_aLastSoundPlayed[CHAT_CLIENT] >= time_freq() * 3 / 10)
		{
			bool PlaySound = CurrentLine.m_Team ? g_Config.m_SndTeamChat : g_Config.m_SndChat;
#if defined(CONF_VIDEORECORDER)
			if(IVideo::Current())
			{
				PlaySound &= (bool)g_Config.m_ClVideoShowChat;
			}
#endif
			if(PlaySound)
			{
				GameClient()->m_Sounds.Play(CSounds::CHN_GUI, SOUND_CHAT_CLIENT, 1.0f);
				m_aLastSoundPlayed[CHAT_CLIENT] = Now;
			}
		}
	}
}
// end of copy

// C++ copy: game/client/components/chat.cpp:1363-1413
void CChat::SendChat(int Team, const char *pLine)
{
	// don't send empty messages
	if(*str_utf8_skip_whitespaces(pLine) == '\0')
		return;

	m_LastChatSend = time();

	if(GameClient()->Client()->IsSixup())
	{
		protocol7::CNetMsg_Cl_Say Msg7;
		Msg7.m_Mode = Team == 1 ? protocol7::CHAT_TEAM : protocol7::CHAT_ALL;
		Msg7.m_Target = -1;
		Msg7.m_pMessage = pLine;
		Client()->SendPackMsgActive(&Msg7, MSGFLAG_VITAL, true);
		return;
	}

	// send chat message
	CNetMsg_Cl_Say Msg;
	Msg.m_Team = Team;
	Msg.m_pMessage = pLine;
	Client()->SendPackMsgActive(&Msg, MSGFLAG_VITAL);
}

void CChat::SendChatQueued(const char *pLine)
{
	if(!pLine || str_length(pLine) < 1)
		return;

	bool AddEntry = false;

	if(m_LastChatSend + time_freq() < time())
	{
		SendChat(m_Mode == MODE_ALL ? 0 : 1, pLine);
		AddEntry = true;
	}
	else if(m_PendingChatCounter < 3)
	{
		++m_PendingChatCounter;
		AddEntry = true;
	}

	if(AddEntry)
	{
		const int Length = str_length(pLine);
		CHistoryEntry *pEntry = m_History.Allocate(sizeof(CHistoryEntry) + Length);
		pEntry->m_Team = m_Mode == MODE_ALL ? 0 : 1;
		str_copy(pEntry->m_aText, pLine, Length + 1);
	}
}
// end of copy

void CChat::OnRenderPending()
{
// C++ copy: game/client/components/chat.cpp:1179-1192
	// send pending chat messages
	if(m_PendingChatCounter > 0 && m_LastChatSend + time_freq() < time())
	{
		CHistoryEntry *pEntry = m_History.Last();
		for(int i = m_PendingChatCounter - 1; pEntry; --i, pEntry = m_History.Prev(pEntry))
		{
			if(i == 0)
			{
				SendChat(pEntry->m_Team, pEntry->m_aText);
				break;
			}
		}
		--m_PendingChatCounter;
	}
// end of copy
}

static const char *const s_apNames[] = {"nameless tee", "Tee", "tee", "Жук", "жук mini", "a b", "q\"x", "brainless tee", "Teeworlds", "ab", "A", "Ölf", "x.y", "ninja", "nin", "Gruß"};
static const char *const s_apCommands[] = {"pause", "practice", "team", "timeout", "tc", "t", "spec", "w", "whisper", "save", "load", "rank"};
static const char *const s_apWords[] = {"hi", "gg", "lol", "/pause", "/t", "/w ", "/p", " ", "  ", "\xe3\x80\x80", "\xc2\xa0", "!", ".", ":", ",", "?", "\xe2\x80\x8b", "x"};
static const char *const s_apInputs[] = {"", "/", "/p", "/pa", "/t", "/t ", "/w n", "/w \"a", "te", "Ж", "hello ", "a", "hi te", "x /p", "/zz", "/save abc", "  "};

static void RandomText(char *pBuf, int Size, const char *pLocalName)
{
	pBuf[0] = '\0';
	const int Kind = Rand() % 10;
	if(Kind == 0)
	{
		// long lines (the 256 code point and 255 byte limits)
		const int Len = 200 + Rand() % 200;
		for(int i = 0; i < Len; i++)
			str_append(pBuf, Rand() % 7 == 0 ? "\xd0\x96" : (Rand() % 5 == 0 ? " " : "a"), Size);
		return;
	}
	if(Kind == 1)
	{
		str_copy(pBuf, "Team successfully saved by ", Size);
		str_append(pBuf, s_apNames[Rand() % std::size(s_apNames)], Size);
		str_append(pBuf, Rand() % 2 ? ". Use '/load code word here' on ddnet.org to continue" : ". Use '/load abc' to continue", Size);
		return;
	}
	const int Words = Rand() % 6;
	for(int i = 0; i < Words; i++)
	{
		if(i)
			str_append(pBuf, Rand() % 4 ? " " : "", Size);
		if(Rand() % 3 == 0)
		{
			// often the local name (highlights), often with punctuation right after it
			str_append(pBuf, pLocalName[0] && Rand() % 2 ? pLocalName : s_apNames[Rand() % std::size(s_apNames)], Size);
			static const char *const s_apAfter[] = {":", ",", ".", "!", "?", "x", "-"};
			if(Rand() % 2)
				str_append(pBuf, s_apAfter[Rand() % std::size(s_apAfter)], Size);
		}
		else
			str_append(pBuf, s_apWords[Rand() % std::size(s_apWords)], Size);
	}
}

static void DumpLine(const CChat::CLine &L)
{
	fprintf(s_pOut, " %d %lld %d %d %d %d %d ", L.m_Initialized, (long long)L.m_Time, L.m_ClientId, L.m_TeamNumber, L.m_Team, L.m_Whisper, L.m_NameColor);
	Hex(L.m_aName);
	fprintf(s_pOut, " ");
	Hex(L.m_aText);
	fprintf(s_pOut, " %d %d %d %d %d", L.m_Friend, L.m_Highlighted, L.m_CustomColor.has_value(), L.m_TimesRepeated, L.m_pManagedTeeRenderInfo != nullptr);
}

static void DumpState(CChat &C)
{
	fprintf(s_pOut, " = %d ", C.m_Mode);
	Hex(C.m_Input.GetString());
	fprintf(s_pOut, " %d %d %d %d %d %d %d %d %d", (int)C.m_Input.GetCursorOffset(), C.m_CompletionUsed, C.m_CompletionChosen,
		C.m_PlaceholderOffset, C.m_PlaceholderLength, C.m_PendingChatCounter, C.m_CurrentLine, C.m_Show, C.m_GameClient.m_Released);
	fprintf(s_pOut, " |");
	DumpLine(C.m_aLines[C.m_CurrentLine]);
	fprintf(s_pOut, "\n");
}

static int Run(FILE *f)
{
	s_pOut = f;
	const int Runs = 40;
	const int Events = 400;
	fprintf(f, "chat %d %d\n", Runs, Events);
	for(int r = 0; r < Runs; r++)
	{
		std::unique_ptr<CChat> pChat = std::make_unique<CChat>();
		CChat &C = *pChat;
		CGameClient &G = C.m_GameClient;
		s_Now = 1000000000ll * (1 + Rand() % 1000);

		g_Config.m_ClShowChatSystem = Rand() % 6 != 0;
		g_Config.m_ClShowChatFriends = Rand() % 8 == 0;
		g_Config.m_ClShowChatTeamMembersOnly = Rand() % 4 == 0;
		g_Config.m_ClChatReset = Rand() % 2;
		g_Config.m_SndServerMessage = Rand() % 4 != 0;
		g_Config.m_SndHighlight = Rand() % 4 != 0;
		g_Config.m_SndTeamChat = Rand() % 4 != 0;
		g_Config.m_SndChat = Rand() % 4 != 0;
		G.m_TeamPlay = Rand() % 3 == 0;
		G.m_Client.m_State = Rand() % 8 == 0 ? IClient::STATE_DEMOPLAYBACK : IClient::STATE_ONLINE;
		fprintf(f, "run %d %d %d %d %d %d %d %d %d %d %d %lld\n", r, g_Config.m_ClShowChatSystem, g_Config.m_ClShowChatFriends,
			g_Config.m_ClShowChatTeamMembersOnly, g_Config.m_ClChatReset, g_Config.m_SndServerMessage, g_Config.m_SndHighlight,
			g_Config.m_SndTeamChat, g_Config.m_SndChat, G.m_TeamPlay, G.m_Client.m_State == IClient::STATE_DEMOPLAYBACK, (long long)s_Now);

		// clients: some active, some only with a name (a ClientInfo but no PlayerInfo)
		static CNetObj_PlayerInfo s_aInfos[MAX_CLIENTS];
		std::vector<int> vActive;
		const int NumClients = 1 + Rand() % 10;
		for(int i = 0; i < NumClients; i++)
		{
			const int Id = Rand() % MAX_CLIENTS;
			CGameClient::CClientData &Client = G.m_aClients[Id];
			str_copy(Client.m_aName, s_apNames[Rand() % std::size(s_apNames)]);
			Client.m_Active = Rand() % 5 != 0;
			Client.m_ChatIgnore = Rand() % 10 == 0;
			Client.m_Friend = Rand() % 4 == 0;
			Client.m_Foe = Rand() % 10 == 0;
			Client.m_Team = (int)(Rand() % 4) - 1;
			Client.m_OtherTeam = Rand() % 3 == 0;
			G.m_Teams.Team(Id, Rand() % 3);
			if(Client.m_Active && std::find(vActive.begin(), vActive.end(), Id) == vActive.end())
				vActive.push_back(Id);
		}
		G.m_Snap.m_LocalClientId = vActive.empty() ? -1 : vActive[Rand() % vActive.size()];
		G.m_aLocalIds[0] = G.m_Snap.m_LocalClientId;
		G.m_aLocalIds[1] = Rand() % 4 == 0 && !vActive.empty() ? vActive[Rand() % vActive.size()] : -1;
		// C++ IsOtherTeam is false without a local player (gameclient.cpp:3981)
		if(G.m_Snap.m_LocalClientId < 0)
			for(auto &Client : G.m_aClients)
				Client.m_OtherTeam = false;
		// m_apInfoByName: the active clients sorted by name (stable, by id first)
		auto SortByName = [&]() {
			vActive.clear();
			for(int Id = 0; Id < MAX_CLIENTS; Id++)
				if(G.m_aClients[Id].m_Active)
					vActive.push_back(Id);
			std::stable_sort(vActive.begin(), vActive.end(), [&](int a, int b) { return str_comp_nocase(G.m_aClients[a].m_aName, G.m_aClients[b].m_aName) < 0; });
			std::fill(std::begin(G.m_Snap.m_apInfoByName), std::end(G.m_Snap.m_apInfoByName), nullptr);
			for(size_t i = 0; i < vActive.size(); i++)
			{
				s_aInfos[vActive[i]].m_ClientId = vActive[i];
				G.m_Snap.m_apInfoByName[i] = &s_aInfos[vActive[i]];
			}
		};
		SortByName();
		std::vector<int> vNamed;
		for(int Id = 0; Id < MAX_CLIENTS; Id++)
			if(G.m_aClients[Id].m_aName[0])
				vNamed.push_back(Id);
		for(int Id = 0; Id < MAX_CLIENTS; Id++)
		{
			const CGameClient::CClientData &Client = G.m_aClients[Id];
			if(!Client.m_aName[0])
				continue;
			fprintf(f, "client %d ", Id);
			Hex(Client.m_aName);
			fprintf(f, " %d %d %d %d %d %d %d\n", Client.m_Active, Client.m_ChatIgnore, Client.m_Friend, Client.m_Foe, Client.m_Team,
				Client.m_OtherTeam, G.m_Teams.Team(Id));
		}
		fprintf(f, "local %d %d %d |", G.m_Snap.m_LocalClientId, G.m_aLocalIds[0], G.m_aLocalIds[1]);
		for(int Id : vActive)
			fprintf(f, " %d", Id);
		fprintf(f, "\n");

		for(int e = 0; e < Events; e++)
		{
			G.m_Released = false;
			const int Kind = Rand() % 24;
			if(Kind < 6)
			{
				char aText[1024];
				RandomText(aText, sizeof(aText), G.m_Snap.m_LocalClientId >= 0 ? G.m_aClients[G.m_Snap.m_LocalClientId].m_aName : "");
				const int Pick = Rand() % 6;
				int ClientId = Pick == 0 ? -1 : Pick == 1 ? (int)(Rand() % MAX_CLIENTS) : Pick == 2 ? G.m_Snap.m_LocalClientId : vActive.empty() ? -1 : vActive[Rand() % vActive.size()];
				if(Rand() % 5 == 0 && C.m_aLines[C.m_CurrentLine].m_Initialized)
				{
					// a repeat of the last line
					str_copy(aText, C.m_aLines[C.m_CurrentLine].m_aText);
					ClientId = C.m_aLines[C.m_CurrentLine].m_ClientId;
				}
				const int Team = Rand() % 3 ? 0 : (int)(Rand() % 4);
				if(ClientId < -1)
					ClientId = -1;
				fprintf(f, "msg %d %d ", ClientId, Team);
				Hex(aText);
				CNetMsg_Sv_Chat Msg;
				Msg.m_Team = Team;
				Msg.m_ClientId = ClientId;
				Msg.m_pMessage = aText;
				C.OnMessage(NETMSGTYPE_SV_CHAT, &Msg);
			}
			else if(Kind == 6 || Kind == 20)
			{
				const char *pName = s_apCommands[Rand() % std::size(s_apCommands)];
				const char *pParams = Rand() % 2 ? "" : "?r[message]";
				if(Rand() % 4)
				{
					fprintf(f, "cmd ");
					Hex(pName);
					fprintf(f, " ");
					Hex(pParams);
					CNetMsg_Sv_CommandInfo Msg;
					Msg.m_pName = pName;
					Msg.m_pArgsFormat = pParams;
					Msg.m_pHelpText = "help";
					C.OnMessage(NETMSGTYPE_SV_COMMANDINFO, &Msg);
				}
				else
				{
					fprintf(f, "cmdrm ");
					Hex(pName);
					CNetMsg_Sv_CommandInfoRemove Msg;
					Msg.m_pName = pName;
					C.OnMessage(NETMSGTYPE_SV_COMMANDINFOREMOVE, &Msg);
				}
			}
			else if(Kind == 7)
			{
				const int Team = Rand() % 2;
				fprintf(f, "enable %d", Team);
				C.EnableMode(Team);
			}
			else if(Kind == 8)
			{
				// typing: a key press, then the text field's new text and cursor
				static const char *const s_apPrefixes[] = {"/s", "/sa", "/w", "/t", "/ti", "/r", "/l", "/p", "/pr"};
				const char *pText = Rand() % 3 ? s_apInputs[Rand() % std::size(s_apInputs)] : s_apPrefixes[Rand() % std::size(s_apPrefixes)];
				char aText[MAX_CHAT_LENGTH];
				str_copy(aText, pText);
				if(Rand() % 3 == 0)
					str_append(aText, s_apNames[Rand() % std::size(s_apNames)]);
				const int Cursor = Rand() % 3 ? str_length(aText) : (int)(Rand() % (str_length(aText) + 1));
				fprintf(f, "type ");
				Hex(aText);
				fprintf(f, " %d", Cursor);
				IInput::CEvent Event = {IInput::FLAG_PRESS, KEY_A, 0, ""};
				C.OnInput(Event);
				if(C.m_Mode != CChat::MODE_NONE)
				{
					C.m_Input.Set(aText);
					C.m_Input.SetCursorOffset(Cursor);
				}
			}
			else if(Kind < 15)
			{
				static const int s_aKeys[] = {KEY_TAB, KEY_TAB, KEY_TAB, KEY_RETURN, KEY_ESCAPE, KEY_UP, KEY_DOWN, KEY_LSHIFT, KEY_B};
				static const int s_aCommandKeys[] = {KEY_TAB, KEY_TAB, KEY_TAB, KEY_TAB, KEY_B, KEY_RETURN};
				// command completion cycles: mostly Tab and Shift+Tab
				const bool Command = C.m_Input.GetString()[0] == '/';
				const int Key = Command ? s_aCommandKeys[Rand() % std::size(s_aCommandKeys)] : s_aKeys[Rand() % std::size(s_aKeys)];
				C.m_InputState.m_Shift = Rand() % (Command ? 2 : 3) == 0;
				fprintf(f, "key %d %d", Key, C.m_InputState.m_Shift);
				IInput::CEvent Event = {IInput::FLAG_PRESS, Key, 0, ""};
				C.OnInput(Event);
			}
			else if(Kind == 21)
			{
				// a quick message: open, type, Enter (several within a second queue up)
				const int Team = Rand() % 2;
				char aText[MAX_CHAT_LENGTH];
				str_copy(aText, s_apInputs[Rand() % std::size(s_apInputs)]);
				str_append(aText, s_apWords[Rand() % std::size(s_apWords)]);
				fprintf(f, "say %d ", Team);
				Hex(aText);
				C.EnableMode(Team);
				IInput::CEvent Event = {IInput::FLAG_PRESS, KEY_A, 0, ""};
				C.OnInput(Event);
				if(C.m_Mode != CChat::MODE_NONE)
					C.m_Input.Set(aText);
				Event.m_Key = KEY_RETURN;
				C.OnInput(Event);
			}
			else if(Kind == 22 || Kind == 19)
			{
				// a client leaves or comes back (the completion list keeps the old ids)
				int Id = vNamed[Rand() % vNamed.size()];
				if(C.m_CompletionUsed && C.m_PlayerCompletionListLength > 0 && Rand() % 2)
					Id = C.m_aPlayerCompletionList[Rand() % C.m_PlayerCompletionListLength].m_ClientId;
				G.m_aClients[Id].m_Active = !G.m_aClients[Id].m_Active;
				SortByName();
				fprintf(f, "active %d %d", Id, G.m_aClients[Id].m_Active);
			}
			else if(Kind < 18 || Kind == 23)
			{
				// sometimes exactly at the 0.3 s sound and 1 s send limits
				static const int s_aEdges[] = {299, 300, 301, 999, 1000, 1001};
				if(Rand() % 4 == 0)
					s_Now += (int64_t)s_aEdges[Rand() % std::size(s_aEdges)] * 1000000ll;
				else
					s_Now += Rand() % 3 == 0 ? (int64_t)(Rand() % 20) * 1000000000ll : (int64_t)(Rand() % (Rand() % 2 ? 300 : 1500)) * 1000000ll;
				fprintf(f, "time %lld", (long long)s_Now);
				C.OnRenderPending();
			}
			else
			{
				fprintf(f, "render");
				C.OnRenderPending();
			}
			DumpState(C);
		}
		fprintf(f, "lines");
		for(int i = 0; i < CChat::MAX_LINES; i++)
		{
			fprintf(f, " %d", i);
			DumpLine(C.m_aLines[i]);
			fprintf(f, " ;");
		}
		fprintf(f, "\n");
	}
	return 0;
}

} // namespace rs_chat

int ChatMode(const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	const int Result = rs_chat::Run(f);
	fclose(f);
	return Result;
}
