use crate::gdt;
use core::arch::global_asm;
use x86_64::structures::paging::{Mapper, OffsetPageTable, Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;

extern "C" {
    fn user_mode_program();
    fn user_mode_program_end();
}

// Programa de "usuario" minimo: escreve uma mensagem via syscall SYS_WRITE
// (int 0x80) e sai via SYS_EXIT. Vive dentro do proprio binario do kernel
// (sem loader de ELF ainda) -- a pagina que contem esse codigo e marcada
// como USER_ACCESSIBLE antes do salto pra ring3.
global_asm!(
    ".global user_mode_program",
    ".global user_mode_program_end",
    "user_mode_program:",
    "lea rdi, [rip + usermode_msg]",
    "mov rsi, 12",
    "mov rax, 1", // SYS_WRITE
    "int 0x80",
    "mov rax, 60", // SYS_EXIT
    "mov rdi, 0",
    "int 0x80",
    "2:",
    "jmp 2b",
    "usermode_msg:",
    ".ascii \"USERLAND OK\\n\"",
    "user_mode_program_end:",
);

const USER_STACK_SIZE: usize = 4096 * 2;
static mut USER_STACK: [u8; USER_STACK_SIZE] = [0; USER_STACK_SIZE];

fn mark_user_accessible(mapper: &mut OffsetPageTable<'static>, start: VirtAddr, end: VirtAddr) {
    let flags =
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
    let start_page = Page::<Size4KiB>::containing_address(start);
    let end_page = Page::<Size4KiB>::containing_address(end);

    for page in Page::range_inclusive(start_page, end_page) {
        unsafe {
            mapper
                .set_flags_p4_entry(page, flags)
                .expect("falha ao atualizar flags p4")
                .ignore();
            mapper
                .set_flags_p3_entry(page, flags)
                .expect("falha ao atualizar flags p3")
                .ignore();
            mapper
                .set_flags_p2_entry(page, flags)
                .expect("falha ao atualizar flags p2")
                .ignore();
            mapper
                .update_flags(page, flags)
                .expect("falha ao atualizar flags da pagina")
                .flush();
        }
    }
}

pub fn jump_to_usermode(mapper: &mut OffsetPageTable<'static>) -> ! {
    let code_start = VirtAddr::new(user_mode_program as *const () as u64);
    let code_end = VirtAddr::new(user_mode_program_end as *const () as u64 - 1);
    mark_user_accessible(mapper, code_start, code_end);

    let stack_start = VirtAddr::from_ptr(core::ptr::addr_of!(USER_STACK));
    let stack_end = stack_start + (USER_STACK_SIZE as u64 - 1);
    mark_user_accessible(mapper, stack_start, stack_end);

    let user_stack_top = (stack_start + USER_STACK_SIZE as u64).as_u64();

    let cs: u64 = gdt::user_code_selector().0 as u64;
    let ss: u64 = gdt::user_data_selector().0 as u64;
    let entry = code_start.as_u64();
    let rflags: u64 = 0x200; // IF=1, resto zerado

    unsafe {
        core::arch::asm!(
            "push {ss}",
            "push {stack_top}",
            "push {rflags}",
            "push {cs}",
            "push {entry}",
            "iretq",
            ss = in(reg) ss,
            stack_top = in(reg) user_stack_top,
            rflags = in(reg) rflags,
            cs = in(reg) cs,
            entry = in(reg) entry,
            options(noreturn),
        );
    }
}
