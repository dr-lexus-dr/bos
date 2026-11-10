//! ELF64 executable headers used by the bOS user-mode launcher.

use crate::mm::paging::AddressSpace;

pub fn is_elf(data: &[u8]) -> bool {
    data.starts_with(b"\x7fELF")
}

pub fn load_elf(_path: &str, _as: &mut AddressSpace) -> Result<u64, &'static str> {
    Err("scheduled exec is unavailable; launch from bSH")
}

pub(crate) struct Segment<'a> {
    pub address: u64,
    pub size: u64,
    pub bytes: &'a [u8],
    pub writable: bool,
    pub executable: bool,
}

pub(crate) struct Executable<'a> {
    pub entry: u64,
    data: &'a [u8],
    headers: &'a [u8],
}

impl<'a> Executable<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, &'static str> {
        if data.len() < 64
            || !is_elf(data)
            || data[4..7] != [2, 1, 1]
            || u16_at(data, 16) != 2
            || u16_at(data, 18) != 62
            || u32_at(data, 20) != 1
            || u16_at(data, 52) != 64
            || u16_at(data, 54) != 56
        {
            return Err("expected a static x86-64 ELF executable");
        }
        let count = usize::from(u16_at(data, 56));
        let offset =
            usize::try_from(u64_at(data, 32)).map_err(|_| "invalid program header offset")?;
        if count == 0 || count > 64 || offset < 64 {
            return Err("invalid program header table");
        }
        let end = offset
            .checked_add(count * 56)
            .ok_or("invalid program header size")?;
        let headers = data
            .get(offset..end)
            .ok_or("truncated program header table")?;
        Ok(Self {
            entry: u64_at(data, 24),
            data,
            headers,
        })
    }

    pub fn segments(&self) -> impl Iterator<Item = Result<Segment<'a>, &'static str>> + '_ {
        self.headers
            .chunks_exact(56)
            .filter_map(|header| match u32_at(header, 0) {
                1 => Some(self.segment(header)),
                2 | 3 | 7 => Some(Err("dynamic linking and TLS are unavailable")),
                _ => None,
            })
    }

    fn segment(&self, header: &[u8]) -> Result<Segment<'a>, &'static str> {
        let flags = u32_at(header, 4);
        let offset = usize::try_from(u64_at(header, 8)).map_err(|_| "invalid segment offset")?;
        let address = u64_at(header, 16);
        let file_size = u64_at(header, 32);
        let size = u64_at(header, 40);
        if file_size > size
            || flags & 4 == 0
            || flags & 3 == 3
            || offset as u64 % 4096 != address % 4096
        {
            return Err("invalid segment size, alignment, or permissions");
        }
        let end = offset
            .checked_add(file_size as usize)
            .ok_or("invalid segment size")?;
        let bytes = self.data.get(offset..end).ok_or("truncated segment")?;
        Ok(Segment {
            address,
            size,
            bytes,
            writable: flags & 2 != 0,
            executable: flags & 1 != 0,
        })
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
