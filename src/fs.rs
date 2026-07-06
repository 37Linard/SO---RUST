use crate::ata;
use alloc::string::String;
use alloc::vec::Vec;
use lazy_static::lazy_static;
use spin::Mutex;

const MAGIC: &[u8; 4] = b"MFS1";
const MAX_FILES: usize = 32;
const ENTRY_SIZE: usize = 64;
const ENTRIES_PER_SECTOR: usize = 512 / ENTRY_SIZE;
const DIR_SECTORS: usize = (MAX_FILES * ENTRY_SIZE) / 512;
const DATA_START_SECTOR: u32 = 1 + DIR_SECTORS as u32;
const MAX_NAME_LEN: usize = 46;

fn u32_at(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
}

fn write_u32(bytes: &mut [u8], off: usize, value: u32) {
    bytes[off..off + 4].copy_from_slice(&value.to_le_bytes());
}

struct DirEntry {
    used: bool,
    name: String,
    start_sector: u32,
    length_bytes: u32,
}

impl DirEntry {
    fn empty() -> Self {
        DirEntry {
            used: false,
            name: String::new(),
            start_sector: 0,
            length_bytes: 0,
        }
    }

    fn parse(bytes: &[u8]) -> Self {
        let used = bytes[0] != 0;
        let name_len = (bytes[1] as usize).min(MAX_NAME_LEN);
        let name = String::from_utf8_lossy(&bytes[2..2 + name_len]).into_owned();
        DirEntry {
            used,
            name,
            start_sector: u32_at(bytes, 48),
            length_bytes: u32_at(bytes, 52),
        }
    }

    fn serialize(&self, out: &mut [u8]) {
        out.fill(0);
        out[0] = self.used as u8;
        let name_bytes = self.name.as_bytes();
        let len = name_bytes.len().min(MAX_NAME_LEN);
        out[1] = len as u8;
        out[2..2 + len].copy_from_slice(&name_bytes[..len]);
        write_u32(out, 48, self.start_sector);
        write_u32(out, 52, self.length_bytes);
    }
}

/// Filesystem persistente (Fase 19) apoiado num disco ATA PIO real
/// (`src/ata.rs`, disco secundario `-hdb`) -- superblock + diretorio de
/// tamanho fixo (setores 1-4) + regiao de dados com bump allocator simples
/// (setor 5 em diante, sem reciclar espaco de delete/overwrite maior --
/// limitacao aceita conscientemente, disco pequeno o bastante pra nao
/// importar numa demo). API publica identica ao RAM-disk que existia
/// desde a Fase 9 -- shell/main/process nao precisaram mudar nada.
pub struct FileSystem {
    entries: Vec<DirEntry>,
    next_free_sector: u32,
}

impl FileSystem {
    fn new() -> Self {
        let mut sector0 = [0u8; 512];
        ata::read_sector(0, &mut sector0);

        if &sector0[0..4] == MAGIC {
            let next_free_sector = u32_at(&sector0, 8);
            let mut entries = Vec::with_capacity(MAX_FILES);
            let mut dir_buf = [0u8; 512];
            for dir_sector in 0..DIR_SECTORS {
                ata::read_sector(1 + dir_sector as u32, &mut dir_buf);
                for slot in 0..ENTRIES_PER_SECTOR {
                    let off = slot * ENTRY_SIZE;
                    entries.push(DirEntry::parse(&dir_buf[off..off + ENTRY_SIZE]));
                }
            }
            FileSystem {
                entries,
                next_free_sector,
            }
        } else {
            // Disco em branco (ou primeira vez) -- formata na hora, sem
            // passo manual do usuario alem de criar o arquivo disk.img.
            let fs = FileSystem {
                entries: (0..MAX_FILES).map(|_| DirEntry::empty()).collect(),
                next_free_sector: DATA_START_SECTOR,
            };
            fs.flush_directory();
            fs
        }
    }

    fn flush_directory(&self) {
        let mut sector0 = [0u8; 512];
        sector0[0..4].copy_from_slice(MAGIC);
        write_u32(
            &mut sector0,
            4,
            self.entries.iter().filter(|e| e.used).count() as u32,
        );
        write_u32(&mut sector0, 8, self.next_free_sector);
        ata::write_sector(0, &sector0);

        for dir_sector in 0..DIR_SECTORS {
            let mut buf = [0u8; 512];
            for slot in 0..ENTRIES_PER_SECTOR {
                let entry_idx = dir_sector * ENTRIES_PER_SECTOR + slot;
                let off = slot * ENTRY_SIZE;
                self.entries[entry_idx].serialize(&mut buf[off..off + ENTRY_SIZE]);
            }
            ata::write_sector(1 + dir_sector as u32, &buf);
        }
    }

    fn find_index(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.used && e.name == name)
    }

    fn read_data(&self, entry: &DirEntry) -> Vec<u8> {
        let sectors_needed = (entry.length_bytes as usize).div_ceil(512);
        let mut data = Vec::with_capacity(sectors_needed * 512);
        let mut buf = [0u8; 512];
        for i in 0..sectors_needed {
            ata::read_sector(entry.start_sector + i as u32, &mut buf);
            data.extend_from_slice(&buf);
        }
        data.truncate(entry.length_bytes as usize);
        data
    }

    /// Aloca espaco novo pros dados sempre no fim (bump allocator) e
    /// escreve -- nunca reaproveita setores de arquivos apagados ou
    /// sobrescritos com conteudo maior (limitacao aceita, ver doc da struct).
    fn write_data(&mut self, data: &[u8]) -> u32 {
        let start = self.next_free_sector;
        let sectors_needed = data.len().div_ceil(512).max(1);
        for i in 0..sectors_needed {
            let chunk_start = i * 512;
            let chunk_end = (chunk_start + 512).min(data.len());
            let mut buf = [0u8; 512];
            buf[..chunk_end - chunk_start].copy_from_slice(&data[chunk_start..chunk_end]);
            ata::write_sector(start + i as u32, &buf);
        }
        self.next_free_sector += sectors_needed as u32;
        start
    }

    pub fn create(&mut self, name: &str, data: &[u8]) {
        let start_sector = self.write_data(data);
        match self.find_index(name) {
            Some(idx) => {
                self.entries[idx].start_sector = start_sector;
                self.entries[idx].length_bytes = data.len() as u32;
            }
            None => match self.entries.iter().position(|e| !e.used) {
                Some(idx) => {
                    self.entries[idx] = DirEntry {
                        used: true,
                        name: String::from(name),
                        start_sector,
                        length_bytes: data.len() as u32,
                    };
                }
                None => return, // diretorio cheio (32 arquivos) -- MVP, ignora
            },
        }
        self.flush_directory();
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.find_index(name).map(|idx| self.read_data(&self.entries[idx]))
    }

    pub fn write(&mut self, name: &str, data: &[u8]) -> bool {
        match self.find_index(name) {
            Some(idx) => {
                let start_sector = self.write_data(data);
                self.entries[idx].start_sector = start_sector;
                self.entries[idx].length_bytes = data.len() as u32;
                self.flush_directory();
                true
            }
            None => false,
        }
    }

    pub fn delete(&mut self, name: &str) -> bool {
        match self.find_index(name) {
            Some(idx) => {
                self.entries[idx].used = false;
                self.flush_directory();
                true
            }
            None => false,
        }
    }

    pub fn list(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.used)
            .map(|e| e.name.clone())
            .collect()
    }
}

lazy_static! {
    pub static ref FS: Mutex<FileSystem> = Mutex::new(FileSystem::new());
}
