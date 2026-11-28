// "votes" mode of rs_parity: the vote state and option list (PLAN §13.7), for
// rust-client/client/tests/vote.rs.
//
// CVoting needs the client, so its message handling and option list code is copied here
// word for word into a stand-in class (in its own namespace); the copies are marked
// "C++ copy: <file>:<lines>" ... "end of copy" and `cpp_copies_match_source` checks them.
// Random vote messages (strings with spaces, multi-byte characters and over-long ones)
// run through it; after each the whole state is written out. Strings are hex.

#include <base/str.h>
#include <base/time.h>

#include <engine/client.h>
#include <engine/shared/config.h>
#include <engine/shared/protocol.h>

#include <game/voting.h>

#include <generated/client_data.h>
#include <generated/protocol.h>

#include <cstdio>
#include <string>

namespace votes_copy {

static int64_t s_Now = 0;

struct CHeap
{
	void Reset() {}
	template<typename T>
	T *Allocate()
	{
		return new T();
	}
};

struct CClientStub
{
	int State() const { return IClient::STATE_ONLINE; }
	bool RconAuthed() const { return false; }
	void Notify(const char *, const char *) {}
};

struct CSounds
{
	enum
	{
		CHN_GUI,
	};
	void Play(int, int, float) {}
};

struct CScoreboardStub
{
	bool m_Active = false;
	bool IsActive() const { return m_Active; }
};

struct CGameClientStub
{
	CSounds m_Sounds;
	CScoreboardStub m_Scoreboard;
};

class CVoting
{
public:
	CHeap m_Heap;
	int64_t m_Opentime;
	int64_t m_Closetime;
	char m_aDescription[VOTE_DESC_LENGTH];
	char m_aReason[VOTE_REASON_LENGTH];
	int m_Voted;
	int m_Yes, m_No, m_Pass, m_Total;
	bool m_ReceivingOptions;
	int m_NumVoteOptions;
	CVoteOptionClient *m_pFirst;
	CVoteOptionClient *m_pLast;
	CVoteOptionClient *m_pRecycleFirst;
	CVoteOptionClient *m_pRecycleLast;
	CClientStub m_Client;
	CGameClientStub m_GameClient;

	CClientStub *Client() { return &m_Client; }
	CGameClientStub *GameClient() { return &m_GameClient; }
	int64_t time() const { return s_Now; }
	void AddOption(const char *pDescription);
	void RemoveOption(const char *pDescription);
	void ClearOptions();
	void OnReset();
	void OnMessage(int MsgType, void *pRawMsg);
	int SecondsLeft() const;
// C++ copy: game/client/components/voting.h:60-61
	bool IsVoting() const { return m_Closetime != 0; }
	int TakenChoice() const { return m_Voted; }
// end of copy
	// the first lines of Render: true when it got past them
	bool RenderHead();
};

// C++ copy: game/client/components/voting.cpp:150-153
int CVoting::SecondsLeft() const
{
	return (m_Closetime - time()) / time_freq();
}
// end of copy

bool CVoting::RenderHead()
{
	bool Shown = false;
	[&]() {
// C++ copy: game/client/components/voting.cpp:340-347
	if((!g_Config.m_ClShowVotesAfterVoting && !GameClient()->m_Scoreboard.IsActive() && TakenChoice()) || !IsVoting())
		return;
	const int Seconds = SecondsLeft();
	if(Seconds < 0)
	{
		OnReset();
		return;
	}
// end of copy
		Shown = true;
	}();
	return Shown;
}

// C++ copy: game/client/components/voting.cpp:161-189
void CVoting::AddOption(const char *pDescription)
{
	if(m_NumVoteOptions == MAX_VOTE_OPTIONS)
		return;

	CVoteOptionClient *pOption;
	if(m_pRecycleFirst)
	{
		pOption = m_pRecycleFirst;
		m_pRecycleFirst = m_pRecycleFirst->m_pNext;
		if(m_pRecycleFirst)
			m_pRecycleFirst->m_pPrev = nullptr;
		else
			m_pRecycleLast = nullptr;
	}
	else
		pOption = m_Heap.Allocate<CVoteOptionClient>();

	pOption->m_pNext = nullptr;
	pOption->m_pPrev = m_pLast;
	if(pOption->m_pPrev)
		pOption->m_pPrev->m_pNext = pOption;
	m_pLast = pOption;
	if(!m_pFirst)
		m_pFirst = pOption;

	str_copy(pOption->m_aDescription, pDescription);
	++m_NumVoteOptions;
}
// end of copy

// C++ copy: game/client/components/voting.cpp:191-220
void CVoting::RemoveOption(const char *pDescription)
{
	for(CVoteOptionClient *pOption = m_pFirst; pOption; pOption = pOption->m_pNext)
	{
		if(str_comp(pOption->m_aDescription, pDescription) == 0)
		{
			// remove it from the list
			if(m_pFirst == pOption)
				m_pFirst = m_pFirst->m_pNext;
			if(m_pLast == pOption)
				m_pLast = m_pLast->m_pPrev;
			if(pOption->m_pPrev)
				pOption->m_pPrev->m_pNext = pOption->m_pNext;
			if(pOption->m_pNext)
				pOption->m_pNext->m_pPrev = pOption->m_pPrev;
			--m_NumVoteOptions;

			// add it to recycle list
			pOption->m_pNext = nullptr;
			pOption->m_pPrev = m_pRecycleLast;
			if(pOption->m_pPrev)
				pOption->m_pPrev->m_pNext = pOption;
			m_pRecycleLast = pOption;
			if(!m_pRecycleFirst)
				m_pRecycleFirst = pOption;

			break;
		}
	}
}
// end of copy

// C++ copy: game/client/components/voting.cpp:222-232
void CVoting::ClearOptions()
{
	m_Heap.Reset();

	m_NumVoteOptions = 0;
	m_pFirst = nullptr;
	m_pLast = nullptr;

	m_pRecycleFirst = nullptr;
	m_pRecycleLast = nullptr;
}
// end of copy

// C++ copy: game/client/components/voting.cpp:234-242
void CVoting::OnReset()
{
	m_Opentime = m_Closetime = 0;
	m_aDescription[0] = '\0';
	m_aReason[0] = '\0';
	m_Yes = m_No = m_Pass = m_Total = 0;
	m_Voted = 0;
	m_ReceivingOptions = false;
}
// end of copy

// C++ copy: game/client/components/voting.cpp:250-336
void CVoting::OnMessage(int MsgType, void *pRawMsg)
{
	if(Client()->State() == IClient::STATE_DEMOPLAYBACK)
		return;

	if(MsgType == NETMSGTYPE_SV_VOTESET)
	{
		CNetMsg_Sv_VoteSet *pMsg = (CNetMsg_Sv_VoteSet *)pRawMsg;
		OnReset();
		if(pMsg->m_Timeout)
		{
			str_copy(m_aDescription, pMsg->m_pDescription);
			str_copy(m_aReason, pMsg->m_pReason);
			m_Opentime = time();
			m_Closetime = time() + time_freq() * pMsg->m_Timeout;

			if(Client()->RconAuthed())
			{
				char aBuf[512];
				str_format(aBuf, sizeof(aBuf), "%s (%s)", m_aDescription, m_aReason);
				Client()->Notify("DDNet Vote", aBuf);
				GameClient()->m_Sounds.Play(CSounds::CHN_GUI, SOUND_CHAT_HIGHLIGHT, 1.0f);
			}
		}
	}
	else if(MsgType == NETMSGTYPE_SV_VOTESTATUS)
	{
		CNetMsg_Sv_VoteStatus *pMsg = (CNetMsg_Sv_VoteStatus *)pRawMsg;
		m_Yes = pMsg->m_Yes;
		m_No = pMsg->m_No;
		m_Pass = pMsg->m_Pass;
		m_Total = pMsg->m_Total;
	}
	else if(MsgType == NETMSGTYPE_SV_VOTECLEAROPTIONS)
	{
		ClearOptions();
	}
	else if(MsgType == NETMSGTYPE_SV_VOTEOPTIONLISTADD)
	{
		CNetMsg_Sv_VoteOptionListAdd *pMsg = (CNetMsg_Sv_VoteOptionListAdd *)pRawMsg;
		int NumOptions = pMsg->m_NumOptions;
		for(int i = 0; i < NumOptions; ++i)
		{
			switch(i)
			{
			case 0: AddOption(pMsg->m_pDescription0); break;
			case 1: AddOption(pMsg->m_pDescription1); break;
			case 2: AddOption(pMsg->m_pDescription2); break;
			case 3: AddOption(pMsg->m_pDescription3); break;
			case 4: AddOption(pMsg->m_pDescription4); break;
			case 5: AddOption(pMsg->m_pDescription5); break;
			case 6: AddOption(pMsg->m_pDescription6); break;
			case 7: AddOption(pMsg->m_pDescription7); break;
			case 8: AddOption(pMsg->m_pDescription8); break;
			case 9: AddOption(pMsg->m_pDescription9); break;
			case 10: AddOption(pMsg->m_pDescription10); break;
			case 11: AddOption(pMsg->m_pDescription11); break;
			case 12: AddOption(pMsg->m_pDescription12); break;
			case 13: AddOption(pMsg->m_pDescription13); break;
			case 14: AddOption(pMsg->m_pDescription14);
			}
		}
	}
	else if(MsgType == NETMSGTYPE_SV_VOTEOPTIONADD)
	{
		CNetMsg_Sv_VoteOptionAdd *pMsg = (CNetMsg_Sv_VoteOptionAdd *)pRawMsg;
		AddOption(pMsg->m_pDescription);
	}
	else if(MsgType == NETMSGTYPE_SV_VOTEOPTIONREMOVE)
	{
		CNetMsg_Sv_VoteOptionRemove *pMsg = (CNetMsg_Sv_VoteOptionRemove *)pRawMsg;
		RemoveOption(pMsg->m_pDescription);
	}
	else if(MsgType == NETMSGTYPE_SV_YOURVOTE)
	{
		CNetMsg_Sv_YourVote *pMsg = (CNetMsg_Sv_YourVote *)pRawMsg;
		m_Voted = pMsg->m_Voted;
	}
	else if(MsgType == NETMSGTYPE_SV_VOTEOPTIONGROUPSTART)
	{
		m_ReceivingOptions = true;
	}
	else if(MsgType == NETMSGTYPE_SV_VOTEOPTIONGROUPEND)
	{
		m_ReceivingOptions = false;
	}
}
// end of copy

static unsigned s_Seed = 1717;
static unsigned Rand()
{
	s_Seed = s_Seed * 1103515245u + 12345u;
	return s_Seed >> 8;
}

// a random string: from a small set so that removals hit, sometimes with spaces,
// multi-byte characters or over 63 bytes
static std::string RandomString()
{
	static const char *s_apWords[] = {"map", "Gold Mine", "kick", "pause", "Ünïcödé", "雪", "x"};
	std::string s = s_apWords[Rand() % 7];
	if(Rand() % 4 == 0)
		s += std::to_string(Rand() % 3);
	if(Rand() % 10 == 0)
		while(s.size() < 70)
			s += s_apWords[Rand() % 7];
	return s;
}

static void Hex(FILE *f, const char *p)
{
	fprintf(f, " ");
	if(!*p)
		fprintf(f, "-");
	for(; *p; p++)
		fprintf(f, "%02x", (unsigned char)*p);
}

} // namespace votes_copy

int VotesMode(const char *pOutFile)
{
	using namespace votes_copy;
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;
	static CVoting s_Voting;
	CVoting &v = s_Voting;
	v.ClearOptions();
	v.OnReset();
	for(int Step = 0; Step < 6000; Step++)
	{
		s_Now += Rand() % 1000000;
		std::string aStr[15];
		for(auto &s : aStr)
			s = RandomString();
		const int Kind = Rand() % 10;
		fprintf(f, "%lld", (long long)s_Now);
		switch(Kind)
		{
		case 0:
		{
			CNetMsg_Sv_VoteSet Msg;
			Msg.m_Timeout = Rand() % 3 == 0 ? 0 : (int)(Rand() % 60);
			Msg.m_pDescription = aStr[0].c_str();
			Msg.m_pReason = aStr[1].c_str();
			fprintf(f, " set %d", Msg.m_Timeout);
			Hex(f, Msg.m_pDescription);
			Hex(f, Msg.m_pReason);
			v.OnMessage(NETMSGTYPE_SV_VOTESET, &Msg);
			break;
		}
		case 1:
		{
			CNetMsg_Sv_VoteStatus Msg;
			Msg.m_Yes = Rand() % 64;
			Msg.m_No = Rand() % 64;
			Msg.m_Pass = Rand() % 64;
			Msg.m_Total = Rand() % 64;
			fprintf(f, " status %d %d %d %d", Msg.m_Yes, Msg.m_No, Msg.m_Pass, Msg.m_Total);
			v.OnMessage(NETMSGTYPE_SV_VOTESTATUS, &Msg);
			break;
		}
		case 2:
		{
			if(Rand() % 2 != 0)
			{
				fprintf(f, " none");
				break;
			}
			CNetMsg_Sv_VoteClearOptions Msg;
			fprintf(f, " clear");
			v.OnMessage(NETMSGTYPE_SV_VOTECLEAROPTIONS, &Msg);
			break;
		}
		case 3:
		{
			CNetMsg_Sv_VoteOptionListAdd Msg;
			Msg.m_NumOptions = 1 + Rand() % 15;
			const char **apDesc[] = {&Msg.m_pDescription0, &Msg.m_pDescription1, &Msg.m_pDescription2, &Msg.m_pDescription3,
				&Msg.m_pDescription4, &Msg.m_pDescription5, &Msg.m_pDescription6, &Msg.m_pDescription7, &Msg.m_pDescription8,
				&Msg.m_pDescription9, &Msg.m_pDescription10, &Msg.m_pDescription11, &Msg.m_pDescription12,
				&Msg.m_pDescription13, &Msg.m_pDescription14};
			fprintf(f, " listadd %d", Msg.m_NumOptions);
			for(int i = 0; i < 15; i++)
			{
				*apDesc[i] = aStr[i].c_str();
				Hex(f, aStr[i].c_str());
			}
			v.OnMessage(NETMSGTYPE_SV_VOTEOPTIONLISTADD, &Msg);
			break;
		}
		case 4:
		case 5:
		{
			CNetMsg_Sv_VoteOptionAdd Msg;
			Msg.m_pDescription = aStr[0].c_str();
			fprintf(f, " add");
			Hex(f, Msg.m_pDescription);
			v.OnMessage(NETMSGTYPE_SV_VOTEOPTIONADD, &Msg);
			break;
		}
		case 6:
		case 7:
		{
			CNetMsg_Sv_VoteOptionRemove Msg;
			Msg.m_pDescription = aStr[0].c_str();
			fprintf(f, " remove");
			Hex(f, Msg.m_pDescription);
			v.OnMessage(NETMSGTYPE_SV_VOTEOPTIONREMOVE, &Msg);
			break;
		}
		case 8:
		{
			CNetMsg_Sv_YourVote Msg;
			Msg.m_Voted = (int)(Rand() % 3) - 1;
			fprintf(f, " yourvote %d", Msg.m_Voted);
			v.OnMessage(NETMSGTYPE_SV_YOURVOTE, &Msg);
			break;
		}
		default:
		{
			const bool Start = Rand() % 2;
			fprintf(f, Start ? " groupstart" : " groupend");
			if(Start)
			{
				CNetMsg_Sv_VoteOptionGroupStart Msg;
				v.OnMessage(NETMSGTYPE_SV_VOTEOPTIONGROUPSTART, &Msg);
			}
			else
			{
				CNetMsg_Sv_VoteOptionGroupEnd Msg;
				v.OnMessage(NETMSGTYPE_SV_VOTEOPTIONGROUPEND, &Msg);
			}
		}
		}
		// CVoting::Render's show rule now and then, sometimes after the close time
		if(Rand() % 3 == 0)
		{
			if(Rand() % 4 == 0 && v.m_Closetime)
				s_Now = v.m_Closetime + (int64_t)(Rand() % 3) * 700000000 - 700000000;
			g_Config.m_ClShowVotesAfterVoting = Rand() % 2;
			v.m_GameClient.m_Scoreboard.m_Active = Rand() % 2;
			const bool Shown = v.RenderHead();
			fprintf(f, " render %lld %d %d %d", (long long)s_Now, g_Config.m_ClShowVotesAfterVoting, v.m_GameClient.m_Scoreboard.m_Active, Shown);
		}
		fprintf(f, " = %lld %lld %d %d %d %d %d %d %d", (long long)v.m_Opentime, (long long)v.m_Closetime, v.m_Voted, v.m_Yes, v.m_No,
			v.m_Pass, v.m_Total, v.m_ReceivingOptions, v.m_NumVoteOptions);
		Hex(f, v.m_aDescription);
		Hex(f, v.m_aReason);
		for(const CVoteOptionClient *p = v.m_pFirst; p; p = p->m_pNext)
			Hex(f, p->m_aDescription);
		fprintf(f, "\n");
	}
	fclose(f);
	return 0;
}
