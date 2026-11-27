// "assets" mode of rs_parity: the asset pack loader (PLAN §13.4), for
// rust-client/client/tests/assets.rs.
//
// CGameClient and CGraphics_Threaded need the whole client, so their loader parts are
// copied here word for word into stand-in classes of the same names (in their own
// namespace): CImageAsset, LoadAssetFromPath, CheckImageDivisibility, IsImageFormatRgba,
// LoadSpriteTexture and IsImageSubFullyTransparent. Each copy is marked "C++ copy:
// <file>:<lines>" ... "end of copy"; `cpp_copies_match_source` checks them. What the
// copies call is the real code: CImageLoader::LoadPng, ResizeImage, g_pData, str_*.
// LoadPng(TYPE_ALL) is the storage search over the given roots in order; the retry chain
// of LoadGameSkin & co. (assets.cpp:128-140) is written out in LoadSkin below.
//
// For every asset kind, every pack name found in assets/<kind>/ of any root (files and
// directories), "default" and a missing name: the path the chain loads, whether the image
// is usable, and each sprite of the kind's sprite set (size and FNV-1a of its pixels,
// which tells whether the default's sprite replaced an invisible one).

#include <base/dbg.h>
#include <base/log.h>
#include <base/math.h>
#include <base/str.h>

#include <engine/gfx/image_loader.h>
#include <engine/gfx/image_manipulation.h>
#include <engine/graphics.h>
#include <engine/image.h>
#include <engine/storage.h>
#include <engine/warning.h>

#include <game/localization.h>

#include <generated/client_data.h>

#include <algorithm>
#include <cinttypes>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <optional>
#include <set>
#include <string>
#include <vector>

namespace rs_assets {

static FILE *s_pOut = nullptr;

// paths are UTF-8 (pack names may be Cyrillic)
static std::filesystem::path Path(const std::string &Utf8)
{
	return std::filesystem::u8path(Utf8);
}

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

class CGraphics_Threaded
{
public:
	std::vector<std::string> m_vRoots;

	void AddWarning(const SWarning &Warning) {}

	// IGraphics::LoadPng(TYPE_ALL): the first root that has the file
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
			return CImageLoader::LoadPng(Reader, pFilename, Image, Incompatible);
		}
		return false;
	}

	IGraphics::CTextureHandle LoadTextureRawMove(CImageInfo &Image, int Flags, const char *pTexName)
	{
		fprintf(s_pOut, "sprite %s %d %d %016" PRIx64 "\n", pTexName, (int)Image.m_Width, (int)Image.m_Height, Fnv(Image.m_pData, Image.DataSize()));
		Image.Free();
		return IGraphics::CTextureHandle();
	}

	bool CheckImageDivisibility(const char *pContextName, CImageInfo &Image, int DivX, int DivY, bool AllowResize);
	bool IsImageFormatRgba(const char *pContextName, const CImageInfo &Image);
	IGraphics::CTextureHandle LoadSpriteTexture(const CImageInfo &FromImageInfo, const std::optional<CImageInfo> &FallbackImageInfo, const CDataSprite *pSprite);
	bool IsImageSubFullyTransparent(const CImageInfo &FromImageInfo, int x, int y, int w, int h);
};

// C++ copy: engine/client/graphics_threaded.cpp:321-365
IGraphics::CTextureHandle CGraphics_Threaded::LoadSpriteTexture(const CImageInfo &FromImageInfo, const std::optional<CImageInfo> &FallbackImageInfo, const CDataSprite *pSprite)
{
	int ImageGridX = FromImageInfo.m_Width / pSprite->m_pSet->m_Gridx;
	int ImageGridY = FromImageInfo.m_Height / pSprite->m_pSet->m_Gridy;
	int x = pSprite->m_X * ImageGridX;
	int y = pSprite->m_Y * ImageGridY;
	int w = pSprite->m_W * ImageGridX;
	int h = pSprite->m_H * ImageGridY;

	// check for invisible texture, maybe due to outdated game assets
	if(FallbackImageInfo.has_value() && IsImageSubFullyTransparent(FromImageInfo, x, y, w, h))
	{
		log_warn("graphics", "Asset '%s' appears to be invisible, falling back to default", pSprite->m_pName);
		return LoadSpriteTexture(FallbackImageInfo.value(), std::nullopt, pSprite);
	}

	CImageInfo SpriteInfo;
	SpriteInfo.m_Width = w;
	SpriteInfo.m_Height = h;
	SpriteInfo.m_Format = FromImageInfo.m_Format;
	SpriteInfo.Allocate();
	SpriteInfo.CopyRectFrom(FromImageInfo, x, y, w, h, 0, 0);
	return LoadTextureRawMove(SpriteInfo, 0, pSprite->m_pName);
}

bool CGraphics_Threaded::IsImageSubFullyTransparent(const CImageInfo &FromImageInfo, int x, int y, int w, int h)
{
	if(FromImageInfo.m_Format == CImageInfo::FORMAT_R || FromImageInfo.m_Format == CImageInfo::FORMAT_RA || FromImageInfo.m_Format == CImageInfo::FORMAT_RGBA)
	{
		const uint8_t *pImgData = FromImageInfo.m_pData;
		const size_t PixelSize = FromImageInfo.PixelSize();
		for(int iy = 0; iy < h; ++iy)
		{
			for(int ix = 0; ix < w; ++ix)
			{
				const size_t RealOffset = (x + ix) * PixelSize + (y + iy) * PixelSize * FromImageInfo.m_Width;
				if(pImgData[RealOffset + (PixelSize - 1)] > 0)
					return false;
			}
		}

		return true;
	}
	return false;
}
// end of copy

// C++ copy: engine/client/graphics_threaded.cpp:593-644
bool CGraphics_Threaded::CheckImageDivisibility(const char *pContextName, CImageInfo &Image, int DivX, int DivY, bool AllowResize)
{
	dbg_assert(DivX != 0 && DivY != 0, "Passing 0 to this function is not allowed.");
	bool ImageIsValid = true;
	bool WidthBroken = Image.m_Width == 0 || (Image.m_Width % DivX) != 0;
	bool HeightBroken = Image.m_Height == 0 || (Image.m_Height % DivY) != 0;
	if(WidthBroken || HeightBroken)
	{
		SWarning NewWarning;
		char aContextNameQuoted[128];
		str_format(aContextNameQuoted, sizeof(aContextNameQuoted), "\"%s\"", pContextName);
		str_format(NewWarning.m_aWarningMsg, sizeof(NewWarning.m_aWarningMsg),
			Localize("The width of texture %s is not divisible by %d, or the height is not divisible by %d, which might cause visual bugs."), aContextNameQuoted, DivX, DivY);
		AddWarning(NewWarning);
		ImageIsValid = false;
	}

	if(AllowResize && !ImageIsValid && Image.m_Width > 0 && Image.m_Height > 0)
	{
		int NewWidth = DivX;
		int NewHeight = DivY;
		if(WidthBroken)
		{
			NewWidth = std::max(HighestBit(Image.m_Width), DivX);
			NewHeight = (NewWidth / DivX) * DivY;
		}
		else
		{
			NewHeight = std::max(HighestBit(Image.m_Height), DivY);
			NewWidth = (NewHeight / DivY) * DivX;
		}
		ResizeImage(Image, NewWidth, NewHeight);
		ImageIsValid = true;
	}

	return ImageIsValid;
}

bool CGraphics_Threaded::IsImageFormatRgba(const char *pContextName, const CImageInfo &Image)
{
	if(Image.m_Format != CImageInfo::FORMAT_RGBA)
	{
		SWarning NewWarning;
		char aContextNameQuoted[128];
		str_format(aContextNameQuoted, sizeof(aContextNameQuoted), "\"%s\"", pContextName);
		str_format(NewWarning.m_aWarningMsg, sizeof(NewWarning.m_aWarningMsg),
			Localize("The format of texture %s is not RGBA which will cause visual bugs."), aContextNameQuoted);
		AddWarning(NewWarning);
		return false;
	}
	return true;
}
// end of copy

class CGameClient
{
	CGraphics_Threaded *m_pGraphics;

public:
	explicit CGameClient(CGraphics_Threaded *pGraphics) :
		m_pGraphics(pGraphics) {}
	CGraphics_Threaded *Graphics() const { return m_pGraphics; }

// C++ copy: game/client/gameclient.h:926-935
	class CImageAsset
	{
	public:
		bool IsLoaded() const { return m_ImageInfo.m_pData != nullptr; }

		char m_aPath[IO_MAX_PATH_LENGTH];
		bool m_IsDefault;
		CImageInfo m_ImageInfo;
		std::optional<CImageInfo> m_FallbackImageInfo;
	};
// end of copy

	CImageAsset LoadAssetFromPath(const char *pPath, bool AsDir, int AssetId, const char *pDirectory) const;

	// LoadGameSkin & co.: the retry chain, then every sprite of the kind's set
	void LoadSkin(int AssetId, const char *pDirectory, const char *pPath, bool AsDir = false)
	{
		CImageAsset LoadedAsset = LoadAssetFromPath(pPath, AsDir, AssetId, pDirectory);
		CImageInfo &ImgInfo = LoadedAsset.m_ImageInfo;
		std::optional<CImageInfo> &FallbackImgInfo = LoadedAsset.m_FallbackImageInfo;
		if(!LoadedAsset.IsLoaded() && !LoadedAsset.m_IsDefault)
		{
			if(AsDir)
				LoadSkin(AssetId, pDirectory, "default");
			else
				LoadSkin(AssetId, pDirectory, pPath, true);
			return;
		}
		const CDataSpriteset *pSet = nullptr;
		for(int i = 0; i < g_pData->m_NumSpritesets; i++)
			if(g_pData->m_aSpritesets[i].m_pImage == &g_pData->m_aImages[AssetId])
				pSet = &g_pData->m_aSpritesets[i];
		const bool Valid = LoadedAsset.IsLoaded() && pSet && Graphics()->CheckImageDivisibility(LoadedAsset.m_aPath, ImgInfo, pSet->m_Gridx, pSet->m_Gridy, true) && Graphics()->IsImageFormatRgba(LoadedAsset.m_aPath, ImgInfo);
		fprintf(s_pOut, "asset %s %d %d\n", LoadedAsset.m_aPath, (int)LoadedAsset.IsLoaded(), (int)Valid);
		if(Valid)
			for(int i = 0; i < g_pData->m_NumSprites; i++)
				if(g_pData->m_aSprites[i].m_pSet == pSet)
					Graphics()->LoadSpriteTexture(ImgInfo, FallbackImgInfo, &g_pData->m_aSprites[i]);
		ImgInfo.Free();
		if(FallbackImgInfo.has_value())
			FallbackImgInfo.value().Free();
	}
};

// C++ copy: game/client/assets.cpp:12-38
CGameClient::CImageAsset CGameClient::LoadAssetFromPath(const char *pPath, bool AsDir, int AssetId, const char *pDirectory) const
{
	CImageAsset LoadedAsset;
	LoadedAsset.m_IsDefault = str_comp(pPath, "default") == 0;
	if(LoadedAsset.m_IsDefault)
	{
		str_copy(LoadedAsset.m_aPath, g_pData->m_aImages[AssetId].m_pFilename);
	}
	else if(AsDir)
	{
		str_format(LoadedAsset.m_aPath, sizeof(LoadedAsset.m_aPath), "assets/%s/%s/%s", pDirectory, pPath, g_pData->m_aImages[AssetId].m_pFilename);
	}
	else
	{
		str_format(LoadedAsset.m_aPath, sizeof(LoadedAsset.m_aPath), "assets/%s/%s.png", pDirectory, pPath);
	}

	Graphics()->LoadPng(LoadedAsset.m_ImageInfo, LoadedAsset.m_aPath, IStorage::TYPE_ALL);

	if(!LoadedAsset.m_IsDefault && LoadedAsset.IsLoaded())
	{
		CImageInfo ImgDefaultInfo;
		if(Graphics()->LoadPng(ImgDefaultInfo, g_pData->m_aImages[AssetId].m_pFilename, IStorage::TYPE_ALL))
			LoadedAsset.m_FallbackImageInfo = std::move(ImgDefaultInfo);
	}
	return LoadedAsset;
}
// end of copy

} // namespace rs_assets

int AssetsMode(const char *pOutFile, int NumRoots, const char *const *ppRoots)
{
	using namespace rs_assets;
	s_pOut = fopen(pOutFile, "w");
	if(!s_pOut)
		return -1;
	CGraphics_Threaded Graphics;
	for(int i = 0; i < NumRoots; i++)
		Graphics.m_vRoots.emplace_back(ppRoots[i]);
	CGameClient GameClient(&Graphics);
	const struct
	{
		int m_Id;
		const char *m_pDir;
	} aKinds[] = {{IMAGE_GAME, "game"}, {IMAGE_PARTICLES, "particles"}, {IMAGE_EMOTICONS, "emoticons"}, {IMAGE_HUD, "hud"}, {IMAGE_EXTRAS, "extras"}};
	for(const auto &Kind : aKinds)
	{
		std::set<std::string> Names = {"default", "no_such_pack"};
		for(const std::string &Root : Graphics.m_vRoots)
		{
			std::error_code Error;
			const std::filesystem::path Dir = Path(Root) / "assets" / Kind.m_pDir;
			for(const auto &Entry : std::filesystem::directory_iterator(Dir, Error))
			{
				if(Entry.is_directory())
					Names.insert(Utf8(Entry.path().filename()));
				else if(Entry.path().extension() == ".png")
					Names.insert(Utf8(Entry.path().stem()));
			}
		}
		for(const std::string &Name : Names)
		{
			fprintf(s_pOut, "pack %s %s\n", Kind.m_pDir, Name.c_str());
			GameClient.LoadSkin(Kind.m_Id, Kind.m_pDir, Name.c_str());
		}
	}
	fclose(s_pOut);
	return 0;
}
