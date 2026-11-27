// "entities" mode of rs_parity: the entities textures (PLAN §13.4), for
// rust-client/client/tests/assets.rs.
//
// CMapImages needs the client, so its entities parts are copied here word for word into a
// stand-in class of the same name (in its own namespace): EMapImageModType and
// gs_apModEntitiesNames, GetEntitiesModType, IsValidTile and GetEntities. Each copy is
// marked "C++ copy: <file>:<lines>" ... "end of copy"; `cpp_copies_match_source` checks
// them. What the copies call is the real code: CImageLoader::LoadPng (through the
// storage search over the given roots), ConvertToGrayscaleRect, the IsValid*Tile
// functions, CImageInfo::CopyRectFrom. The graphics backend has no texture arrays (as
// DDNet-rs), so the tune colour map is not built.
//
// For every entities pack in assets/entities of any root and "default", every mod and
// both masks: the path loaded and both layer textures' size, format and FNV-1a.

#include <base/dbg.h>
#include <base/log.h>
#include <base/math.h>
#include <base/mem.h>
#include <base/str.h>

#include <engine/gfx/image_loader.h>
#include <engine/gfx/image_manipulation.h>
#include <engine/graphics.h>
#include <engine/image.h>
#include <engine/storage.h>

#include <game/map/render_interfaces.h>
#include <game/mapitems.h>

#include <cinttypes>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <set>
#include <string>
#include <vector>

namespace rs_entities {

static FILE *s_pOut = nullptr;

static std::filesystem::path Path(const std::string &Utf8) { return std::filesystem::u8path(Utf8); }
static std::string Utf8(const std::filesystem::path &Path)
{
	const auto s = Path.u8string();
	return std::string(s.begin(), s.end());
}

static uint64_t Fnv(const uint8_t *pData, size_t Size)
{
	uint64_t h = 0xcbf29ce484222325ull;
	for(size_t i = 0; i < Size; i++)
		h = (h ^ pData[i]) * 0x100000001b3ull;
	return h;
}

// C++ copy: game/client/components/mapimages.h:13-34
enum EMapImageModType
{
	MAP_IMAGE_MOD_TYPE_DDNET = 0,
	MAP_IMAGE_MOD_TYPE_DDRACE,
	MAP_IMAGE_MOD_TYPE_RACE,
	MAP_IMAGE_MOD_TYPE_BLOCKWORLDS,
	MAP_IMAGE_MOD_TYPE_FNG,
	MAP_IMAGE_MOD_TYPE_VANILLA,
	MAP_IMAGE_MOD_TYPE_FDDRACE,

	MAP_IMAGE_MOD_TYPE_COUNT,
};

constexpr const char *const gs_apModEntitiesNames[] = {
	"ddnet",
	"ddrace",
	"race",
	"blockworlds",
	"fng",
	"vanilla",
	"f-ddrace",
};
// end of copy

// the game info fields GetEntities reads
struct CGameInfo
{
	bool m_EntitiesFDDrace = false;
	bool m_EntitiesDDNet = false;
	bool m_EntitiesDDRace = false;
	bool m_EntitiesRace = false;
	bool m_EntitiesBW = false;
	bool m_EntitiesFNG = false;
	bool m_EntitiesVanilla = false;
	bool m_DontMaskEntities = false;
};

struct CGameClient
{
	CGameInfo m_GameInfo;
};

class CGraphics
{
public:
	std::vector<std::string> m_vRoots;
	bool HasTextureArraysSupport() const { return false; }
	int TextureLoadFlags() const { return 0; }
	bool LoadPng(CImageInfo &Image, const char *pFilename, int StorageType)
	{
		for(const std::string &Root : m_vRoots)
		{
			std::ifstream File(Path(Root) / Path(pFilename), std::ios::binary);
			if(!File)
				continue;
			const std::vector<uint8_t> vData((std::istreambuf_iterator<char>(File)), std::istreambuf_iterator<char>());
			CByteBufferReader Reader(vData.data(), vData.size());
			int Incompatible;
			// the file loader of CGraphics_Threaded::LoadPng (gray images fail there)
			return CImageLoader::LoadPng(Reader, pFilename, Image, Incompatible) && (Image.m_Format == CImageInfo::FORMAT_RGB || Image.m_Format == CImageInfo::FORMAT_RGBA);
		}
		return false;
	}
	IGraphics::CTextureHandle LoadTextureRaw(const CImageInfo &Image, int Flags, const char *pTexName)
	{
		fprintf(s_pOut, "layer %s %d %d %d %016" PRIx64 "\n", pTexName, (int)Image.m_Width, (int)Image.m_Height, (int)Image.m_Format, Fnv(Image.m_pData, Image.DataSize()));
		return IGraphics::CTextureHandle();
	}
	IGraphics::CTextureHandle LoadTextureRawMove(CImageInfo &Image, int Flags) { return IGraphics::CTextureHandle(); }
};

// C++ copy: game/client/components/mapimages.cpp:200-218
static EMapImageModType GetEntitiesModType(const CGameInfo &GameInfo)
{
	if(GameInfo.m_EntitiesFDDrace)
		return MAP_IMAGE_MOD_TYPE_FDDRACE;
	else if(GameInfo.m_EntitiesDDNet)
		return MAP_IMAGE_MOD_TYPE_DDNET;
	else if(GameInfo.m_EntitiesDDRace)
		return MAP_IMAGE_MOD_TYPE_DDRACE;
	else if(GameInfo.m_EntitiesRace)
		return MAP_IMAGE_MOD_TYPE_RACE;
	else if(GameInfo.m_EntitiesBW)
		return MAP_IMAGE_MOD_TYPE_BLOCKWORLDS;
	else if(GameInfo.m_EntitiesFNG)
		return MAP_IMAGE_MOD_TYPE_FNG;
	else if(GameInfo.m_EntitiesVanilla)
		return MAP_IMAGE_MOD_TYPE_VANILLA;
	else
		return MAP_IMAGE_MOD_TYPE_DDNET;
}
// end of copy

// C++ copy: game/client/components/mapimages.cpp:220-260
static bool IsValidTile(int LayerType, bool EntitiesAreMasked, EMapImageModType EntitiesModType, int TileIndex)
{
	if(TileIndex == TILE_AIR)
		return false;
	if(!EntitiesAreMasked)
		return true;

	if(EntitiesModType == MAP_IMAGE_MOD_TYPE_DDNET || EntitiesModType == MAP_IMAGE_MOD_TYPE_DDRACE)
	{
		if(EntitiesModType == MAP_IMAGE_MOD_TYPE_DDNET || TileIndex != TILE_SPEED_BOOST_OLD)
		{
			if(LayerType == MAP_IMAGE_ENTITY_LAYER_TYPE_ALL_EXCEPT_SWITCH &&
				!IsValidGameTile(TileIndex) &&
				!IsValidFrontTile(TileIndex) &&
				!IsValidSpeedupTile(TileIndex) &&
				!IsValidTeleTile(TileIndex) &&
				!IsValidTuneTile(TileIndex))
			{
				return false;
			}
			else if(LayerType == MAP_IMAGE_ENTITY_LAYER_TYPE_SWITCH &&
				!IsValidSwitchTile(TileIndex))
			{
				return false;
			}
		}
	}
	else if(EntitiesModType == MAP_IMAGE_MOD_TYPE_RACE && IsCreditsTile(TileIndex))
	{
		return false;
	}
	else if(EntitiesModType == MAP_IMAGE_MOD_TYPE_FNG && IsCreditsTile(TileIndex))
	{
		return false;
	}
	else if(EntitiesModType == MAP_IMAGE_MOD_TYPE_VANILLA && IsCreditsTile(TileIndex))
	{
		return false;
	}
	return true;
}
// end of copy

class CMapImages
{
public:
	CGraphics *m_pGraphics;
	CGameClient *m_pGameClient;
	char m_aEntitiesPath[IO_MAX_PATH_LENGTH];
	bool m_aEntitiesIsLoaded[MAP_IMAGE_MOD_TYPE_COUNT * 2] = {};
	bool m_TuneColorsIsLoaded = false;
	IGraphics::CTextureHandle m_aaEntitiesTextures[MAP_IMAGE_MOD_TYPE_COUNT * 2][MAP_IMAGE_ENTITY_LAYER_TYPE_COUNT];
	IGraphics::CTextureHandle m_TuneColorMapTexture;
	CGraphics *Graphics() const { return m_pGraphics; }
	CGameClient *GameClient() const { return m_pGameClient; }
	IGraphics::CTextureHandle GetEntities(EMapImageEntityLayerType EntityLayerType);
};

// C++ copy: game/client/components/mapimages.cpp:262-365
IGraphics::CTextureHandle CMapImages::GetEntities(EMapImageEntityLayerType EntityLayerType)
{
	const bool EntitiesAreMasked = !GameClient()->m_GameInfo.m_DontMaskEntities;
	const EMapImageModType EntitiesModType = GetEntitiesModType(GameClient()->m_GameInfo);

	if(!m_aEntitiesIsLoaded[(EntitiesModType * 2) + (int)EntitiesAreMasked])
	{
		m_aEntitiesIsLoaded[(EntitiesModType * 2) + (int)EntitiesAreMasked] = true;

		int TextureLoadFlag = 0;
		if(Graphics()->HasTextureArraysSupport())
			TextureLoadFlag = Graphics()->TextureLoadFlags() | IGraphics::TEXLOAD_NO_2D_TEXTURE;

		CImageInfo ImgInfo;
		char aPath[IO_MAX_PATH_LENGTH];
		str_format(aPath, sizeof(aPath), "%s/%s.png", m_aEntitiesPath, gs_apModEntitiesNames[EntitiesModType]);
		Graphics()->LoadPng(ImgInfo, aPath, IStorage::TYPE_ALL);

		// try as single ddnet replacement
		if(ImgInfo.m_pData == nullptr && EntitiesModType == MAP_IMAGE_MOD_TYPE_DDNET)
		{
			str_format(aPath, sizeof(aPath), "%s.png", m_aEntitiesPath);
			Graphics()->LoadPng(ImgInfo, aPath, IStorage::TYPE_ALL);
		}

		// try default
		if(ImgInfo.m_pData == nullptr)
		{
			str_format(aPath, sizeof(aPath), "editor/entities_clear/%s.png", gs_apModEntitiesNames[EntitiesModType]);
			Graphics()->LoadPng(ImgInfo, aPath, IStorage::TYPE_ALL);
		}

		if(ImgInfo.m_pData != nullptr)
		{
			CImageInfo BuildImageInfo;
			BuildImageInfo.m_Width = ImgInfo.m_Width;
			BuildImageInfo.m_Height = ImgInfo.m_Height;
			BuildImageInfo.m_Format = ImgInfo.m_Format;
			BuildImageInfo.Allocate(); // allocate already transparent image

			// convert tune tile to gray
			const size_t CopyWidth = ImgInfo.m_Width / 16;
			const size_t CopyHeight = ImgInfo.m_Height / 16;
			const size_t TuneTileX = static_cast<size_t>(TILE_TUNE % 16) * CopyWidth;
			const size_t TuneTileY = static_cast<size_t>(TILE_TUNE / 16) * CopyHeight;

			ConvertToGrayscaleRect(ImgInfo, TuneTileX, TuneTileY, CopyWidth, CopyHeight);

			// build game layer
			for(int LayerType = 0; LayerType < MAP_IMAGE_ENTITY_LAYER_TYPE_COUNT; ++LayerType)
			{
				dbg_assert(!m_aaEntitiesTextures[(EntitiesModType * 2) + (int)EntitiesAreMasked][LayerType].IsValid(), "entities texture already loaded when it should not be");

				// set everything transparent
				mem_zero(BuildImageInfo.m_pData, BuildImageInfo.DataSize());

				for(int i = 0; i < 256; ++i)
				{
					int TileIndex = i;
					if(IsValidTile(LayerType, EntitiesAreMasked, EntitiesModType, TileIndex))
					{
						if(LayerType == MAP_IMAGE_ENTITY_LAYER_TYPE_SWITCH && TileIndex == TILE_SWITCHTIMEDOPEN)
						{
							TileIndex = 8;
						}

						const size_t OffsetX = (size_t)(TileIndex % 16) * CopyWidth;
						const size_t OffsetY = (size_t)(TileIndex / 16) * CopyHeight;
						BuildImageInfo.CopyRectFrom(ImgInfo, OffsetX, OffsetY, CopyWidth, CopyHeight, OffsetX, OffsetY);
					}
				}

				m_aaEntitiesTextures[(EntitiesModType * 2) + (int)EntitiesAreMasked][LayerType] = Graphics()->LoadTextureRaw(BuildImageInfo, TextureLoadFlag, aPath);
			}

			BuildImageInfo.Free();

			// build tune map from the tune tile
			if(Graphics()->HasTextureArraysSupport())
			{
				CImageInfo TuneMapInfo;
				TuneMapInfo.m_Width = ImgInfo.m_Width;
				TuneMapInfo.m_Height = ImgInfo.m_Height;
				TuneMapInfo.m_Format = ImgInfo.m_Format;
				TuneMapInfo.AllocateFillZero();

				for(int TileIndex = 1; TileIndex < 256; ++TileIndex)
				{
					size_t StartX = CopyWidth * (TileIndex % 16);
					size_t StartY = CopyHeight * (TileIndex / 16);
					TuneMapInfo.CopyRectFrom(ImgInfo, TuneTileX, TuneTileY, CopyWidth, CopyHeight, StartX, StartY);
					float Hue = std::fmod((TileIndex - 1) * normalized_golden_angle, 1.0f);
					ColorizeWithHueRect(TuneMapInfo, Hue, 0.75f, StartX, StartY, CopyWidth, CopyHeight);
				}
				m_TuneColorMapTexture = Graphics()->LoadTextureRawMove(TuneMapInfo, TextureLoadFlag);
				m_TuneColorsIsLoaded = true;
			}

			ImgInfo.Free();
		}
	}

	return m_aaEntitiesTextures[(EntitiesModType * 2) + (int)EntitiesAreMasked][EntityLayerType];
}
// end of copy

} // namespace rs_entities

int EntitiesMode(const char *pOutFile, int NumRoots, const char *const *ppRoots)
{
	using namespace rs_entities;
	s_pOut = fopen(pOutFile, "w");
	if(!s_pOut)
		return -1;
	CGraphics Graphics;
	for(int i = 0; i < NumRoots; i++)
		Graphics.m_vRoots.emplace_back(ppRoots[i]);
	std::set<std::string> Packs = {"default"};
	for(const std::string &Root : Graphics.m_vRoots)
	{
		std::error_code Error;
		for(const auto &Entry : std::filesystem::directory_iterator(Path(Root) / "assets" / "entities", Error))
		{
			if(Entry.is_directory())
				Packs.insert(Utf8(Entry.path().filename()));
			else if(Entry.path().extension() == ".png")
				Packs.insert(Utf8(Entry.path().stem()));
		}
	}
	for(const std::string &Pack : Packs)
	{
		for(int Mod = 0; Mod < MAP_IMAGE_MOD_TYPE_COUNT; Mod++)
		{
			for(int Masked = 0; Masked < 2; Masked++)
			{
				CGameClient GameClient;
				CGameInfo &Info = GameClient.m_GameInfo;
				Info.m_EntitiesFDDrace = Mod == MAP_IMAGE_MOD_TYPE_FDDRACE;
				Info.m_EntitiesDDNet = Mod == MAP_IMAGE_MOD_TYPE_DDNET;
				Info.m_EntitiesDDRace = Mod == MAP_IMAGE_MOD_TYPE_DDRACE;
				Info.m_EntitiesRace = Mod == MAP_IMAGE_MOD_TYPE_RACE;
				Info.m_EntitiesBW = Mod == MAP_IMAGE_MOD_TYPE_BLOCKWORLDS;
				Info.m_EntitiesFNG = Mod == MAP_IMAGE_MOD_TYPE_FNG;
				Info.m_EntitiesVanilla = Mod == MAP_IMAGE_MOD_TYPE_VANILLA;
				Info.m_DontMaskEntities = !Masked;
				CMapImages MapImages;
				MapImages.m_pGraphics = &Graphics;
				MapImages.m_pGameClient = &GameClient;
				// CMapImages::OnInit's entities path
				if(Pack == "default")
					str_copy(MapImages.m_aEntitiesPath, "editor/entities_clear");
				else
					str_format(MapImages.m_aEntitiesPath, sizeof(MapImages.m_aEntitiesPath), "assets/entities/%s", Pack.c_str());
				fprintf(s_pOut, "pack %s %d %d\n", Pack.c_str(), Mod, Masked);
				MapImages.GetEntities(MAP_IMAGE_ENTITY_LAYER_TYPE_ALL_EXCEPT_SWITCH);
			}
		}
	}
	fclose(s_pOut);
	return 0;
}
