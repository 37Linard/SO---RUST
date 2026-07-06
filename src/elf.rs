use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageTableFlags, Size4KiB,
};
use x86_64::VirtAddr;

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EM_X86_64: u16 = 0x3E;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;

fn u16_at(bytes: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap())
}

struct ProgramHeader {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_filesz: u64,
    p_memsz: u64,
}

/// Carrega um ELF64 estatico (nao-PIE, sem relocacao) mapeando cada
/// segmento PT_LOAD no proprio p_vaddr do header. Retorna o entry point
/// (e_entry) pronto pra ser usado num salto pra ring3.
pub fn load_elf(
    mapper: &mut OffsetPageTable<'static>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    bytes: &[u8],
) -> Result<u64, &'static str> {
    if bytes.len() < 64 || bytes[0..4] != ELF_MAGIC {
        return Err("magic ELF invalido");
    }
    if bytes[4] != ELFCLASS64 {
        return Err("esperado ELF64 (ELFCLASS64)");
    }
    if bytes[5] != ELFDATA2LSB {
        return Err("esperado little-endian (ELFDATA2LSB)");
    }
    if u16_at(bytes, 18) != EM_X86_64 {
        return Err("esperado maquina EM_X86_64");
    }

    let e_entry = u64_at(bytes, 24);
    let e_phoff = u64_at(bytes, 32) as usize;
    let e_phentsize = u16_at(bytes, 54) as usize;
    let e_phnum = u16_at(bytes, 56) as usize;

    for i in 0..e_phnum {
        let ph_off = e_phoff + i * e_phentsize;
        let ph = ProgramHeader {
            p_type: u32_at(bytes, ph_off),
            p_flags: u32_at(bytes, ph_off + 4),
            p_offset: u64_at(bytes, ph_off + 8),
            p_vaddr: u64_at(bytes, ph_off + 16),
            p_filesz: u64_at(bytes, ph_off + 32),
            p_memsz: u64_at(bytes, ph_off + 40),
        };

        if ph.p_type != PT_LOAD {
            continue;
        }

        map_segment(mapper, frame_allocator, &ph, bytes)?;
    }

    Ok(e_entry)
}

fn map_segment(
    mapper: &mut OffsetPageTable<'static>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    ph: &ProgramHeader,
    bytes: &[u8],
) -> Result<(), &'static str> {
    // Mapeia sempre WRITABLE primeiro -- mesmo pra segmentos so-leitura/exec,
    // pois o proprio kernel (ring0) precisa escrever o conteudo do arquivo
    // na pagina antes do processo rodar. CR0.WP fica ligado, entao um W=0
    // bloqueia escrita ate pra ring0 (foi assim que a primeira tentativa
    // deu double fault: page fault ao copiar pra pagina R+X sem handler
    // de page fault instalado). Flags finais do ELF sao aplicadas so depois
    // da copia, via update_flags.
    let mut final_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if ph.p_flags & PF_W != 0 {
        final_flags |= PageTableFlags::WRITABLE;
    }
    if ph.p_flags & PF_X == 0 {
        final_flags |= PageTableFlags::NO_EXECUTE;
    }
    let writable_flags =
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

    let seg_start = VirtAddr::new(ph.p_vaddr);
    let seg_end = VirtAddr::new(ph.p_vaddr + ph.p_memsz - 1);
    let start_page = Page::<Size4KiB>::containing_address(seg_start);
    let end_page = Page::<Size4KiB>::containing_address(seg_end);

    for page in Page::range_inclusive(start_page, end_page) {
        if mapper.translate_page(page).is_err() {
            let frame = frame_allocator
                .allocate_frame()
                .ok_or("sem memoria fisica pra mapear segmento ELF")?;
            unsafe {
                mapper
                    .map_to(page, frame, writable_flags, frame_allocator)
                    .map_err(|_| "falha ao mapear pagina do segmento ELF")?
                    .flush();
            }
        } else {
            unsafe {
                mapper
                    .update_flags(page, writable_flags)
                    .map_err(|_| "falha ao atualizar flags da pagina do segmento ELF")?
                    .flush();
            }
        }
    }

    let file_bytes = &bytes[ph.p_offset as usize..(ph.p_offset + ph.p_filesz) as usize];
    unsafe {
        let dest = ph.p_vaddr as *mut u8;
        core::ptr::copy_nonoverlapping(file_bytes.as_ptr(), dest, file_bytes.len());
        if ph.p_memsz > ph.p_filesz {
            let bss_start = dest.add(file_bytes.len());
            core::ptr::write_bytes(bss_start, 0, (ph.p_memsz - ph.p_filesz) as usize);
        }
    }

    for page in Page::range_inclusive(start_page, end_page) {
        unsafe {
            mapper
                .update_flags(page, final_flags)
                .map_err(|_| "falha ao aplicar flags finais do segmento ELF")?
                .flush();
        }
    }

    Ok(())
}
