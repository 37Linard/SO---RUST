#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::VirtAddr;

mod allocator;
mod ata;
mod desktop;
mod elf;
mod font8x8;
mod framebuffer;
mod fs;
mod gdt;
mod gui_console;
mod interrupts;
mod memory;
mod mouse;
mod net;
mod pci;
mod process;
mod rtl8139;
mod serial;
mod shell;
mod syscall;
mod task;
mod vga_buffer;

use task::{executor::Executor, Task};

/// Offset de memoria fisica do bootloader, guardado aqui pro comando `gui`
/// do shell (`shell.rs`) conseguir chamar `draw_desktop()` -- so' existia
/// local em `kernel_main` antes (Fase 20).
static PHYS_MEM_OFFSET: AtomicU64 = AtomicU64::new(0);

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("meu-os booted!");
    println!("VGA writer online, println! funcionando.");

    gdt::init();
    interrupts::init_idt();
    interrupts::init_pics();
    mouse::init();
    println!("GDT + IDT + PIC carregados.");

    x86_64::instructions::interrupts::int3();
    println!("Voltou do breakpoint sem crashar.");

    x86_64::instructions::interrupts::enable();
    println!("Interrupcoes de hardware ativas.");

    PHYS_MEM_OFFSET.store(boot_info.physical_memory_offset, Ordering::Relaxed);
    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe { memory::BootInfoFrameAllocator::init(&boot_info.memory_map) };

    allocator::init_heap(&mut mapper, &mut frame_allocator).expect("falha ao iniciar heap");
    println!("Heap inicializado.");

    let heap_value = Box::new(41);
    println!("heap_value em {:p}", heap_value);

    let mut vec = Vec::new();
    for i in 0..500 {
        vec.push(i);
    }
    println!("vec em {:p}, len={}", vec.as_slice(), vec.len());

    test_fs();
    install_programs();
    *process::KERNEL_MEMORY.lock() = Some((mapper, frame_allocator));

    println!("Shell pronto. Digite 'help' pra ver comandos. Binarios instalados: 'hello'.");
    let mut executor = Executor::new();
    executor.spawn(Task::new(shell::run()));
    executor.run();
}

/// Binarios "instalados" no RAM-disk no boot -- sem driver de disco ainda,
/// entao o ELF compilado em `programs/hello` (build separado, ver README)
/// e embutido direto no binario do kernel.
fn install_programs() {
    let hello_elf: &[u8] = include_bytes!("../programs/hello/target/x86_64-meu_os/release/hello");
    fs::FS.lock().create("hello", hello_elf);

    let bgtask_elf: &[u8] =
        include_bytes!("../programs/bgtask/target/x86_64-meu_os/release/bgtask");
    fs::FS.lock().create("bgtask", bgtask_elf);
}

/// Ativa o modo grafico (VGA 320x200) e desenha o compositor multi-janela
/// (Fase 20) -- chamado pelo comando `gui` do shell. Mudanca visual so' de
/// ida: o buffer de texto (0xb8000) para de aparecer, mas o shell continua
/// rodando por baixo (aceita comandos, so' nao da pra ver o prompt).
pub fn draw_desktop() {
    let physical_memory_offset = PHYS_MEM_OFFSET.load(Ordering::Relaxed);
    desktop::setup_palette();
    desktop::init(framebuffer::Framebuffer::new(physical_memory_offset));
    println!("Tela de trabalho desenhada (VGA modo 13h, 320x200).");
}

fn test_fs() {
    let mut filesystem = fs::FS.lock();
    filesystem.create("hello.txt", b"Ola do meu-os!\n");
    filesystem.create("readme.md", b"# meu-os\nSO em Rust.\n");

    println!("arquivos: {:?}", filesystem.list());

    if let Some(data) = filesystem.read("hello.txt") {
        let content = core::str::from_utf8(&data).unwrap_or("<invalido>");
        println!("hello.txt: {}", content);
    }

    filesystem.write("hello.txt", b"conteudo atualizado\n");
    if let Some(data) = filesystem.read("hello.txt") {
        let content = core::str::from_utf8(&data).unwrap_or("<invalido>");
        println!("hello.txt (apos write): {}", content);
    }

    filesystem.delete("readme.md");
    println!("arquivos apos delete: {:?}", filesystem.list());
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    serial_println!("PANIC: {}", info);
    loop {}
}
