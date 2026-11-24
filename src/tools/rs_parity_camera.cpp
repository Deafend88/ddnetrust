// "camera" mode of rs_parity: runs the real CControls and CCamera code (mouse
// scaling and clamping, zoom steps and smoothing, dyncam offsets, the camera center),
// and packs Cl_ShowDistance and Cl_CameraInfo the way CGameClient::OnNewSnapshot fills
// them, and writes inputs and results for rust-client/client/tests/input.rs.
//
// The components normally live inside a CGameClient. Here they are constructed in a
// zeroed block of CGameClient-sized memory (only the members they read are set: the
// IClient times and state, m_Snap, m_GameInfo, m_LocalCharacterPos), so their member
// functions run unchanged. Only the playing (not spectating) paths are exercised.

#include <base/math.h>
#include <base/mem.h>
#include <base/str.h>

#include <engine/client.h>
#include <engine/graphics.h>
#include <engine/message.h>
#include <engine/shared/config.h>

#include <game/client/components/camera.h>
#include <game/client/components/controls.h>
#include <game/client/gameclient.h>

#include <cstdio>
#include <cstring>
#include <new>

// Access to private members: explicit template instantiations may name them.
template<typename Tag, typename Tag::type M>
struct SRob
{
	friend typename Tag::type Get(Tag) { return M; }
};
struct SClientPtr
{
	typedef IClient *CGameClient::*type;
	friend type Get(SClientPtr);
};
template struct SRob<SClientPtr, &CGameClient::m_pClient>;
typedef void (*FCon)(IConsole::IResult *, void *);
struct SZoomPlus
{
	typedef FCon type;
	friend type Get(SZoomPlus);
};
template struct SRob<SZoomPlus, &CCamera::ConZoomPlus>;
struct SZoomMinus
{
	typedef FCon type;
	friend type Get(SZoomMinus);
};
template struct SRob<SZoomMinus, &CCamera::ConZoomMinus>;
struct SZoom
{
	typedef FCon type;
	friend type Get(SZoom);
};
template struct SRob<SZoom, &CCamera::ConZoom>;
struct SSpeedBias
{
	typedef float CCamera::*type;
	friend type Get(SSpeedBias);
};
template struct SRob<SSpeedBias, &CCamera::m_DyncamSmoothingSpeedBias>;

// IClient's protected time and state fields, named through a derived class.
struct SClientFields : IClient
{
	static float IClient::*LocalTime() { return &SClientFields::m_LocalTime; }
	static float IClient::*FrameTime() { return &SClientFields::m_RenderFrameTime; }
	static EClientState IClient::*State() { return &SClientFields::m_State; }
};

// Not reached on the paths exercised here (no world pause; SnapInput is not called).
bool CGameClient::IsWorldPaused() const { return false; }
bool CScoreboard::IsActive() const { return false; }

// a console result with at most one float argument
class CFloatResult : public IConsole::IResult
{
	float m_Value;

public:
	CFloatResult(bool HasArg, float Value) :
		IResult(-1), m_Value(Value) { m_NumArgs = HasArg; }
	int GetInteger(unsigned Index) const override { return (int)m_Value; }
	float GetFloat(unsigned Index) const override { return m_Value; }
	const char *GetString(unsigned Index) const override { return ""; }
	ColorHSLA GetColor(unsigned Index, float DarkestLighting) const override { return ColorHSLA(0, 0, 0); }
	void RemoveArgument(unsigned Index) override {}
	int GetVictim(unsigned Slot) const override { return -1; }
};

static unsigned s_CamSeed = 777;
static unsigned CamRand()
{
	s_CamSeed = s_CamSeed * 1103515245u + 12345u;
	return s_CamSeed >> 8;
}

static unsigned Bits(float f)
{
	unsigned u;
	mem_copy(&u, &f, sizeof(u));
	return u;
}

static int Pick(int Default, int Max)
{
	return CamRand() % 3 == 0 ? (int)(CamRand() % (Max + 1)) : Default;
}

// the camera and mouse settings, in the order written to the "config" lines
static int *s_apSettings[] = {
	&g_Config.m_ClDyncam, &g_Config.m_ClDyncamMaxDistance, &g_Config.m_ClDyncamMinDistance,
	&g_Config.m_ClDyncamMousesens, &g_Config.m_ClDyncamDeadzone, &g_Config.m_ClDyncamFollowFactor,
	&g_Config.m_ClDyncamSmoothness, &g_Config.m_ClDyncamStabilizing, &g_Config.m_ClMouseDeadzone,
	&g_Config.m_ClMouseFollowfactor, &g_Config.m_ClMouseMaxDistance, &g_Config.m_ClMouseMinDistance,
	&g_Config.m_InpMousesens, &g_Config.m_ClDefaultZoom, &g_Config.m_ClSmoothZoomTime};

static void RandomSettings(FILE *f)
{
	g_Config.m_ClDyncam = CamRand() % 2;
	g_Config.m_ClDyncamMaxDistance = Pick(1000, 2000);
	g_Config.m_ClDyncamMinDistance = Pick(0, 2000);
	g_Config.m_ClDyncamMousesens = CamRand() % 2 ? 0 : Pick(0, 1000);
	g_Config.m_ClDyncamDeadzone = 1 + Pick(299, 1299);
	g_Config.m_ClDyncamFollowFactor = Pick(60, 200);
	g_Config.m_ClDyncamSmoothness = Pick(0, 100);
	g_Config.m_ClDyncamStabilizing = Pick(0, 100);
	g_Config.m_ClMouseDeadzone = Pick(0, 3000);
	g_Config.m_ClMouseFollowfactor = Pick(0, 200);
	g_Config.m_ClMouseMaxDistance = Pick(400, 5000);
	g_Config.m_ClMouseMinDistance = Pick(0, 5000);
	g_Config.m_InpMousesens = 1 + Pick(199, 1000);
	g_Config.m_ClDefaultZoom = Pick(10, 20);
	g_Config.m_ClSmoothZoomTime = Pick(250, 1000);
	fprintf(f, "config");
	for(int *pValue : s_apSettings)
		fprintf(f, " %d", *pValue);
	fprintf(f, "\n");
}

int CameraMode(const char *pOutFile)
{
	FILE *f = fopen(pOutFile, "wb");
	if(!f)
		return -1;

	alignas(CGameClient) static unsigned char s_aGameClient[sizeof(CGameClient)];
	alignas(IClient) static unsigned char s_aClient[sizeof(IClient)];
	CGameClient *pGameClient = reinterpret_cast<CGameClient *>(s_aGameClient);
	IClient *pClient = reinterpret_cast<IClient *>(s_aClient);
	pGameClient->*Get(SClientPtr()) = pClient;
	pClient->*SClientFields::State() = IClient::STATE_ONLINE;
	CControls *pControls = new(&pGameClient->m_Controls) CControls();
	CCamera *pCamera = new(&pGameClient->m_Camera) CCamera();
	pControls->OnInterfacesInit(pGameClient);
	pCamera->OnInterfacesInit(pGameClient);
	g_Config.m_ClLimitMaxZoomLevel = 0; // 1 asks IGraphics about tile buffering
	g_Config.m_ClDummy = 0;

	// the camera messages, filled like CGameClient::OnNewSnapshot (gameclient.cpp:2361)
	alignas(IGraphics) static unsigned char s_aNoGraphics[sizeof(IGraphics)] = {};
	const IGraphics *pGraphics = reinterpret_cast<const IGraphics *>(s_aNoGraphics); // CalcScreenParams only reads its arguments
	auto Hex = [](const CMsgPacker &Packer) {
		static char s_aBuf[256];
		str_hex(s_aBuf, sizeof(s_aBuf), Packer.Data(), Packer.Size());
		return s_aBuf;
	};
	for(int i = 0; i < 2000; i++)
	{
		const float Aspect = (float)(1 + CamRand() % 4000) / (float)(1 + CamRand() % 4000);
		const float Zoom = CamRand() % 4 == 0 ? CCamera::ZoomStepsToValue((int)(CamRand() % 41) - 20) : (1 + CamRand() % 300000) / 1000.0f;
		CNetMsg_Cl_ShowDistance Distance;
		float x, y;
		pGraphics->CalcScreenParams(Aspect, Zoom, &x, &y);
		Distance.m_X = x;
		Distance.m_Y = y;
		CMsgPacker DistancePacker(&Distance);
		Distance.Pack(&DistancePacker);
		const float Deadzone = (int)(CamRand() % 3001);
		const float FollowFactor = (int)(CamRand() % 201);
		CNetMsg_Cl_CameraInfo Info;
		Info.m_Zoom = round_truncate(Zoom * 1000.f);
		Info.m_Deadzone = Deadzone;
		Info.m_FollowFactor = FollowFactor;
		CMsgPacker InfoPacker(&Info);
		Info.Pack(&InfoPacker);
		fprintf(f, "msg %08x %08x %d %d | %d %s", Bits(Aspect), Bits(Zoom), (int)Deadzone, (int)FollowFactor, DistancePacker.m_MsgId, Hex(DistancePacker));
		fprintf(f, " %d %s\n", InfoPacker.m_MsgId, Hex(InfoPacker));
	}

	// mouse: OnCursorMove scaling and ClampMousePos with random settings
	for(int i = 0; i < 5000; i++)
	{
		RandomSettings(f);
		vec2 Pos;
		switch(CamRand() % 4)
		{
		case 0: Pos = vec2(0, 0); break;
		case 1: Pos = vec2((int)(CamRand() % 5) - 2, (int)(CamRand() % 5) - 2) * 0.0006f; break;
		default: Pos = vec2((int)(CamRand() % 12001) - 6000, (int)(CamRand() % 12001) - 6000) / 3.0f;
		}
		const int Dx = (int)(CamRand() % 201) - 100, Dy = CamRand() % 4 ? (int)(CamRand() % 201) - 100 : 0;
		pControls->m_aMousePos[0] = Pos;
		pControls->ClampMousePos();
		const vec2 Clamped = pControls->m_aMousePos[0];
		pControls->m_aMousePos[0] = Pos;
		pControls->OnCursorMove(Dx, Dy, IInput::CURSOR_MOUSE);
		const vec2 Moved = pControls->m_aMousePos[0];
		fprintf(f, "mouse %08x %08x %d %d %08x %08x %08x %08x %08x %08x\n", Bits(Pos.x), Bits(Pos.y), Dx, Dy,
			Bits(pControls->GetMinMouseDistance()), Bits(pControls->GetMaxMouseDistance()),
			Bits(Clamped.x), Bits(Clamped.y), Bits(Moved.x), Bits(Moved.y));
	}

	// camera: frames of UpdateCamera + OnRender with mouse moves, zoom commands and a
	// moving tee, as CGameClient::OnUpdate/OnRender run them
	for(int s = 0; s < 400; s++)
	{
		RandomSettings(f);
		// CCamera() leaves m_ZoomSmoothingTarget and the zoom smoothing times unset
		// (they are only read while zooming); zero them so the output is deterministic
		pCamera->~CCamera();
		mem_zero(static_cast<void *>(pCamera), sizeof(CCamera));
		new(pCamera) CCamera();
		pCamera->OnInterfacesInit(pGameClient);
		pControls->m_aMousePos[0] = vec2(0, 0);
		pGameClient->m_GameInfo.m_AllowZoom = CamRand() % 8 != 0;
		pGameClient->m_LocalCharacterPos = vec2(CamRand() % 64000, CamRand() % 64000) / 10.0f;
		pCamera->OnReset();
		float Time = (CamRand() % 100000) / 7.0f;
		for(int Frame = 0; Frame < 80; Frame++)
		{
			const float FrameTime = CamRand() % 10 == 0 ? 0.0001f : (1 + CamRand() % 40) / 1000.0f;
			Time += FrameTime;
			pClient->*SClientFields::LocalTime() = Time;
			pClient->*SClientFields::FrameTime() = FrameTime;
			const vec2 Step = vec2((int)(CamRand() % 41) - 20, (int)(CamRand() % 41) - 20) / 4.0f;
			pGameClient->m_LocalCharacterPos += Step;
			// one input event before the frame
			// 0-4 mouse move, 5 zoom+, 6 zoom-, 7 zoom, 8 toggle AllowZoom, else nothing
			int Action = CamRand() % 12;
			if((Action == 7 && CamRand() % 3 != 0) || (Action == 8 && CamRand() % 20 != 0))
				Action = 11;
			const bool HasArg = CamRand() % 3 == 0;
			const float Arg = (int)(CamRand() % 81 - 40) / 8.0f;
			int Dx = 0, Dy = 0;
			if(Action <= 4)
			{
				Dx = (int)(CamRand() % 121) - 60;
				Dy = (int)(CamRand() % 121) - 60;
				pControls->OnCursorMove(Dx, Dy, IInput::CURSOR_MOUSE);
			}
			else if(Action == 5)
			{
				CFloatResult Result(HasArg, Arg);
				Get(SZoomPlus())(&Result, pCamera);
			}
			else if(Action == 6)
			{
				CFloatResult Result(HasArg, Arg);
				Get(SZoomMinus())(&Result, pCamera);
			}
			else if(Action == 7)
			{
				CFloatResult Result(HasArg, Arg + 10.0f);
				Get(SZoom())(&Result, pCamera);
			}
			else if(Action == 8)
				pGameClient->m_GameInfo.m_AllowZoom = !pGameClient->m_GameInfo.m_AllowZoom;
			pCamera->UpdateCamera();
			pCamera->OnRender();
			fprintf(f, "frame %08x %08x %08x %08x %d %d %d %d %d %08x | %08x %d %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x %d\n",
				Bits(Time), Bits(FrameTime), Bits(pGameClient->m_LocalCharacterPos.x), Bits(pGameClient->m_LocalCharacterPos.y),
				Action, Dx, Dy, HasArg, pGameClient->m_GameInfo.m_AllowZoom, Bits(Arg),
				Bits(pCamera->m_Zoom), pCamera->m_Zooming, Bits(pCamera->m_ZoomSmoothingTarget), Bits(pCamera->m_UserZoomTarget),
				Bits(pCamera->*Get(SSpeedBias())), Bits(pCamera->m_DyncamTargetCameraOffset.x), Bits(pCamera->m_DyncamTargetCameraOffset.y),
				Bits(pCamera->m_aDyncamCurrentCameraOffset[0].x), Bits(pCamera->m_aDyncamCurrentCameraOffset[0].y),
				Bits(pCamera->m_Center.x), Bits(pCamera->m_Center.y), Bits(pControls->m_aMousePos[0].x), Bits(pControls->m_aMousePos[0].y),
				(int)pControls->m_aMouseInputType[0]);
		}
	}
	fclose(f);
	return 0;
}
