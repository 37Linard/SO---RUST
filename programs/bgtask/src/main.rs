#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

// Programa de teste da Fase 17 (spawn em background): loop mais longo que
// looper_a/looper_b (Fase 14) de proposito -- da tempo do usuario digitar
// outros comandos no shell enquanto ele roda, provando que o shell continua
// respondendo (nao trava esperando ele terminar, como run_elf/preempt fazem).
global_asm!(
    ".global _start",
    "_start:",
    "xor r12, r12",
    "1:",
    "lea rdi, [rip + msg]",
    "mov rsi, 9", // tamanho exato da string "bgtask..\n" (contado, nao calculado --
                  // ver gotcha da Fase 13: "mov reg, label_absoluto" vira
                  // dereference de memoria no assembler Intel do LLVM)
    "mov rax, 1", // SYS_WRITE
    "int 0x80",
    "mov rcx, 50000000",
    "2:",
    "dec rcx",
    "jnz 2b",
    "inc r12",
    "cmp r12, 15",
    "jl 1b",
    "mov rax, 60", // SYS_EXIT
    "mov rdi, 0",
    "int 0x80",
    "3:",
    "jmp 3b",
    "msg:",
    ".ascii \"bgtask..\\n\"",
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
