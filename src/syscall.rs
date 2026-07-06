use crate::{print, println};
use core::arch::global_asm;

pub const SYSCALL_INTERRUPT_ID: u8 = 0x80;

pub const SYS_WRITE: u64 = 1;
pub const SYS_EXIT: u64 = 60;

/// Layout precisa bater com a ordem de push no stub asm (`syscall_handler_asm`):
/// ultimo push (rax) fica no offset 0, entao offset cresce na ordem inversa dos pushes.
#[repr(C)]
struct Registers {
    rax: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
}

#[no_mangle]
extern "C" fn syscall_dispatch(regs: &mut Registers) {
    regs.rax = match regs.rax {
        SYS_WRITE => sys_write(regs.rdi, regs.rsi),
        SYS_EXIT => sys_exit(regs.rdi),
        n => {
            println!("syscall desconhecida: {}", n);
            u64::MAX
        }
    };
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

fn sys_exit(code: u64) -> u64 {
    println!("\nprocesso saiu com codigo {}", code);
    code
}

extern "C" {
    pub fn syscall_handler_asm();
}

global_asm!(
    ".global syscall_handler_asm",
    "syscall_handler_asm:",
    "push r11",
    "push r10",
    "push r9",
    "push r8",
    "push rcx",
    "push rdx",
    "push rsi",
    "push rdi",
    "push rax",
    "mov rdi, rsp",
    "call syscall_dispatch",
    "pop rax",
    "pop rdi",
    "pop rsi",
    "pop rdx",
    "pop rcx",
    "pop r8",
    "pop r9",
    "pop r10",
    "pop r11",
    "iretq",
);
