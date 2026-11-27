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

#include <base/log.h>
#include <base/mem.h>
#include <base/str.h>

#include <algorithm>
#include <cinttypes>
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
