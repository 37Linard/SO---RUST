use crate::process::TrapFrame;
use crate::{print, println};
use core::arch::global_asm;

pub const SYSCALL_INTERRUPT_ID: u8 = 0x80;

pub const SYS_WRITE: u64 = 1;
pub const SYS_EXIT: u64 = 60;

#[no_mangle]
extern "C" fn syscall_dispatch(regs: &mut TrapFrame) {
    match regs.rax {
        SYS_WRITE => regs.rax = sys_write(regs.rdi, regs.rsi),
        // Se o scheduler da Fase 14 estiver ativo, isso pode trocar pro
        // proximo processo vivo em vez de sair de vez -- so diverge
        // (exit_to_kernel) quando nao ha mais ninguem pra rodar.
        SYS_EXIT => {
            let code = regs.rdi;
            crate::process::scheduler_exit_current(regs, code);
        }
        n => {
            println!("syscall desconhecida: {}", n);
            regs.rax = u64::MAX;
        }
    }
}

fn sys_write(ptr: u64, len: u64) -> u64 {
    let slice = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    match core::str::from_utf8(slice) {
        Ok(s) => {
            print!("{}", s);
            crate::serial_print!("{}", s);
            len
        }
        Err(_) => u64::MAX,
    }
}

extern "C" {
    pub fn syscall_handler_asm();
}

global_asm!(
    ".global syscall_handler_asm",
    "syscall_handler_asm:",
    // Mesma ordem/layout de `process::TrapFrame` e de `timer_entry_asm` --
    // os dois pontos de entrada alimentam o mesmo scheduler, entao os GPRs
    // completos (nao so os caller-saved) precisam estar la, mesmo que
    // SYS_WRITE em si nao use rbx/rbp/r12-r15.
    "push r15",
    "push r14",
    "push r13",
    "push r12",
    "push r11",
    "push r10",
    "push r9",
    "push r8",
    "push rbp",
    "push rbx",
    "push rdx",
    "push rcx",
    "push rsi",
    "push rdi",
    "push rax",
    "mov rdi, rsp",
    "call syscall_dispatch",
    "pop rax",
    "pop rdi",
    "pop rsi",
    "pop rcx",
    "pop rdx",
    "pop rbx",
    "pop rbp",
    "pop r8",
    "pop r9",
    "pop r10",
    "pop r11",
    "pop r12",
    "pop r13",
    "pop r14",
    "pop r15",
    "iretq",
);
