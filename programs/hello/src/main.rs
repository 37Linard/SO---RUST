#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

// Sem crt0/libc: o linker usa "_start" como entry point por padrao.
// Programa minimo: escreve uma mensagem via SYS_WRITE e sai via SYS_EXIT
// (mesmos numeros de syscall que o kernel ja implementa em src/syscall.rs).
global_asm!(
    ".global _start",
    "_start:",
    "lea rdi, [rip + msg]",
    "mov rsi, 49", // tamanho exato da string abaixo (contado, nao calculado --
                   // "mov reg, label_absoluto" no assembler Intel do LLVM
                   // vira dereference de memoria, nao imediato; foi assim
                   // que a 1a tentativa deu #PF lendo CR2=0x31=49)
    "mov rax, 1", // SYS_WRITE
    "int 0x80",
    "mov rax, 60", // SYS_EXIT
    "mov rdi, 0",
    "int 0x80",
    "2:",
    "jmp 2b",
    "msg:",
    ".ascii \"Ola do userland real (ELF carregado de verdade)!\\n\"",
);

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 60u64, // SYS_EXIT
            in("rdi") 1u64,
            options(noreturn),
        );
    }
}
