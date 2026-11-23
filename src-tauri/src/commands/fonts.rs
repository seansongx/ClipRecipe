//! 列出系统字体（供 `pipe.text` 选择字体文件）。
//!
//! 只读取字体文件的表目录、`name` 表与 `OS/2` 表，不加载整个文件；`.ttc/.otc` 字体集合只看第一个字体
//! （drawtext 的 `fontfile` 也只使用第一个）。名称优先取简体中文，其次英文。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use serde::Serialize;

use super::blocking;
use crate::fsutil;

pub const FONT_EXTENSIONS: [&str; 4] = ["ttf", "ttc", "otf", "otc"];

/// 一个字体文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemFont {
    /// 显示名称（完整名，优先中文）
    pub name: String,
    /// 字体族（英文）
    pub family: String,
    pub path: String,
    /// 是否声明支持简体或繁体中文
    pub cjk: bool,
}

/// 字体文件里读出的名称信息。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontNames {
    pub full_name: String,
    pub family: String,
    pub cjk: bool,
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

fn read_at<R: Read + Seek>(r: &mut R, offset: u64, len: usize) -> Option<Vec<u8>> {
    r.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0; len];
    r.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// 从 `name` 表中按 nameID 取名称：简体中文 → 英文 → 任意 Windows 平台记录 → Mac Roman。
fn pick_name(table: &[u8], name_id: u16) -> Option<String> {
    let count = usize::from(be16(table, 2)?);
    let storage = usize::from(be16(table, 4)?);
    let mut best: Option<(u8, String)> = None;
    for i in 0..count {
        let rec = 6 + i * 12;
        let (platform, _encoding, language, id) = (
            be16(table, rec)?,
            be16(table, rec + 2)?,
            be16(table, rec + 4)?,
            be16(table, rec + 6)?,
        );
        if id != name_id {
            continue;
        }
        let (len, off) = (
            usize::from(be16(table, rec + 8)?),
            usize::from(be16(table, rec + 10)?),
        );
        let Some(bytes) = table.get(storage + off..storage + off + len) else {
            continue;
        };
        let (rank, text) = match platform {
            0 | 3 => {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|p| u16::from_be_bytes([p[0], p[1]]))
                    .collect();
                let Ok(text) = String::from_utf16(&units) else {
                    continue;
                };
                let rank = match language {
                    0x0804 => 0,
                    0x0409 => 1,
                    _ => 2,
                };
                (rank, text)
            }
            1 => (3, bytes.iter().map(|&b| char::from(b)).collect()),
            _ => continue,
        };
        let text = text.trim().to_owned();
        if text.is_empty() {
            continue;
        }
        if best.as_ref().map_or(true, |(r, _)| rank < *r) {
            best = Some((rank, text));
        }
    }
    best.map(|(_, t)| t)
}

/// 英文名称（族名用于搜索 / 显示副标题）。
fn pick_english(table: &[u8], name_id: u16) -> Option<String> {
    let count = usize::from(be16(table, 2)?);
    let storage = usize::from(be16(table, 4)?);
    (0..count).find_map(|i| {
        let rec = 6 + i * 12;
        if be16(table, rec)? != 3
            || be16(table, rec + 4)? != 0x0409
            || be16(table, rec + 6)? != name_id
        {
            return None;
        }
        let (len, off) = (
            usize::from(be16(table, rec + 8)?),
            usize::from(be16(table, rec + 10)?),
        );
        let bytes = table.get(storage + off..storage + off + len)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|p| u16::from_be_bytes([p[0], p[1]]))
            .collect();
        String::from_utf16(&units).ok()
    })
}

fn parse_face<R: Read + Seek>(r: &mut R, base: u64) -> Option<FontNames> {
    let header = read_at(r, base, 12)?;
    let version = be32(&header, 0)?;
    if !matches!(version, 0x0001_0000 | 0x4F54_544F | 0x7472_7565) {
        return None; // 不是 TrueType / OpenType
    }
    let tables = usize::from(be16(&header, 4)?);
    let dir = read_at(r, base + 12, tables * 16)?;
    let mut name = None;
    let mut os2 = None;
    for i in 0..tables {
        let rec = i * 16;
        let tag = dir.get(rec..rec + 4)?;
        let offset = u64::from(be32(&dir, rec + 8)?);
        let length = be32(&dir, rec + 12)? as usize;
        match tag {
            b"name" => name = read_at(r, offset, length.min(1 << 20)),
            b"OS/2" => os2 = read_at(r, offset, length.min(96)),
            _ => {}
        }
    }
    let name = name?;
    let full_name = pick_name(&name, 4).or_else(|| pick_name(&name, 1))?;
    let family = pick_english(&name, 1)
        .or_else(|| pick_name(&name, 1))
        .unwrap_or_else(|| full_name.clone());
    // OS/2 ulCodePageRange1（偏移 78，version ≥ 1）：第 18 位简体中文（936），第 20 位繁体中文（950）
    let cjk = os2
        .as_deref()
        .filter(|t| be16(t, 0).is_some_and(|v| v >= 1))
        .and_then(|t| be32(t, 78))
        .is_some_and(|pages| pages & (1 << 18 | 1 << 20) != 0);
    Some(FontNames {
        full_name,
        family,
        cjk,
    })
}

/// 读取一个字体文件（或字体集合的第一个字体）的名称信息。
pub fn read_font_names<R: Read + Seek>(r: &mut R) -> Option<FontNames> {
    let head = read_at(r, 0, 16)?;
    if head.get(0..4)? == b"ttcf" {
        let first = u64::from(be32(&head, 12)?);
        parse_face(r, first)
    } else {
        parse_face(r, 0)
    }
}

/// 系统字体目录。
pub fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if cfg!(windows) {
        let root = std::env::var_os("WINDIR")
            .or_else(|| std::env::var_os("SystemRoot"))
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        dirs.push(root.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        dirs.push("/System/Library/Fonts".into());
        dirs.push("/Library/Fonts".into());
        if let Some(home) = home {
            dirs.push(home.join("Library/Fonts"));
        }
    } else {
        dirs.push("/usr/share/fonts".into());
        dirs.push("/usr/local/share/fonts".into());
        if let Some(home) = home {
            dirs.push(home.join(".local/share/fonts"));
            dirs.push(home.join(".fonts"));
        }
    }
    dirs
}

/// 列出目录中的字体：支持中文的排在前面，其余按名称排序；同名只保留第一个。
pub fn list_fonts(dirs: &[PathBuf]) -> Vec<SystemFont> {
    let mut fonts: Vec<SystemFont> = dirs
        .iter()
        .flat_map(|dir| fsutil::list_files(dir, true, &FONT_EXTENSIONS).unwrap_or_default())
        .filter_map(|path| {
            let names = read_font_names(&mut File::open(&path).ok()?)?;
            Some(SystemFont {
                name: names.full_name,
                family: names.family,
                path: path.to_string_lossy().into_owned(),
                cjk: names.cjk,
            })
        })
        .collect();
    fonts.sort_by(|a, b| {
        b.cjk
            .cmp(&a.cjk)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    fonts.dedup_by(|a, b| a.name == b.name);
    fonts
}

/// 列出系统字体。
#[tauri::command]
pub async fn list_system_fonts() -> Result<Vec<SystemFont>, String> {
    blocking(|| list_fonts(&font_dirs())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::Path;

    /// 构造一个只有 `name` 与 `OS/2` 表的最小字体。
    fn tiny_font(names: &[(u16, u16, u16, &str)], code_pages: u32) -> Vec<u8> {
        let mut storage = Vec::new();
        let mut records = Vec::new();
        for &(platform, language, id, text) in names {
            let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
            records.extend(platform.to_be_bytes());
            records.extend(1u16.to_be_bytes());
            records.extend(language.to_be_bytes());
            records.extend(id.to_be_bytes());
            records.extend((bytes.len() as u16).to_be_bytes());
            records.extend((storage.len() as u16).to_be_bytes());
            storage.extend(bytes);
        }
        let mut name = Vec::new();
        name.extend(0u16.to_be_bytes());
        name.extend((names.len() as u16).to_be_bytes());
        name.extend((6 + records.len() as u16).to_be_bytes());
        name.extend(records);
        name.extend(storage);
        let mut os2 = vec![0u8; 86];
        os2[0..2].copy_from_slice(&1u16.to_be_bytes());
        os2[78..82].copy_from_slice(&code_pages.to_be_bytes());

        let dir_len = 12 + 2 * 16;
        let mut font = Vec::new();
        font.extend(0x0001_0000u32.to_be_bytes());
        font.extend(2u16.to_be_bytes());
        font.extend([0u8; 6]);
        let name_off = dir_len as u32;
        let os2_off = name_off + name.len() as u32;
        for (tag, off, len) in [
            (b"OS/2", os2_off, os2.len() as u32),
            (b"name", name_off, name.len() as u32),
        ] {
            font.extend(tag);
            font.extend(0u32.to_be_bytes());
            font.extend(off.to_be_bytes());
            font.extend(len.to_be_bytes());
        }
        font.extend(name);
        font.extend(os2);
        font
    }

    #[test]
    fn reads_localized_names_and_cjk_flag() {
        let font = tiny_font(
            &[
                (3, 0x0409, 1, "Sample Sans"),
                (3, 0x0409, 4, "Sample Sans Bold"),
                (3, 0x0804, 4, "示例黑体 粗体"),
            ],
            1 << 18,
        );
        let names = read_font_names(&mut Cursor::new(&font)).unwrap();
        assert_eq!(
            names,
            FontNames {
                full_name: "示例黑体 粗体".into(),
                family: "Sample Sans".into(),
                cjk: true,
            }
        );

        let latin = tiny_font(&[(3, 0x0409, 1, "Plain")], 1);
        let names = read_font_names(&mut Cursor::new(&latin)).unwrap();
        assert_eq!(names.full_name, "Plain");
        assert!(!names.cjk);

        // 字体集合：读取第一个字体
        let mut ttc = Vec::new();
        ttc.extend(b"ttcf");
        ttc.extend(0x0001_0000u32.to_be_bytes());
        ttc.extend(1u32.to_be_bytes());
        ttc.extend(16u32.to_be_bytes());
        let face_start = ttc.len() as u32;
        let mut face = tiny_font(&[(3, 0x0409, 4, "Collected")], 1 << 20);
        // 表偏移是相对整个文件的
        for i in 0..2 {
            let at = 12 + i * 16 + 8;
            let off = u32::from_be_bytes(face[at..at + 4].try_into().unwrap()) + face_start;
            face[at..at + 4].copy_from_slice(&off.to_be_bytes());
        }
        ttc.extend(face);
        let names = read_font_names(&mut Cursor::new(&ttc)).unwrap();
        assert_eq!(names.full_name, "Collected");
        assert!(names.cjk);

        assert_eq!(
            read_font_names(&mut Cursor::new(b"not a font at all")),
            None
        );
    }

    #[test]
    fn lists_installed_fonts() {
        let msyh = font_dirs()
            .first()
            .map(|d| d.join("msyh.ttc"))
            .filter(|p| cfg!(windows) && p.is_file());
        let Some(msyh) = msyh else {
            return; // 只在装有微软雅黑的 Windows 上检查真实字体
        };
        let fonts = list_fonts(&font_dirs());
        let found = fonts
            .iter()
            .find(|f| Path::new(&f.path) == msyh)
            .expect("应列出微软雅黑");
        assert!(found.cjk, "{found:?}");
        assert_eq!(found.family, "Microsoft YaHei");
        assert!(fonts[0].cjk, "支持中文的字体排在前面");
    }
}
