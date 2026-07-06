#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

// Loop com espera ocupada entre cada SYS_WRITE, pra dar tempo do timer
// interromper no meio -- prova visual de preempcao real quando rodado
// junto com looper_b (mensagens de A e B saem intercaladas no serial).
global_asm!(
    ".global _start",
    "_start:",
    "xor r12, r12",
    "1:",
    "lea rdi, [rip + msg]",
    "mov rsi, 2",
    "mov rax, 1", // SYS_WRITE
    "int 0x80",
    "mov rcx, 50000000",
    "2:",
    "dec rcx",
    "jnz 2b",
    "inc r12",
    "cmp r12, 5",
    "jl 1b",
    "mov rax, 60", // SYS_EXIT
    "mov rdi, 0",
    "int 0x80",
    "3:",
    "jmp 3b",
    "msg:",
    ".ascii \"A\\n\"",
);

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 60u64,
            in("rdi") 1u64,
            options(noreturn),
        );
    }
}
