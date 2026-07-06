use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use lazy_static::lazy_static;
use spin::Mutex;

/// Filesystem em memoria (RAM-disk). Sem driver de disco ainda, entao
/// nada aqui sobrevive a um reboot -- prova o conceito de VFS
/// (create/read/write/delete/list) antes de existir I/O real de disco.
pub struct FileSystem {
    files: BTreeMap<String, Vec<u8>>,
}

impl FileSystem {
    fn new() -> Self {
        FileSystem {
            files: BTreeMap::new(),
        }
    }

    pub fn create(&mut self, name: &str, data: &[u8]) {
        self.files.insert(String::from(name), Vec::from(data));
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.files.get(name).cloned()
    }

    pub fn write(&mut self, name: &str, data: &[u8]) -> bool {
        match self.files.get_mut(name) {
            Some(existing) => {
                existing.clear();
                existing.extend_from_slice(data);
                true
            }
            None => false,
        }
    }

    pub fn delete(&mut self, name: &str) -> bool {
        self.files.remove(name).is_some()
    }

    pub fn list(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }
}

lazy_static! {
    pub static ref FS: Mutex<FileSystem> = Mutex::new(FileSystem::new());
}
