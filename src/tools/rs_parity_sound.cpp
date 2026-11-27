// "wv" and "opus" modes of rs_parity: the sound decoders (PLAN §13.3), for
// rust-client/client/tests/sound.rs.
//
// CSound (engine/client/sound.cpp) needs SDL and the graphics engine, so its decoder is
// copied here word for word into a stand-in class of the same name (in its own
// namespace): CWavpackReader, DecodeWV and DecodeOpus. Each copy is marked "C++ copy: <file>:<lines>"
// ... "end of copy"; the Rust test `cpp_copies_match_source` checks that every copy is
// still identical (whitespace aside) to those source lines. What the copies call is the
// real code: libwavpack and opusfile from ddnet-libs.
//
// Every .wv file of the given directories is decoded as it is and in 4 damaged variants
// (cut short, one bit flipped, 8 random bytes, one bit flipped in the first 64 bytes);
// the dump has the damage, the result and an FNV-1a hash of the 16-bit samples.
// "opus": every .opus file of a directory (map sounds, see extract_map_sounds.py) is
// decoded; the result goes to <out dir>/opus.txt and the samples to <out dir>/<name>.pcm.
// "mixer": the engine itself (rs_mixer namespace): copies of CSample/CChannel/CVoice,
// AdvanceVoice, Mix, AllocSample, RateConvert and UnloadSample through IsPlaying, in a
// stand-in CSound without the device. Real samples (DecodeWV) are loaded at two mixing
// rates and driven by random calls (play with every flag, the voice setters on live,
// stale and invalid handles, channels, listener, stop/pause, unload/reload, sample
// times, master volume) and mixes of random lengths; every call, result and mix hash
// is dumped for the Rust replay.

#include <base/dbg.h>
#include <base/lock.h>
#include <base/log.h>
#include <base/math.h>
#include <base/vmath.h>

#include <engine/sound.h>
#include <base/mem.h>
#include <base/str.h>

#include <algorithm>
#include <atomic>
#include <cinttypes>
#include <cmath>
#include <cstdio> // SEEK_*
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <limits>
#include <string>
#include <vector>

extern "C" {
#include <opusfile.h>
#include <wavpack.h>
}

namespace rs_sound {

struct CSample
{
	short *m_pData = nullptr;
	int m_NumFrames = 0;
	int m_Rate = 0;
	int m_Channels = 0;
	int m_LoopStart = 0;
	int m_PausedAt = 0;
};

class CSound
{
	bool m_SoundEnabled = true;

public:
	bool DecodeWV(CSample &Sample, const void *pData, unsigned DataSize, const char *pContextName) const;
	bool DecodeOpus(CSample &Sample, const void *pData, unsigned DataSize, const char *pContextName) const;
};

// C++ copy: engine/client/sound.cpp:464-546
bool CSound::DecodeOpus(CSample &Sample, const void *pData, unsigned DataSize, const char *pContextName) const
{
	int OpusError = 0;
	OggOpusFile *pOpusFile = op_open_memory((const unsigned char *)pData, DataSize, &OpusError);
	if(pOpusFile)
	{
		const int NumChannels = op_channel_count(pOpusFile, -1);
		if(NumChannels > 2)
		{
			op_free(pOpusFile);
			log_error("sound/opus", "File is not mono or stereo. Filename='%s'", pContextName);
			return false;
		}

		const int NumSamples = op_pcm_total(pOpusFile, -1); // per channel!
		if(NumSamples < 0)
		{
			op_free(pOpusFile);
			log_error("sound/opus", "Failed to get number of samples, error %d. Filename='%s'", NumSamples, pContextName);
			return false;
		}

		short *pSampleData = (short *)calloc((size_t)NumSamples * NumChannels, sizeof(short));

		int Pos = 0;
		while(Pos < NumSamples)
		{
			const int Read = op_read(pOpusFile, pSampleData + Pos * NumChannels, (NumSamples - Pos) * NumChannels, nullptr);
			if(Read < 0)
			{
				free(pSampleData);
				op_free(pOpusFile);
				log_error("sound/opus", "op_read error %d at %d. Filename='%s'", Read, Pos, pContextName);
				return false;
			}
			else if(Read == 0) // EOF
				break;
			Pos += Read;
		}

		Sample.m_pData = pSampleData;
		Sample.m_NumFrames = Pos;
		Sample.m_Rate = 48000;
		Sample.m_Channels = NumChannels;
		Sample.m_LoopStart = 0;
		Sample.m_PausedAt = 0;

		const OpusTags *pTags = op_tags(pOpusFile, -1);
		if(pTags)
		{
			for(int i = 0; i < pTags->comments; ++i)
			{
				const char *pComment = pTags->user_comments[i];
				if(!pComment)
					continue;
				if(!str_startswith(pComment, "LOOP_START="))
					continue;
				int LoopStart = -1;
				if(!str_toint(pComment + str_length("LOOP_START="), &LoopStart))
				{
					log_error("sound/opus", "Invalid LOOP_START tag. Value='%s' Filename='%s'", pComment + str_length("LOOP_START="), pContextName);
					break;
				}
				if(LoopStart < 0 || LoopStart >= Sample.m_NumFrames)
				{
					log_error("sound/opus", "Tag LOOP_START out of range. Value=%d Min=0 Max=%d Filename='%s'", LoopStart, Sample.m_NumFrames - 1, pContextName);
					break;
				}
				Sample.m_LoopStart = LoopStart;
				break;
			}
		}

		op_free(pOpusFile);
	}
	else
	{
		log_error("sound/opus", "Failed to decode sample, error %d. Filename='%s'", OpusError, pContextName);
		return false;
	}

	return true;
}
// end of copy

// C++ copy: engine/client/sound.cpp:549-610
class CWavpackReader
{
	const char *m_pBuffer;
	int m_Size;
	int m_Position = 0;

public:
	CWavpackReader(const void *pBuffer, int Size) :
		m_pBuffer(static_cast<const char *>(pBuffer)), m_Size(Size)
	{
	}

	static int ReadData(void *pId, void *pBuffer, int Size)
	{
		CWavpackReader *pReader = static_cast<CWavpackReader *>(pId);
		int ChunkSize = std::min(Size, pReader->m_Size - pReader->m_Position);
		mem_copy(pBuffer, pReader->m_pBuffer + pReader->m_Position, ChunkSize);
		pReader->m_Position += ChunkSize;
		return ChunkSize;
	}

	static int CanSeek(void *pId)
	{
		return 1;
	}

	static int SetPosAbs(void *pId, uint32_t Position)
	{
		CWavpackReader *pReader = static_cast<CWavpackReader *>(pId);
		if(Position > (uint32_t)pReader->m_Size)
			return -1;
		pReader->m_Position = Position;
		return 0;
	}

	static int SetPosRel(void *pId, int32_t Delta, int Mode)
	{
		CWavpackReader *pReader = static_cast<CWavpackReader *>(pId);
		const int64_t Base = Mode == SEEK_SET ? 0 : (Mode == SEEK_CUR ? pReader->m_Position : pReader->m_Size);
		const int64_t Position = Base + Delta;
		if(Position < 0 || Position > pReader->m_Size)
			return -1;
		pReader->m_Position = Position;
		return 0;
	}

	static unsigned int GetPos(void *pId)
	{
		return static_cast<CWavpackReader *>(pId)->m_Position;
	}

	static unsigned int GetLength(void *pId)
	{
		return static_cast<CWavpackReader *>(pId)->m_Size;
	}

	static int PushBackByte(void *pId, int Char)
	{
		static_cast<CWavpackReader *>(pId)->m_Position -= 1;
		return 0;
	}
};
// end of copy

// C++ copy: engine/client/sound.cpp:612-691
bool CSound::DecodeWV(CSample &Sample, const void *pData, unsigned DataSize, const char *pContextName) const
{
	// no need to load sound when we are running with no sound
	if(!m_SoundEnabled)
		return false;

	char aError[100];

	CWavpackReader Reader(pData, DataSize);
	WavpackStreamReader Callback = {};
	Callback.can_seek = CWavpackReader::CanSeek;
	Callback.get_length = CWavpackReader::GetLength;
	Callback.get_pos = CWavpackReader::GetPos;
	Callback.push_back_byte = CWavpackReader::PushBackByte;
	Callback.read_bytes = CWavpackReader::ReadData;
	Callback.set_pos_abs = CWavpackReader::SetPosAbs;
	Callback.set_pos_rel = CWavpackReader::SetPosRel;
	WavpackContext *pContext = WavpackOpenFileInputEx(&Callback, &Reader, nullptr, aError, 0, 0);
	if(pContext)
	{
		const int BitsPerSample = WavpackGetBitsPerSample(pContext);
		const unsigned int SampleRate = WavpackGetSampleRate(pContext);
		const int NumChannels = WavpackGetNumChannels(pContext);

		if(NumChannels > 2)
		{
			log_error("sound/wv", "File is not mono or stereo. Filename='%s'", pContextName);
			WavpackCloseFile(pContext);
			return false;
		}

		if(BitsPerSample != 16)
		{
			log_error("sound/wv", "Bits per sample is %d, not 16. Filename='%s'", BitsPerSample, pContextName);
			WavpackCloseFile(pContext);
			return false;
		}

		// wavpack reports -1 when the length of the stream is unknown
		const int64_t NumSamples = WavpackGetNumSamples64(pContext);
		if(NumSamples <= 0 || NumSamples > std::numeric_limits<int>::max())
		{
			log_error("sound/wv", "Number of samples is %" PRId64 ", not supported. Filename='%s'", NumSamples, pContextName);
			WavpackCloseFile(pContext);
			return false;
		}

		int *pBuffer = (int *)calloc((size_t)NumSamples * NumChannels, sizeof(int));
		if(!WavpackUnpackSamples(pContext, pBuffer, NumSamples))
		{
			free(pBuffer);
			log_error("sound/wv", "WavpackUnpackSamples failed. NumSamples=%" PRId64 " NumChannels=%d Filename='%s'", NumSamples, NumChannels, pContextName);
			WavpackCloseFile(pContext);
			return false;
		}

		Sample.m_pData = (short *)calloc((size_t)NumSamples * NumChannels, sizeof(short));

		int *pSrc = pBuffer;
		short *pDst = Sample.m_pData;
		for(int64_t i = 0; i < NumSamples * NumChannels; i++)
			*pDst++ = (short)*pSrc++;

		free(pBuffer);
		WavpackCloseFile(pContext);

		Sample.m_NumFrames = NumSamples;
		Sample.m_Rate = SampleRate;
		Sample.m_Channels = NumChannels;
		Sample.m_LoopStart = 0;
		Sample.m_PausedAt = 0;
	}
	else
	{
		log_error("sound/wv", "Failed to decode sample (%s). Filename='%s'", aError, pContextName);
		return false;
	}

	return true;
}
// end of copy

static unsigned s_Seed = 977;
static unsigned Rand()
{
	s_Seed = s_Seed * 1103515245u + 12345u;
	return s_Seed >> 8;
}

static uint64_t Fnv(const short *pData, size_t Count)
{
	uint64_t h = 0xcbf29ce484222325ull;
	for(size_t i = 0; i < Count; i++)
	{
		const unsigned short v = (unsigned short)pData[i];
		h = (h ^ (v & 0xff)) * 0x100000001b3ull;
		h = (h ^ (v >> 8)) * 0x100000001b3ull;
	}
	return h;
}

} // namespace rs_sound

namespace rs_mixer {

static constexpr int SAMPLE_INDEX_USED = -2;
static constexpr int SAMPLE_INDEX_FULL = -1;

// C++ copy: engine/client/sound.h:15-61
struct CSample
{
	int m_Index;
	int m_NextFreeSampleIndex;

	short *m_pData;
	int m_NumFrames;
	int m_Rate;
	int m_Channels;
	int m_LoopStart;
	int m_PausedAt;

	float TotalTime() const
	{
		return m_NumFrames / (float)m_Rate;
	}

	bool IsLoaded() const
	{
		return m_pData != nullptr;
	}
};

struct CChannel
{
	int m_Vol;
	int m_Pan;
};

struct CVoice
{
	CSample *m_pSample;
	CChannel *m_pChannel;
	int m_Age; // increases when reused
	int m_Tick;
	int m_Vol; // 0 - 255
	int m_Flags;
	vec2 m_Position;
	float m_Falloff; // [0.0, 1.0]

	int m_Shape;
	union
	{
		ISound::CVoiceShapeCircle m_Circle;
		ISound::CVoiceShapeRectangle m_Rectangle;
	};
};
// end of copy

// CSound of engine/client/sound.h without SDL, the graphics engine and the storage
class CSound : public ISound
{
public:
	enum
	{
		NUM_SAMPLES = 512,
		NUM_VOICES = 256,
		NUM_CHANNELS = 16,
	};

	CLock m_SoundLock;
	CSample m_aSamples[NUM_SAMPLES] = {{0}};
	int m_FirstFreeSampleIndex = 0;
	CVoice m_aVoices[NUM_VOICES] = {{nullptr}};
	CChannel m_aChannels[NUM_CHANNELS] = {{255, 0}};
	int m_NextVoice = 0;
	uint32_t m_MaxFrames = 0;
	std::atomic<float> m_ListenerPositionX = 0.0f;
	std::atomic<float> m_ListenerPositionY = 0.0f;
	std::atomic<int> m_SoundVolume = 100;
	int m_MixingRate = 48000;
	int *m_pMixBuffer = nullptr;

	// the sample setup of CSound::Init, with the mixing rate SDL gave and snd_buffer_size
	void Init(int MixingRate, int BufferSize)
	{
// C++ copy: engine/client/sound.cpp:209-218
	const CLockScope LockScope(m_SoundLock);
	m_FirstFreeSampleIndex = 0;
	for(size_t i = 0; i < std::size(m_aSamples) - 1; ++i)
	{
		m_aSamples[i].m_Index = i;
		m_aSamples[i].m_NextFreeSampleIndex = i + 1;
		m_aSamples[i].m_pData = nullptr;
	}
	m_aSamples[std::size(m_aSamples) - 1].m_Index = std::size(m_aSamples) - 1;
	m_aSamples[std::size(m_aSamples) - 1].m_NextFreeSampleIndex = SAMPLE_INDEX_FULL;
// end of copy
		m_MixingRate = MixingRate;
		m_MaxFrames = BufferSize * 2;
		m_pMixBuffer = (int *)calloc(m_MaxFrames * 2, sizeof(int));
	}
	~CSound()
	{
		for(auto &Sample : m_aSamples)
			free(Sample.m_pData);
		free(m_pMixBuffer);
	}
	// LoadWVFromMem/LoadOpusFromMem around an already decoded sample
	int Load(const rs_sound::CSample &Decoded, int LoopStart)
	{
		CSample *pSample = AllocSample();
		if(!pSample)
			return -1;
		const size_t Count = (size_t)Decoded.m_NumFrames * Decoded.m_Channels;
		pSample->m_pData = (short *)malloc(Count * sizeof(short));
		mem_copy(pSample->m_pData, Decoded.m_pData, Count * sizeof(short));
		pSample->m_NumFrames = Decoded.m_NumFrames;
		pSample->m_Rate = Decoded.m_Rate;
		pSample->m_Channels = Decoded.m_Channels;
		pSample->m_LoopStart = LoopStart;
		pSample->m_PausedAt = 0;
		RateConvert(*pSample);
		return pSample->m_Index;
	}

	unsigned AdvanceVoice(CVoice &Voice, unsigned Frames);
	CSample *AllocSample();
	void RateConvert(CSample &Sample) const;

	bool IsSoundEnabled() override { return true; }
	int LoadOpus(const char *pFilename, int StorageType) override { return -1; }
	int LoadWV(const char *pFilename, int StorageType) override { return -1; }
	int LoadOpusFromMem(const void *pData, unsigned DataSize, bool ForceLoad, const char *pContextName) override { return -1; }
	int LoadWVFromMem(const void *pData, unsigned DataSize, bool ForceLoad, const char *pContextName) override { return -1; }
	void UnloadSample(int SampleId) override;
	float GetSampleTotalTime(int SampleId) override;
	float GetSampleCurrentTime(int SampleId) override;
	void SetSampleCurrentTime(int SampleId, float Time) override;
	void SetChannel(int ChannelId, float Vol, float Pan) override;
	void SetListenerPosition(vec2 Position) override;
	void SetVoiceVolume(CVoiceHandle Voice, float Volume) override;
	void SetVoiceFalloff(CVoiceHandle Voice, float Falloff) override;
	void SetVoicePosition(CVoiceHandle Voice, vec2 Position) override;
	void SetVoiceTimeOffset(CVoiceHandle Voice, float TimeOffset) override;
	void SetVoiceCircle(CVoiceHandle Voice, float Radius) override;
	void SetVoiceRectangle(CVoiceHandle Voice, float Width, float Height) override;
	CVoiceHandle Play(int ChannelId, int SampleId, int Flags, float Volume, vec2 Position);
	CVoiceHandle PlayAt(int ChannelId, int SampleId, int Flags, float Volume, vec2 Position) override;
	CVoiceHandle Play(int ChannelId, int SampleId, int Flags, float Volume) override;
	void Pause(int SampleId) override;
	void Stop(int SampleId) override;
	void StopAll() override;
	void StopVoice(CVoiceHandle Voice) override;
	bool IsPlaying(int SampleId) override;
	int MixingRate() const override { return m_MixingRate; }
	void Mix(short *pFinalOut, unsigned Frames) override;
	void PauseAudioDevice() override {}
	void UnpauseAudioDevice() override {}
	CVoiceHandle Handle(int Id, int Age) { return CreateVoiceHandle(Id, Age); }
};

// C++ copy: engine/client/sound.cpp:35-181
unsigned CSound::AdvanceVoice(CVoice &Voice, unsigned Frames)
{
	// make sure that we don't go outside the sound data
	const unsigned Advanced = std::min(Frames, (unsigned)(Voice.m_pSample->m_NumFrames - Voice.m_Tick));
	Voice.m_Tick += Advanced;

	// free voice if not used any more
	if(Voice.m_Tick == Voice.m_pSample->m_NumFrames)
	{
		if(Voice.m_Flags & ISound::FLAG_LOOP)
		{
			Voice.m_Tick = Voice.m_pSample->m_LoopStart;
		}
		else
		{
			Voice.m_pSample = nullptr;
			Voice.m_Age++;
		}
	}

	return Advanced;
}

void CSound::Mix(short *pFinalOut, unsigned Frames)
{
	Frames = std::min(Frames, m_MaxFrames);
	mem_zero(m_pMixBuffer, Frames * 2 * sizeof(int));

	// acquire lock while we are mixing
	m_SoundLock.lock();

	const int MasterVol = m_SoundVolume.load(std::memory_order_relaxed);

	for(auto &Voice : m_aVoices)
	{
		if(!Voice.m_pSample)
			continue;

		// mix voice
		int *pOut = m_pMixBuffer;

		const CSample *pSample = Voice.m_pSample;
		const int Step = pSample->m_Channels; // setup input sources
		short *pInL = &pSample->m_pData[Voice.m_Tick * Step];
		short *pInR = &pSample->m_pData[Voice.m_Tick * Step + 1];

		const unsigned End = AdvanceVoice(Voice, Frames);

		int VolumeR = round_truncate(Voice.m_pChannel->m_Vol * (Voice.m_Vol / 255.0f));
		int VolumeL = VolumeR;

		// check if we have a mono sound
		if(pSample->m_Channels == 1)
			pInR = pInL;

		// volume calculation
		if(Voice.m_Flags & ISound::FLAG_POS && Voice.m_pChannel->m_Pan)
		{
			// TODO: we should respect the channel panning value
			const vec2 Delta = Voice.m_Position - vec2(m_ListenerPositionX.load(std::memory_order_relaxed), m_ListenerPositionY.load(std::memory_order_relaxed));
			vec2 Falloff = vec2(0.0f, 0.0f);

			float RangeX = 0.0f; // for panning
			bool InVoiceField = false;

			switch(Voice.m_Shape)
			{
			case ISound::SHAPE_CIRCLE:
			{
				const float Radius = Voice.m_Circle.m_Radius;
				RangeX = Radius;

				const float Dist = length(Delta);
				if(Dist < Radius)
				{
					InVoiceField = true;

					// falloff
					const float FalloffDistance = Radius * Voice.m_Falloff;
					Falloff.x = Falloff.y = Dist > FalloffDistance ? (Radius - Dist) / (Radius - FalloffDistance) : 1.0f;
				}
				break;
			}

			case ISound::SHAPE_RECTANGLE:
			{
				const vec2 AbsoluteDelta = vec2(absolute(Delta.x), absolute(Delta.y));
				const float w = Voice.m_Rectangle.m_Width / 2.0f;
				const float h = Voice.m_Rectangle.m_Height / 2.0f;
				RangeX = w;

				if(AbsoluteDelta.x < w && AbsoluteDelta.y < h)
				{
					InVoiceField = true;

					// falloff
					const vec2 FalloffDistance = vec2(w, h) * Voice.m_Falloff;
					Falloff.x = AbsoluteDelta.x > FalloffDistance.x ? (w - AbsoluteDelta.x) / (w - FalloffDistance.x) : 1.0f;
					Falloff.y = AbsoluteDelta.y > FalloffDistance.y ? (h - AbsoluteDelta.y) / (h - FalloffDistance.y) : 1.0f;
				}
				break;
			}
			};

			if(InVoiceField)
			{
				// panning
				if(!(Voice.m_Flags & ISound::FLAG_NO_PANNING))
				{
					if(Delta.x > 0)
						VolumeL = ((RangeX - absolute(Delta.x)) * VolumeL) / RangeX;
					else
						VolumeR = ((RangeX - absolute(Delta.x)) * VolumeR) / RangeX;
				}

				{
					VolumeL *= Falloff.x * Falloff.y;
					VolumeR *= Falloff.x * Falloff.y;
				}
			}
			else
			{
				VolumeL = 0;
				VolumeR = 0;
			}
		}

		// process all frames
		for(unsigned s = 0; s < End; s++)
		{
			*pOut++ += (*pInL) * VolumeL;
			*pOut++ += (*pInR) * VolumeR;
			pInL += Step;
			pInR += Step;
		}
	}

	m_SoundLock.unlock();

	// clamp accumulated values
	for(unsigned i = 0; i < Frames * 2; i++)
		pFinalOut[i] = std::clamp<int>(((m_pMixBuffer[i] * MasterVol) / 101) >> 8, std::numeric_limits<short>::min(), std::numeric_limits<short>::max());

#if defined(CONF_ARCH_ENDIAN_BIG)
	swap_endian(pFinalOut, sizeof(short), Frames * 2);
#endif
}
// end of copy

// C++ copy: engine/client/sound.cpp:408-462
CSample *CSound::AllocSample()
{
	const CLockScope LockScope(m_SoundLock);
	if(m_FirstFreeSampleIndex == SAMPLE_INDEX_FULL)
		return nullptr;

	CSample *pSample = &m_aSamples[m_FirstFreeSampleIndex];
	dbg_assert(
		pSample->m_pData == nullptr && pSample->m_NextFreeSampleIndex != SAMPLE_INDEX_USED,
		"Sample was not unloaded (index=%d, next=%d, duration=%f, data=%p)",
		pSample->m_Index, pSample->m_NextFreeSampleIndex, pSample->TotalTime(), pSample->m_pData);
	m_FirstFreeSampleIndex = pSample->m_NextFreeSampleIndex;
	pSample->m_NextFreeSampleIndex = SAMPLE_INDEX_USED;
	return pSample;
}

void CSound::RateConvert(CSample &Sample) const
{
	dbg_assert(Sample.IsLoaded(), "Sample not loaded: %d", Sample.m_Index);
	// make sure that we need to convert this sound
	if(Sample.m_Rate == m_MixingRate)
		return;

	// allocate new data
	const int NumFrames = (int)((Sample.m_NumFrames / (float)Sample.m_Rate) * m_MixingRate);
	short *pNewData = (short *)calloc((size_t)NumFrames * Sample.m_Channels, sizeof(short));

	for(int i = 0; i < NumFrames; i++)
	{
		// resample TODO: this should be done better, like linear at least
		float a = i / (float)NumFrames;
		int f = (int)(a * Sample.m_NumFrames);
		if(f >= Sample.m_NumFrames)
			f = Sample.m_NumFrames - 1;

		// set new data
		if(Sample.m_Channels == 1)
			pNewData[i] = Sample.m_pData[f];
		else if(Sample.m_Channels == 2)
		{
			pNewData[i * 2] = Sample.m_pData[f * 2];
			pNewData[i * 2 + 1] = Sample.m_pData[f * 2 + 1];
		}
	}

	// adjust looping position, note that this is not precise
	const double Factor = (double)m_MixingRate / (double)Sample.m_Rate;
	Sample.m_LoopStart = std::round(Sample.m_LoopStart * Factor);

	// free old data and apply new
	free(Sample.m_pData);
	Sample.m_pData = pNewData;
	Sample.m_NumFrames = NumFrames;
	Sample.m_Rate = m_MixingRate;
}
// end of copy

// C++ copy: engine/client/sound.cpp:807-1166
void CSound::UnloadSample(int SampleId)
{
	if(SampleId == -1)
		return;

	dbg_assert(SampleId >= 0 && SampleId < NUM_SAMPLES, "SampleId invalid: %d", SampleId);
	const CLockScope LockScope(m_SoundLock);
	CSample &Sample = m_aSamples[SampleId];

	if(Sample.IsLoaded())
	{
		// Stop voices using this sample
		for(auto &Voice : m_aVoices)
		{
			if(Voice.m_pSample == &Sample)
			{
				Voice.m_pSample = nullptr;
			}
		}

		// Free data
		free(Sample.m_pData);
		Sample.m_pData = nullptr;
	}

	// Free slot
	if(Sample.m_NextFreeSampleIndex == SAMPLE_INDEX_USED)
	{
		Sample.m_NextFreeSampleIndex = m_FirstFreeSampleIndex;
		m_FirstFreeSampleIndex = Sample.m_Index;
	}
}

float CSound::GetSampleTotalTime(int SampleId)
{
	dbg_assert(SampleId >= 0 && SampleId < NUM_SAMPLES, "SampleId invalid: %d", SampleId);

	const CLockScope LockScope(m_SoundLock);
	dbg_assert(m_aSamples[SampleId].IsLoaded(), "Sample not loaded: %d", SampleId);
	return m_aSamples[SampleId].TotalTime();
}

float CSound::GetSampleCurrentTime(int SampleId)
{
	dbg_assert(SampleId >= 0 && SampleId < NUM_SAMPLES, "SampleId invalid: %d", SampleId);

	const CLockScope LockScope(m_SoundLock);
	dbg_assert(m_aSamples[SampleId].IsLoaded(), "Sample not loaded: %d", SampleId);
	CSample *pSample = &m_aSamples[SampleId];
	for(auto &Voice : m_aVoices)
	{
		if(Voice.m_pSample == pSample)
		{
			return Voice.m_Tick / (float)pSample->m_Rate;
		}
	}

	return pSample->m_PausedAt / (float)pSample->m_Rate;
}

void CSound::SetSampleCurrentTime(int SampleId, float Time)
{
	dbg_assert(SampleId >= 0 && SampleId < NUM_SAMPLES, "SampleId invalid: %d", SampleId);

	const CLockScope LockScope(m_SoundLock);
	dbg_assert(m_aSamples[SampleId].IsLoaded(), "Sample not loaded: %d", SampleId);
	CSample *pSample = &m_aSamples[SampleId];
	for(auto &Voice : m_aVoices)
	{
		if(Voice.m_pSample == pSample)
		{
			Voice.m_Tick = pSample->m_NumFrames * Time;
			return;
		}
	}

	pSample->m_PausedAt = pSample->m_NumFrames * Time;
}

void CSound::SetChannel(int ChannelId, float Vol, float Pan)
{
	dbg_assert(ChannelId >= 0 && ChannelId < NUM_CHANNELS, "ChannelId invalid: %d", ChannelId);

	const CLockScope LockScope(m_SoundLock);
	m_aChannels[ChannelId].m_Vol = (int)(Vol * 255.0f);
	m_aChannels[ChannelId].m_Pan = (int)(Pan * 255.0f); // TODO: this is only on and off right now
}

void CSound::SetListenerPosition(vec2 Position)
{
	m_ListenerPositionX.store(Position.x, std::memory_order_relaxed);
	m_ListenerPositionY.store(Position.y, std::memory_order_relaxed);
}

void CSound::SetVoiceVolume(CVoiceHandle Voice, float Volume)
{
	if(!Voice.IsValid())
		return;

	int VoiceId = Voice.Id();

	const CLockScope LockScope(m_SoundLock);
	if(m_aVoices[VoiceId].m_Age != Voice.Age())
		return;

	Volume = std::clamp(Volume, 0.0f, 1.0f);
	m_aVoices[VoiceId].m_Vol = (int)(Volume * 255.0f);
}

void CSound::SetVoiceFalloff(CVoiceHandle Voice, float Falloff)
{
	if(!Voice.IsValid())
		return;

	int VoiceId = Voice.Id();

	const CLockScope LockScope(m_SoundLock);
	if(m_aVoices[VoiceId].m_Age != Voice.Age())
		return;

	Falloff = std::clamp(Falloff, 0.0f, 1.0f);
	m_aVoices[VoiceId].m_Falloff = Falloff;
}

void CSound::SetVoicePosition(CVoiceHandle Voice, vec2 Position)
{
	if(!Voice.IsValid())
		return;

	int VoiceId = Voice.Id();

	const CLockScope LockScope(m_SoundLock);
	if(m_aVoices[VoiceId].m_Age != Voice.Age())
		return;

	m_aVoices[VoiceId].m_Position = Position;
}

void CSound::SetVoiceTimeOffset(CVoiceHandle Voice, float TimeOffset)
{
	if(!Voice.IsValid())
		return;

	int VoiceId = Voice.Id();

	const CLockScope LockScope(m_SoundLock);
	if(m_aVoices[VoiceId].m_Age != Voice.Age())
		return;

	if(!m_aVoices[VoiceId].m_pSample)
		return;

	int Tick = 0;
	bool IsLooping = m_aVoices[VoiceId].m_Flags & ISound::FLAG_LOOP;
	uint64_t TickOffset = m_aVoices[VoiceId].m_pSample->m_Rate * TimeOffset;
	if(m_aVoices[VoiceId].m_pSample->m_NumFrames > 0 && IsLooping)
	{
		const int LoopStart = m_aVoices[VoiceId].m_pSample->m_LoopStart;
		const int NumFrames = m_aVoices[VoiceId].m_pSample->m_NumFrames;
		if(TickOffset < static_cast<uint64_t>(NumFrames))
		{
			// Still in first playthrough
			Tick = TickOffset;
		}
		else
		{
			// Past first playthrough, wrap within loop section only
			const int LoopLength = NumFrames - LoopStart;
			if(LoopLength > 0)
				Tick = LoopStart + ((TickOffset - NumFrames) % LoopLength);
			else
				Tick = LoopStart;
		}
	}
	else
	{
		Tick = std::clamp<uint64_t>(TickOffset, 0, m_aVoices[VoiceId].m_pSample->m_NumFrames);
	}

	// at least 200msec off, else depend on buffer size
	float Threshold = std::max(0.2f * m_aVoices[VoiceId].m_pSample->m_Rate, (float)m_MaxFrames);
	if(absolute(m_aVoices[VoiceId].m_Tick - Tick) > Threshold)
	{
		// take care of looping (modulo!)
		if(!(IsLooping && (std::min(m_aVoices[VoiceId].m_Tick, Tick) + m_aVoices[VoiceId].m_pSample->m_NumFrames - std::max(m_aVoices[VoiceId].m_Tick, Tick)) <= Threshold))
		{
			m_aVoices[VoiceId].m_Tick = Tick;
		}
	}
}

void CSound::SetVoiceCircle(CVoiceHandle Voice, float Radius)
{
	if(!Voice.IsValid())
		return;

	int VoiceId = Voice.Id();

	const CLockScope LockScope(m_SoundLock);
	if(m_aVoices[VoiceId].m_Age != Voice.Age())
		return;

	m_aVoices[VoiceId].m_Shape = ISound::SHAPE_CIRCLE;
	m_aVoices[VoiceId].m_Circle.m_Radius = std::max(0.0f, Radius);
}

void CSound::SetVoiceRectangle(CVoiceHandle Voice, float Width, float Height)
{
	if(!Voice.IsValid())
		return;

	int VoiceId = Voice.Id();

	const CLockScope LockScope(m_SoundLock);
	if(m_aVoices[VoiceId].m_Age != Voice.Age())
		return;

	m_aVoices[VoiceId].m_Shape = ISound::SHAPE_RECTANGLE;
	m_aVoices[VoiceId].m_Rectangle.m_Width = std::max(0.0f, Width);
	m_aVoices[VoiceId].m_Rectangle.m_Height = std::max(0.0f, Height);
}

ISound::CVoiceHandle CSound::Play(int ChannelId, int SampleId, int Flags, float Volume, vec2 Position)
{
	dbg_assert(ChannelId >= 0 && ChannelId < NUM_CHANNELS, "ChannelId invalid: %d", ChannelId);
	dbg_assert(SampleId >= 0 && SampleId < NUM_SAMPLES, "SampleId invalid: %d", SampleId);

	const CLockScope LockScope(m_SoundLock);

	// search for voice
	int VoiceId = -1;
	for(int i = 0; i < NUM_VOICES; i++)
	{
		int NextId = (m_NextVoice + i) % NUM_VOICES;
		if(!m_aVoices[NextId].m_pSample)
		{
			VoiceId = NextId;
			m_NextVoice = NextId + 1;
			break;
		}
	}
	if(VoiceId == -1)
	{
		return CreateVoiceHandle(-1, -1);
	}

	// voice found, use it
	m_aVoices[VoiceId].m_pSample = &m_aSamples[SampleId];
	m_aVoices[VoiceId].m_pChannel = &m_aChannels[ChannelId];
	if(Flags & FLAG_LOOP)
	{
		m_aVoices[VoiceId].m_Tick = m_aSamples[SampleId].m_PausedAt;
	}
	else if(Flags & FLAG_PREVIEW)
	{
		m_aVoices[VoiceId].m_Tick = m_aSamples[SampleId].m_PausedAt;
		m_aSamples[SampleId].m_PausedAt = 0;
	}
	else
	{
		m_aVoices[VoiceId].m_Tick = 0;
	}
	m_aVoices[VoiceId].m_Vol = (int)(std::clamp(Volume, 0.0f, 1.0f) * 255.0f);
	m_aVoices[VoiceId].m_Flags = Flags;
	m_aVoices[VoiceId].m_Position = Position;
	m_aVoices[VoiceId].m_Falloff = 0.0f;
	m_aVoices[VoiceId].m_Shape = ISound::SHAPE_CIRCLE;
	m_aVoices[VoiceId].m_Circle.m_Radius = 1500;
	return CreateVoiceHandle(VoiceId, m_aVoices[VoiceId].m_Age);
}

ISound::CVoiceHandle CSound::PlayAt(int ChannelId, int SampleId, int Flags, float Volume, vec2 Position)
{
	return Play(ChannelId, SampleId, Flags | ISound::FLAG_POS, Volume, Position);
}

ISound::CVoiceHandle CSound::Play(int ChannelId, int SampleId, int Flags, float Volume)
{
	return Play(ChannelId, SampleId, Flags, Volume, vec2(0.0f, 0.0f));
}

void CSound::Pause(int SampleId)
{
	dbg_assert(SampleId >= 0 && SampleId < NUM_SAMPLES, "SampleId invalid: %d", SampleId);

	// TODO: a nice fade out
	const CLockScope LockScope(m_SoundLock);
	CSample *pSample = &m_aSamples[SampleId];
	dbg_assert(m_aSamples[SampleId].IsLoaded(), "Sample not loaded: %d", SampleId);
	for(auto &Voice : m_aVoices)
	{
		if(Voice.m_pSample == pSample)
		{
			Voice.m_pSample->m_PausedAt = Voice.m_Tick;
			Voice.m_pSample = nullptr;
		}
	}
}

void CSound::Stop(int SampleId)
{
	dbg_assert(SampleId >= 0 && SampleId < NUM_SAMPLES, "SampleId invalid: %d", SampleId);

	// TODO: a nice fade out
	const CLockScope LockScope(m_SoundLock);
	CSample *pSample = &m_aSamples[SampleId];
	dbg_assert(m_aSamples[SampleId].IsLoaded(), "Sample not loaded: %d", SampleId);
	for(auto &Voice : m_aVoices)
	{
		if(Voice.m_pSample == pSample)
		{
			if(Voice.m_Flags & FLAG_LOOP)
				Voice.m_pSample->m_PausedAt = Voice.m_Tick;
			else
				Voice.m_pSample->m_PausedAt = 0;
			Voice.m_pSample = nullptr;
		}
	}
}

void CSound::StopAll()
{
	// TODO: a nice fade out
	const CLockScope LockScope(m_SoundLock);
	for(auto &Voice : m_aVoices)
	{
		if(Voice.m_pSample)
		{
			if(Voice.m_Flags & FLAG_LOOP)
				Voice.m_pSample->m_PausedAt = Voice.m_Tick;
			else
				Voice.m_pSample->m_PausedAt = 0;
		}
		Voice.m_pSample = nullptr;
	}
}

void CSound::StopVoice(CVoiceHandle Voice)
{
	if(!Voice.IsValid())
		return;

	int VoiceId = Voice.Id();

	const CLockScope LockScope(m_SoundLock);
	if(m_aVoices[VoiceId].m_Age != Voice.Age())
		return;

	m_aVoices[VoiceId].m_pSample = nullptr;
	m_aVoices[VoiceId].m_Age++;
}

bool CSound::IsPlaying(int SampleId)
{
	dbg_assert(SampleId >= 0 && SampleId < NUM_SAMPLES, "SampleId invalid: %d", SampleId);
	const CLockScope LockScope(m_SoundLock);
	const CSample *pSample = &m_aSamples[SampleId];
	dbg_assert(m_aSamples[SampleId].IsLoaded(), "Sample not loaded: %d", SampleId);
	return std::any_of(std::begin(m_aVoices), std::end(m_aVoices), [pSample](const auto &Voice) { return Voice.m_pSample == pSample; });
}
// end of copy

static float Frand(float Lo, float Hi)
{
	return Lo + (Hi - Lo) * (rs_sound::Rand() % 100001) / 100000.0f;
}

static unsigned Bits(float f)
{
	unsigned u;
	mem_copy(&u, &f, sizeof(u));
	return u;
}

} // namespace rs_mixer

int MixerMode(const char *pOutFile, const char *pAudioDir)
{
	using rs_sound::Rand;
	FILE *pOut = fopen(pOutFile, "w");
	if(!pOut)
		return -1;
	std::vector<std::string> vAll;
	for(const auto &Entry : std::filesystem::directory_iterator(pAudioDir))
		if(Entry.path().extension() == ".wv" && Entry.path().filename() != "music_menu.wv")
			vAll.push_back(Entry.path().filename().string());
	std::sort(vAll.begin(), vAll.end());
	// every 10th sound plus the stereo and the 48 kHz ones
	std::vector<std::string> vNames;
	for(size_t i = 0; i < vAll.size(); i++)
		if(i % 10 == 0 || vAll[i].rfind("wp_laser", 0) == 0 || vAll[i] == "wp_ninja_attack-01.wv")
			vNames.push_back(vAll[i]);
	rs_sound::CSound Decoder;
	std::vector<rs_sound::CSample> vDecoded;
	for(const std::string &Name : vNames)
	{
		std::ifstream File(std::filesystem::path(pAudioDir) / Name, std::ios::binary);
		const std::vector<unsigned char> vData((std::istreambuf_iterator<char>(File)), std::istreambuf_iterator<char>());
		rs_sound::CSample Sample;
		if(!Decoder.DecodeWV(Sample, vData.data(), vData.size(), Name.c_str()))
			return -1;
		vDecoded.push_back(Sample);
	}
	for(int Rate : {48000, 44100})
	{
		rs_mixer::CSound Sound;
		Sound.Init(Rate, 512);
		fprintf(pOut, "run %d 512 %d\n", Rate, (int)vNames.size());
		std::vector<int> vIds(vNames.size()), vLoop(vNames.size());
		for(size_t i = 0; i < vNames.size(); i++)
		{
			vLoop[i] = i % 3 == 0 ? Rand() % vDecoded[i].m_NumFrames : 0;
			vIds[i] = Sound.Load(vDecoded[i], vLoop[i]);
			fprintf(pOut, "load %s %d %d\n", vNames[i].c_str(), vLoop[i], vIds[i]);
		}
		std::vector<ISound::CVoiceHandle> vHandles;
		auto Handle = [&]() {
			const unsigned r = Rand() % 10;
			if(r == 0 || vHandles.empty())
				return Sound.Handle((int)(Rand() % 300) - 20, (int)(Rand() % 4) - 1);
			// mostly one of the last plays, whose voices are likely still alive
			if(r < 7)
				return vHandles[vHandles.size() - 1 - Rand() % std::min<size_t>(vHandles.size(), 4)];
			return vHandles[Rand() % vHandles.size()];
		};
		auto Loaded = [&]() {
			std::vector<int> v;
			for(int Id : vIds)
				if(Id >= 0)
					v.push_back(Id);
			return v;
		};
		for(int Op = 0; Op < 5000; Op++)
		{
			const unsigned Kind = Rand() % 24;
			const std::vector<int> vLoaded = Loaded();
			// sometimes a slot that was never loaded (a slot that was unloaded keeps its frame
			// count without data, which C++ Mix would read through a null pointer)
			const int AnySample = vLoaded.empty() || Rand() % 8 == 0 ? (int)(64 + Rand() % 448) : vLoaded[Rand() % vLoaded.size()];
			if(Kind <= 4)
			{
				const int Ch = Rand() % 16, Flags = Rand() % 16;
				const float Vol = rs_mixer::Frand(-0.2f, 1.3f);
				const vec2 Pos(rs_mixer::Frand(-3000, 3000), rs_mixer::Frand(-3000, 3000));
				const int How = Rand() % 3;
				ISound::CVoiceHandle h = How == 0 ? Sound.Play(Ch, AnySample, Flags, Vol) : How == 1 ? Sound.PlayAt(Ch, AnySample, Flags, Vol, Pos) : Sound.Play(Ch, AnySample, Flags, Vol, Pos);
				vHandles.push_back(h);
				fprintf(pOut, "play %d %d %d %d %08x %08x %08x = %d %d\n", How, Ch, AnySample, Flags, rs_mixer::Bits(Vol), rs_mixer::Bits(Pos.x), rs_mixer::Bits(Pos.y), h.Id(), h.Age());
			}
			else if(Kind <= 10)
			{
				const ISound::CVoiceHandle h = Handle();
				const float a = rs_mixer::Frand(-0.3f, 1.3f), c = rs_mixer::Frand(-3000, 3000);
				float b = rs_mixer::Frand(-3000, 3000);
				const int Which = Rand() % 6;
				if(Which == 3)
				{
					// as map sounds do: half the offsets near where the voice is
					b = std::abs(b) / 100.0f;
					const rs_mixer::CVoice *pVoice = h.Id() >= 0 && h.Id() < 256 ? &Sound.m_aVoices[h.Id()] : nullptr;
					if(Rand() % 2 && pVoice && pVoice->m_pSample)
						b = std::max(0.0f, pVoice->m_Tick / (float)pVoice->m_pSample->m_Rate + rs_mixer::Frand(-0.4f, 0.4f));
				}
				if(Which == 0)
					Sound.SetVoiceVolume(h, a);
				else if(Which == 1)
					Sound.SetVoiceFalloff(h, a);
				else if(Which == 2)
					Sound.SetVoicePosition(h, vec2(b, c));
				else if(Which == 3)
					Sound.SetVoiceTimeOffset(h, b);
				else if(Which == 4)
					Sound.SetVoiceCircle(h, b);
				else
					Sound.SetVoiceRectangle(h, b, c);
				fprintf(pOut, "voice %d %d %d %08x %08x %08x\n", Which, h.Id(), h.Age(), rs_mixer::Bits(a), rs_mixer::Bits(b), rs_mixer::Bits(c));
			}
			else if(Kind == 11)
			{
				const int Ch = Rand() % 16;
				const float Pans[] = {0.0f, 1.0f, 0.5f, 0.004f};
				const float Vol = rs_mixer::Frand(0, 1), Pan = Pans[Rand() % 4];
				Sound.SetChannel(Ch, Vol, Pan);
				fprintf(pOut, "channel %d %08x %08x\n", Ch, rs_mixer::Bits(Vol), rs_mixer::Bits(Pan));
			}
			else if(Kind == 12)
			{
				const vec2 Pos(rs_mixer::Frand(-3000, 3000), rs_mixer::Frand(-3000, 3000));
				Sound.SetListenerPosition(Pos);
				fprintf(pOut, "listener %08x %08x\n", rs_mixer::Bits(Pos.x), rs_mixer::Bits(Pos.y));
			}
			else if(Kind == 13 && !vLoaded.empty())
			{
				// the sample calls assert a loaded sample
				const int Id = vLoaded[Rand() % vLoaded.size()];
				const int Which = Rand() % 6;
				const float t = rs_mixer::Frand(0, 1);
				float r = 0;
				if(Which == 0)
					Sound.Stop(Id);
				else if(Which == 1)
					Sound.Pause(Id);
				else if(Which == 2)
					r = Sound.IsPlaying(Id);
				else if(Which == 3)
					Sound.SetSampleCurrentTime(Id, t);
				else if(Which == 4)
					r = Sound.GetSampleCurrentTime(Id);
				else
					r = Sound.GetSampleTotalTime(Id);
				fprintf(pOut, "sample %d %d %08x = %08x\n", Which, Id, rs_mixer::Bits(t), rs_mixer::Bits(r));
			}
			else if(Kind == 14)
			{
				const ISound::CVoiceHandle h = Handle();
				Sound.StopVoice(h);
				fprintf(pOut, "stopvoice %d %d\n", h.Id(), h.Age());
			}
			else if(Kind == 15 && Rand() % 20 == 0)
			{
				Sound.StopAll();
				fprintf(pOut, "stopall\n");
			}
			else if(Kind == 16 && Rand() % 4 == 0)
			{
				// unload one sample or load an unloaded one again
				const size_t i = Rand() % vIds.size();
				if(vIds[i] >= 0)
				{
					Sound.UnloadSample(vIds[i]);
					fprintf(pOut, "unload %d\n", vIds[i]);
					vIds[i] = -1;
				}
				else
				{
					vIds[i] = Sound.Load(vDecoded[i], vLoop[i]);
					fprintf(pOut, "reload %d = %d\n", (int)i, vIds[i]);
				}
			}
			else if(Kind == 17)
			{
				Sound.m_SoundVolume = Rand() % 101;
				fprintf(pOut, "volume %d\n", Sound.m_SoundVolume.load());
			}
			else
			{
				const unsigned Frames = 1 + Rand() % 1100;
				std::vector<short> vOut(Frames * 2, 0x5555);
				Sound.Mix(vOut.data(), Frames);
				fprintf(pOut, "mix %u = %016" PRIx64 "\n", Frames, rs_sound::Fnv(vOut.data(), vOut.size()));
			}
		}
	}
	for(auto &Sample : vDecoded)
		free(Sample.m_pData);
	fclose(pOut);
	return 0;
}

int WvMode(const char *pOutFile, int NumDirs, const char *const *ppDirs)
{
	using namespace rs_sound;
	FILE *pOut = fopen(pOutFile, "w");
	if(!pOut)
		return -1;
	CSound Sound;
	for(int d = 0; d < NumDirs; d++)
	{
		std::vector<std::string> vNames;
		for(const auto &Entry : std::filesystem::directory_iterator(ppDirs[d]))
			if(Entry.path().extension() == ".wv")
				vNames.push_back(Entry.path().filename().string());
		std::sort(vNames.begin(), vNames.end());
		fprintf(pOut, "dir %s %d\n", ppDirs[d], (int)vNames.size());
		for(const std::string &Name : vNames)
		{
			std::ifstream File(std::filesystem::path(ppDirs[d]) / Name, std::ios::binary);
			const std::vector<unsigned char> vOrig((std::istreambuf_iterator<char>(File)), std::istreambuf_iterator<char>());
			for(int Variant = 0; Variant < 5; Variant++)
			{
				std::vector<unsigned char> vData = vOrig;
				std::vector<std::pair<unsigned, unsigned>> vSet;
				const unsigned Len = vData.size();
				if(Variant == 1 && Len)
					vData.resize(Rand() % Len);
				else if((Variant == 2 || Variant == 4) && Len)
				{
					const unsigned Pos = Rand() % (Variant == 4 ? std::min(Len, 64u) : Len);
					vSet.emplace_back(Pos, vData[Pos] ^ (1u << (Rand() % 8)));
				}
				else if(Variant == 3 && Len)
					for(int i = 0; i < 8; i++)
						vSet.emplace_back(Rand() % Len, Rand() % 256);
				for(const auto &[Pos, Value] : vSet)
					vData[Pos] = Value;
				fprintf(pOut, "file %s %d %u %d", Name.c_str(), Variant, (unsigned)vData.size(), (int)vSet.size());
				for(const auto &[Pos, Value] : vSet)
					fprintf(pOut, " %u:%u", Pos, Value);
				CSample Sample;
				if(Sound.DecodeWV(Sample, vData.data(), vData.size(), Name.c_str()))
				{
					fprintf(pOut, " ok %d %d %d %016" PRIx64 "\n", Sample.m_NumFrames, Sample.m_Rate, Sample.m_Channels,
						Fnv(Sample.m_pData, (size_t)Sample.m_NumFrames * Sample.m_Channels));
					free(Sample.m_pData);
				}
				else
					fprintf(pOut, " fail\n");
			}
		}
	}
	fclose(pOut);
	return 0;
}

int OpusMode(const char *pOutDir, const char *pDir)
{
	using namespace rs_sound;
	FILE *pOut = fopen((std::string(pOutDir) + "/opus.txt").c_str(), "w");
	if(!pOut)
		return -1;
	std::vector<std::string> vNames;
	for(const auto &Entry : std::filesystem::directory_iterator(pDir))
		if(Entry.path().extension() == ".opus")
			vNames.push_back(Entry.path().filename().string());
	std::sort(vNames.begin(), vNames.end());
	CSound Sound;
	for(const std::string &Name : vNames)
	{
		std::ifstream File(std::filesystem::path(pDir) / Name, std::ios::binary);
		const std::vector<unsigned char> vData((std::istreambuf_iterator<char>(File)), std::istreambuf_iterator<char>());
		CSample Sample;
		if(!Sound.DecodeOpus(Sample, vData.data(), vData.size(), Name.c_str()))
		{
			fprintf(pOut, "file %s fail\n", Name.c_str());
			continue;
		}
		fprintf(pOut, "file %s ok %d %d %d %d\n", Name.c_str(), Sample.m_NumFrames, Sample.m_Rate, Sample.m_Channels, Sample.m_LoopStart);
		FILE *pPcm = fopen((std::string(pOutDir) + "/" + Name + ".pcm").c_str(), "wb");
		fwrite(Sample.m_pData, sizeof(short), (size_t)Sample.m_NumFrames * Sample.m_Channels, pPcm);
		fclose(pPcm);
		free(Sample.m_pData);
	}
	fclose(pOut);
	return 0;
}
