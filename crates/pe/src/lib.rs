//! Minimal PE32 (i386) reader: sections, entry point, and the import table.
//!
//! Only what the recompiler and runtime need for `Bug3D.exe`. Malformed input
//! returns an error; nothing here executes or modifies the image.

use std::fmt;

#[derive(Debug)]
pub struct PeError(pub String);

impl fmt::Display for PeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PE parse error: {}", self.0)
    }
}
impl std::error::Error for PeError {}

type Result<T> = std::result::Result<T, PeError>;

#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    /// Absolute virtual address (image base + RVA).
    pub va: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}

impl Section {
    pub fn contains(&self, va: u32) -> bool {
        va >= self.va && va < self.va + self.virtual_size.max(self.raw_size)
    }
    pub fn is_executable(&self) -> bool {
        self.characteristics & 0x2000_0000 != 0
    }
}

#[derive(Debug, Clone)]
pub struct Import {
    pub dll: String,
    /// Function name, or `#<ordinal>` for ordinal imports.
    pub name: String,
    /// Absolute address of this entry's IAT slot.
    pub iat_va: u32,
}

#[derive(Debug, Clone)]
pub struct Pe {
    pub image_base: u32,
    pub entry_va: u32,
    pub size_of_image: u32,
    pub sections: Vec<Section>,
    pub imports: Vec<Import>,
}

fn u16_at(d: &[u8], o: usize) -> Result<u16> {
    d.get(o..o + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| PeError(format!("read u16 out of range at {o:#x}")))
}
fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| PeError(format!("read u32 out of range at {o:#x}")))
}

impl Pe {
    pub fn parse(d: &[u8]) -> Result<Pe> {
        if d.get(0..2) != Some(b"MZ") {
            return Err(PeError("missing MZ".into()));
        }
        let pe = u32_at(d, 0x3c)? as usize;
        if d.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(PeError("missing PE signature".into()));
        }
        if u16_at(d, pe + 4)? != 0x14c {
            return Err(PeError("not an i386 image".into()));
        }
        let nsec = u16_at(d, pe + 6)? as usize;
        let opt_size = u16_at(d, pe + 20)? as usize;
        let opt = pe + 24;
        if u16_at(d, opt)? != 0x10b {
            return Err(PeError("not PE32".into()));
        }
        let entry_rva = u32_at(d, opt + 16)?;
        let image_base = u32_at(d, opt + 28)?;
        let size_of_image = u32_at(d, opt + 56)?;
        let import_rva = u32_at(d, opt + 96 + 8)?;
        let mut sections = Vec::with_capacity(nsec);
        for i in 0..nsec {
            let o = opt + opt_size + i * 40;
            let raw_name = d.get(o..o + 8).ok_or_else(|| PeError("section table truncated".into()))?;
            let name = String::from_utf8_lossy(raw_name).trim_end_matches('\0').to_string();
            sections.push(Section {
                name,
                virtual_size: u32_at(d, o + 8)?,
                va: image_base.wrapping_add(u32_at(d, o + 12)?),
                raw_size: u32_at(d, o + 16)?,
                raw_offset: u32_at(d, o + 20)?,
                characteristics: u32_at(d, o + 36)?,
            });
        }
        let mut pe_img = Pe { image_base, entry_va: image_base + entry_rva, size_of_image, sections, imports: Vec::new() };
        if import_rva != 0 {
            pe_img.imports = pe_img.parse_imports(d, image_base + import_rva)?;
        }
        Ok(pe_img)
    }

    /// File offset for an absolute VA, if it is backed by raw data.
    pub fn file_offset(&self, va: u32) -> Option<usize> {
        self.sections.iter().find_map(|s| {
            (va >= s.va && va < s.va + s.raw_size).then(|| (s.raw_offset + (va - s.va)) as usize)
        })
    }

    pub fn section_for(&self, va: u32) -> Option<&Section> {
        self.sections.iter().find(|s| s.contains(va))
    }

    fn cstr_at(&self, d: &[u8], va: u32) -> Result<String> {
        let o = self.file_offset(va).ok_or_else(|| PeError(format!("string VA {va:#x} unmapped")))?;
        let end = d[o..].iter().position(|&b| b == 0).ok_or_else(|| PeError("unterminated string".into()))?;
        Ok(String::from_utf8_lossy(&d[o..o + end]).into_owned())
    }

    fn parse_imports(&self, d: &[u8], dir_va: u32) -> Result<Vec<Import>> {
        let mut out = Vec::new();
        let mut desc = dir_va;
        loop {
            let o = self.file_offset(desc).ok_or_else(|| PeError("import directory unmapped".into()))?;
            let ilt = u32_at(d, o)?;
            let name_rva = u32_at(d, o + 12)?;
            let iat = u32_at(d, o + 16)?;
            if ilt == 0 && name_rva == 0 && iat == 0 {
                break;
            }
            let dll = self.cstr_at(d, self.image_base + name_rva)?;
            let lookup = if ilt != 0 { ilt } else { iat };
            let mut i = 0u32;
            loop {
                let thunk_va = self.image_base + lookup + i * 4;
                let to = self.file_offset(thunk_va).ok_or_else(|| PeError("ILT unmapped".into()))?;
                let v = u32_at(d, to)?;
                if v == 0 {
                    break;
                }
                let name = if v & 0x8000_0000 != 0 {
                    format!("#{}", v & 0xffff)
                } else {
                    self.cstr_at(d, self.image_base + v + 2)?
                };
                out.push(Import { dll: dll.clone(), name, iat_va: self.image_base + iat + i * 4 });
                i += 1;
            }
            desc += 20;
        }
        Ok(out)
    }
}
